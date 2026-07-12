use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use dashmap::DashMap;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::config::AppConfig;
use crate::db;
use crate::db::outbox::OutboxEntry;
use crate::imap::client::ImapClient;
use crate::mail_transport::MailTransport;
use crate::realtime::events::{EventBus, MailEvent};
use crate::routes::send::{self, SendCredentials, SendJob};
use crate::smtp::client::SmtpClient;
use crate::smtp::types::PgpSendParams;

/// How long a worker waits for a wake-up (enqueue/undo/retry) before
/// shutting itself down, when there's nothing scheduled to wait for.
/// A later `ensure_worker` call (next login, next enqueue) respawns it.
const WORKER_IDLE_TIMEOUT_SECS: u64 = 600;

/// Failed sends are retried with exponential backoff up to this many times
/// before the entry is marked permanently `failed` and left for the user
/// to retry manually from the Outbox view.
const MAX_ATTEMPTS: i64 = 5;

fn now_iso() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Exponential backoff: 30s, 60s, 120s, 240s, 480s, capped at 30 minutes.
fn backoff_duration(attempt_count: i64) -> Duration {
    let secs = 30_u64.saturating_mul(1_u64 << attempt_count.clamp(0, 6) as u32);
    Duration::from_secs(secs.min(1800))
}

/// Parses timestamps in the app's own `now_iso()`/`send_after` format
/// (`"%Y-%m-%dT%H:%M:%SZ"`, always UTC). Must use `NaiveDateTime`, not
/// `DateTime::parse_from_str` — the trailing `Z` here is a literal
/// character, not a `%z`/`%Z` offset specifier, so `DateTime::parse_from_str`
/// has no offset to construct a `FixedOffset` from and always fails.
fn parse_iso(ts: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%SZ")
        .ok()
        .map(|naive| naive.and_utc())
}

/// Owns exactly one background outbox-draining worker per user, mirroring
/// `SyncWorkerManager`'s shape. Credentials are captured once when a
/// worker is first spawned and held only in that task's memory for its
/// lifetime — never persisted. If the process restarts, scheduled entries
/// stay in the DB untouched and simply wait for the next `ensure_worker`
/// call (the user's next login, or their next enqueue) to resume.
/// A running worker's bell (to poke it) and its `JoinHandle` (to check
/// whether it's still alive). Held together in one map entry so
/// check-then-spawn is a single atomic operation per user — splitting this
/// into two maps let two concurrent `ensure_worker` calls both see "no
/// worker" and both spawn, with the second silently orphaning the first.
struct Worker {
    bell: Arc<Notify>,
    handle: JoinHandle<()>,
}

pub struct OutboxWorkerManager {
    config: Arc<AppConfig>,
    imap_client: Arc<dyn ImapClient>,
    smtp_client: Arc<dyn SmtpClient>,
    transport: Arc<MailTransport>,
    event_bus: Arc<EventBus>,
    db_pool_manager: Arc<db::pool::DbPoolManager>,
    workers: DashMap<String, Worker>,
}

impl OutboxWorkerManager {
    pub fn new(
        config: Arc<AppConfig>,
        imap_client: Arc<dyn ImapClient>,
        smtp_client: Arc<dyn SmtpClient>,
        transport: Arc<MailTransport>,
        event_bus: Arc<EventBus>,
        db_pool_manager: Arc<db::pool::DbPoolManager>,
    ) -> Self {
        Self {
            config,
            imap_client,
            smtp_client,
            transport,
            event_bus,
            db_pool_manager,
            workers: DashMap::new(),
        }
    }

