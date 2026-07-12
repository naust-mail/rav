use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::config::AppConfig;
use crate::db;
use crate::imap::client::{ImapClient, ImapCredentials};
use crate::mail_transport::MailTransport;
use crate::realtime::events::EventBus;
use crate::realtime::idle;
use crate::realtime::sync::run_sync;
use crate::search::engine::SearchEngine;
use crate::smtp::client::SmtpClient;

/// How long a worker waits for a wake-up before shutting itself down.
/// Generous relative to the 30s keepalive cadence so a connected user's
/// worker never times out under normal use; only truly abandoned workers
/// (no WS connection, no IDLE, nobody polling) get reaped.
const WORKER_IDLE_TIMEOUT_SECS: u64 = 600;

/// Ceiling on a single sync cycle so one hung IMAP call can't wedge a
/// user's worker forever.
const SYNC_CYCLE_TIMEOUT_SECS: u64 = 120;

/// Owns exactly one background sync worker per user.
///
/// Anything that wants a user's cache refreshed — IDLE waking up, a stale
/// folder view, a periodic keepalive — calls `ensure_worker` and rings the
/// returned bell. The worker is spawned on first use, coalesces repeated
/// rings into a single sync cycle, and self-terminates after sitting idle;
/// a later `ensure_worker` call respawns it on demand. Nothing here is
/// tied to a specific caller's lifecycle (unlike `IdleManager`, which is
/// torn down explicitly on WebSocket disconnect) — that's deliberate, so
/// future callers (e.g. an outbox worker) can reuse a running worker or
/// spawn one without knowing who else depends on it.
/// App-wide services a sync cycle needs. All are long-lived singletons
/// (same `Arc` for the process lifetime) — captured once here instead of
/// being re-threaded through every caller of `ensure_worker`.
pub struct SyncWorkerManager {
    config: Arc<AppConfig>,
    imap_client: Arc<dyn ImapClient>,
    event_bus: Arc<EventBus>,
    search_engine: Arc<SearchEngine>,
    smtp_client: Arc<dyn SmtpClient>,
    transport: Arc<MailTransport>,
    db_pool_manager: Arc<db::pool::DbPoolManager>,
    bells: DashMap<String, Arc<Notify>>,
    tasks: DashMap<String, JoinHandle<()>>,
}

impl SyncWorkerManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: Arc<AppConfig>,
        imap_client: Arc<dyn ImapClient>,
        event_bus: Arc<EventBus>,
        search_engine: Arc<SearchEngine>,
        smtp_client: Arc<dyn SmtpClient>,
        transport: Arc<MailTransport>,
        db_pool_manager: Arc<db::pool::DbPoolManager>,
    ) -> Self {
        Self {
            config,
            imap_client,
            event_bus,
            search_engine,
            smtp_client,
            transport,
            db_pool_manager,
            bells: DashMap::new(),
            tasks: DashMap::new(),
        }
    }

    /// Ensure a sync worker is running for this user (spawning one if
    /// there's none, or if the previous one already exited — from its own
    /// idle timeout or a panic), and return its wake-up bell. Ringing the
    /// bell (`Notify::notify_one`) schedules a sync cycle.
    pub fn ensure_worker(&self, user_hash: String, creds: ImapCredentials) -> Arc<Notify> {
        if let Some(task) = self.tasks.get(&user_hash)
            && !task.is_finished()
            && let Some(bell) = self.bells.get(&user_hash)
        {
            return bell.clone();
        }

        let bell = Arc::new(Notify::new());
        self.bells.insert(user_hash.clone(), bell.clone());

        let worker_user_hash = user_hash.clone();
        let worker_bell = bell.clone();
        let config = self.config.clone();
        let imap_client = self.imap_client.clone();
        let event_bus = self.event_bus.clone();
        let search_engine = self.search_engine.clone();
        let smtp_client = self.smtp_client.clone();
        let transport = self.transport.clone();
        let db_pool_manager = self.db_pool_manager.clone();
        let handle = tokio::spawn(async move {
            worker_loop(
                worker_user_hash, creds, config, imap_client, event_bus,
                search_engine, smtp_client, transport, db_pool_manager, worker_bell,
            ).await;
        });
        self.tasks.insert(user_hash, handle);

        bell
    }
}

#[allow(clippy::too_many_arguments)]
async fn worker_loop(
    user_hash: String,
    creds: ImapCredentials,
    config: Arc<AppConfig>,
    imap_client: Arc<dyn ImapClient>,
    event_bus: Arc<EventBus>,
    search_engine: Arc<SearchEngine>,
    smtp_client: Arc<dyn SmtpClient>,
    transport: Arc<MailTransport>,
    db_pool_manager: Arc<db::pool::DbPoolManager>,
    bell: Arc<Notify>,
) {
    loop {
        tokio::select! {
            _ = bell.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(WORKER_IDLE_TIMEOUT_SECS)) => {
                tracing::debug!(user_hash = %user_hash, "Sync worker idle timeout, shutting down");
                return;
            }
        }

        let max_uid_before = inbox_max_uid(&db_pool_manager, &user_hash).await;

        match tokio::time::timeout(
            Duration::from_secs(SYNC_CYCLE_TIMEOUT_SECS),
            run_sync(&user_hash, &creds, imap_client.as_ref(), &event_bus, &search_engine, &db_pool_manager),
        ).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::warn!(user_hash = %user_hash, error = %e, "Sync worker cycle failed, will retry on next wake-up");
            }
            Err(_) => {
                tracing::warn!(user_hash = %user_hash, "Sync worker cycle timed out, will retry on next wake-up");
            }
        }

        let max_uid_after = inbox_max_uid(&db_pool_manager, &user_hash).await;
        if max_uid_after > max_uid_before {
            let bg_user_hash = user_hash.clone();
            let bg_config = config.clone();
            let bg_creds = creds.clone();
            let bg_smtp = smtp_client.clone();
            let bg_imap = imap_client.clone();
            let bg_transport = transport.clone();
            let bg_event_bus = event_bus.clone();
            let bg_db_pool_manager = db_pool_manager.clone();
            tokio::spawn(async move {
                idle::process_new_messages(
                    &bg_user_hash, &bg_config, &bg_creds, &bg_smtp, &bg_imap, &bg_transport,
                    &bg_event_bus, &bg_db_pool_manager, max_uid_before,
                ).await;
            });
        }
    }
}

async fn inbox_max_uid(db_pool_manager: &db::pool::DbPoolManager, user_hash: &str) -> u32 {
    db::pool::with_user_db(db_pool_manager, user_hash, |conn| {
        Ok(db::messages::max_uid(conn, "INBOX").unwrap_or(0))
    })
    .await
    .unwrap_or(0)
}