    /// Ensure an outbox worker is running for this user, spawning one with
    /// the given credentials if none is running. If a worker is already
    /// running, the credentials passed here are ignored (the running
    /// worker keeps whatever it was spawned with) — call this again after
    /// login to guarantee a worker with fresh credentials exists.
    pub fn ensure_worker(&self, user_hash: String, email: String, password: String) -> Arc<Notify> {
        use dashmap::mapref::entry::Entry;

        // Holds this shard's lock for the whole check-then-spawn, so two
        // concurrent callers for the same user can't both decide to spawn.
        match self.workers.entry(user_hash.clone()) {
            Entry::Occupied(occupied) if !occupied.get().handle.is_finished() => {
                tracing::debug!(user_hash = %user_hash, "Outbox worker: reusing existing worker");
                occupied.get().bell.clone()
            }
            entry => {
                tracing::debug!(user_hash = %user_hash, "Outbox worker: spawning new worker");
                let bell = Arc::new(Notify::new());

                let worker_user_hash = user_hash.clone();
                let worker_bell = bell.clone();
                let creds = SendCredentials { user_hash: worker_user_hash, email, password };
                let deps = OutboxDeps {
                    config: self.config.clone(),
                    imap_client: self.imap_client.clone(),
                    smtp_client: self.smtp_client.clone(),
                    transport: self.transport.clone(),
                    event_bus: self.event_bus.clone(),
                    db_pool_manager: self.db_pool_manager.clone(),
                };
                let handle = tokio::spawn(async move {
                    worker_loop(creds, deps, worker_bell).await;
                });

                match entry {
                    Entry::Occupied(mut occupied) => { occupied.insert(Worker { bell: bell.clone(), handle }); }
                    Entry::Vacant(vacant) => { vacant.insert(Worker { bell: bell.clone(), handle }); }
                }

                bell
            }
        }
    }
}

/// Long-lived services an outbox worker's cycle needs, bundled so
/// `worker_loop` and `process_entry` take one param instead of threading
/// each `Arc` through separately.
#[derive(Clone)]
struct OutboxDeps {
    config: Arc<AppConfig>,
    imap_client: Arc<dyn ImapClient>,
    smtp_client: Arc<dyn SmtpClient>,
    transport: Arc<MailTransport>,
    event_bus: Arc<EventBus>,
    db_pool_manager: Arc<db::pool::DbPoolManager>,
}

async fn worker_loop(creds: SendCredentials, deps: OutboxDeps, bell: Arc<Notify>) {
    let user_hash = creds.user_hash.clone();
    let db_pool_manager = deps.db_pool_manager.clone();
    loop {
        let due_result = db::pool::with_user_db(&db_pool_manager, &user_hash, |conn| {
            db::outbox::list_due(conn, &now_iso())
        })
        .await;

        let due = match due_result {
            Ok(due) => due,
            Err(e) => {
                tracing::warn!(user_hash = %user_hash, error = %e, "Outbox worker: failed to open DB, retrying after idle timeout");
                tokio::time::sleep(Duration::from_secs(WORKER_IDLE_TIMEOUT_SECS)).await;
                continue;
            }
        };

        tracing::debug!(user_hash = %user_hash, due_count = due.len(), "Outbox worker: tick");

        for entry in due {
            process_entry(&entry, &creds, &deps).await;
        }

        let next_deadline_result = db::pool::with_user_db(&db_pool_manager, &user_hash, |conn| {
            db::outbox::next_send_after(conn)
        })
        .await;

        let next_deadline = match next_deadline_result {
            Ok(deadline) => deadline,
            Err(e) => {
                tracing::warn!(user_hash = %user_hash, error = %e, "Outbox worker: failed to query next send_after, retrying after idle timeout");
                tokio::time::sleep(Duration::from_secs(WORKER_IDLE_TIMEOUT_SECS)).await;
                continue;
            }
        };

        tracing::debug!(user_hash = %user_hash, next_deadline = ?next_deadline, "Outbox worker: next_deadline computed");

        match next_deadline.as_deref().and_then(parse_iso) {
            Some(deadline) => {
                let wait = (deadline - Utc::now()).to_std().unwrap_or(Duration::ZERO);
                tracing::debug!(user_hash = %user_hash, deadline = %deadline, wait_secs = wait.as_secs(), "Outbox worker: sleeping until next due entry");
                tokio::select! {
                    _ = bell.notified() => {}
                    _ = tokio::time::sleep(wait) => {}
                }
            }
            None => {
                tokio::select! {
                    _ = bell.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(WORKER_IDLE_TIMEOUT_SECS)) => {
                        tracing::debug!(user_hash = %user_hash, "Outbox worker idle timeout, shutting down");
                        return;
                    }
                }
            }
        }
    }
}

async fn process_entry(entry: &OutboxEntry, creds: &SendCredentials, deps: &OutboxDeps) {
    let user_hash = creds.user_hash.as_str();
    let OutboxDeps { config, imap_client, smtp_client, transport, event_bus, db_pool_manager } = deps;
    tracing::debug!(id = %entry.id, user_hash = %user_hash, attempt = entry.attempt_count + 1, "Outbox worker: attempting send");

    let mark_sending_result = db::pool::with_user_db(db_pool_manager, user_hash, {
        let id = entry.id.clone();
        move |conn| db::outbox::mark_sending(conn, &id)
    })
    .await;
    if mark_sending_result.is_err() {
        return;
    }

    event_bus
        .publish(user_hash, MailEvent::OutboxStateChanged {
            id: entry.id.clone(),
            state: "sending".to_string(),
            fail_reason: None,
        })
        .await;

    let pgp: Option<PgpSendParams> = match &entry.pgp_json {
        Some(json) => serde_json::from_str(json).ok(),
        None => None,
    };

    let job = SendJob {
        to: entry.to_addrs.clone(),
        cc: entry.cc_addrs.clone(),
        bcc: entry.bcc_addrs.clone(),
        subject: entry.subject.clone(),
        text_body: entry.text_body.clone(),
        html_body: entry.html_body.clone(),
        in_reply_to: entry.in_reply_to.clone(),
        references: entry.references_hdr.clone(),
        draft_id: entry.draft_id.clone(),
        from_identity_id: entry.from_identity_id,
        pgp,
    };

    match send::perform_send(config, transport, smtp_client, imap_client, db_pool_manager, creds, job).await {
        Ok(_message_id) => {
            let _ = db::pool::with_user_db(db_pool_manager, user_hash, {
                let id = entry.id.clone();
                move |conn| db::outbox::delete(conn, &id)
            })
            .await;
            event_bus
                .publish(user_hash, MailEvent::OutboxStateChanged {
                    id: entry.id.clone(),
                    state: "sent".to_string(),
                    fail_reason: None,
                })
                .await;
        }
        Err(e) if send::is_retryable(&e) && entry.attempt_count + 1 < MAX_ATTEMPTS => {
            let reason = e.to_string();
            let next = Utc::now() + chrono::Duration::from_std(backoff_duration(entry.attempt_count)).unwrap_or_default();
            let _ = db::pool::with_user_db(db_pool_manager, user_hash, {
                let id = entry.id.clone();
                let next_str = next.format("%Y-%m-%dT%H:%M:%SZ").to_string();
                let reason = reason.clone();
                move |conn| db::outbox::mark_retry(conn, &id, &next_str, &reason)
            })
            .await;
            tracing::warn!(id = %entry.id, error = %reason, attempt = entry.attempt_count + 1, "Outbox send failed, will retry");
        }
        Err(e) => {
            let reason = e.to_string();
            let _ = db::pool::with_user_db(db_pool_manager, user_hash, {
                let id = entry.id.clone();
                let reason = reason.clone();
                move |conn| db::outbox::mark_failed(conn, &id, &reason)
            })
            .await;
            tracing::warn!(id = %entry.id, error = %reason, "Outbox send permanently failed");
            event_bus
                .publish(user_hash, MailEvent::OutboxStateChanged {
                    id: entry.id.clone(),
                    state: "failed".to_string(),
                    fail_reason: Some(reason),
                })
                .await;
        }
    }
}
