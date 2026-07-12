use std::collections::HashSet;
use std::sync::Arc;

use dashmap::DashMap;
use tokio::task::JoinHandle;

use crate::config::AppConfig;
use crate::imap::client::{ImapClient, ImapCredentials};
use crate::mail_transport::MailTransport;
use crate::realtime::events::{EventBus, MailEvent};
use crate::smtp::client::{SmtpClient, SmtpCredentials, SendableMessage};
use crate::{db, imap};

/// Manages long-lived IMAP IDLE connections, one per (user, folder) pair.
///
/// When a WebSocket client connects, the IdleManager starts IDLE for INBOX.
/// When the client disconnects, all IDLE tasks for that user are stopped.
pub struct IdleManager {
    /// Active IDLE tasks keyed by `(user_hash, folder_name)`.
    tasks: DashMap<(String, String), JoinHandle<()>>,
}

impl IdleManager {
    pub fn new() -> Self {
        Self {
            tasks: DashMap::new(),
        }
    }

    /// Start an IDLE task for a specific user + folder.
    /// If one is already running, this is a no-op.
    pub async fn start_idle(
        &self,
        user_hash: String,
        folder: String,
        creds: ImapCredentials,
        transport: Arc<MailTransport>,
        sync_worker_manager: Arc<crate::realtime::worker::SyncWorkerManager>,
    ) {
        let key = (user_hash.clone(), folder.clone());
        if self.tasks.contains_key(&key) {
            return;
        }

        let task_user_hash = user_hash.clone();
        let task_folder = folder.clone();

        let handle = tokio::spawn(async move {
            idle_loop(
                &task_user_hash, &task_folder, &creds,
                &transport, sync_worker_manager,
            ).await;
        });

        self.tasks.insert(key, handle);
    }

    /// Stop the IDLE task for a specific user + folder.
    #[allow(dead_code)]
    pub async fn stop_idle(&self, user_hash: &str, folder: &str) {
        let key = (user_hash.to_string(), folder.to_string());
        if let Some((_, handle)) = self.tasks.remove(&key) {
            handle.abort();
        }
    }

    /// Stop all IDLE tasks for a user (called on WebSocket disconnect).
    pub async fn stop_all(&self, user_hash: &str) {
        let keys_to_remove: Vec<_> = self
            .tasks
            .iter()
            .filter(|entry| entry.key().0 == user_hash)
            .map(|entry| entry.key().clone())
            .collect();

        for key in keys_to_remove {
            if let Some((_, handle)) = self.tasks.remove(&key) {
                handle.abort();
            }
        }
    }
}

impl Default for IdleManager {
    fn default() -> Self {
        Self::new()
    }
}

/// The inner IDLE loop with auto-reconnect and exponential backoff.
///
/// This opens a dedicated IMAP connection, SELECTs the folder, and enters
/// IDLE mode. When the server notifies of changes, it wakes the user's
/// sync worker and re-enters IDLE. If the connection drops, it
/// reconnects with exponential backoff.
async fn idle_loop(
    user_hash: &str,
    folder: &str,
    creds: &ImapCredentials,
    transport: &Arc<MailTransport>,
    sync_worker_manager: Arc<crate::realtime::worker::SyncWorkerManager>,
) {
    let mut backoff = std::time::Duration::from_secs(1);
    let max_backoff = std::time::Duration::from_secs(60);

    loop {
        match run_idle_session(
            user_hash, folder, creds, transport,
            sync_worker_manager.clone(),
        ).await {
            Ok(()) => {
                // Session ended normally (shouldn't happen in practice).
                tracing::info!(user_hash = %user_hash, folder = %folder, "IDLE session ended normally");
                break;
            }
            Err(e) => {
                tracing::warn!(
                    user_hash = %user_hash,
                    folder = %folder,
                    error = %e,
                    backoff_secs = backoff.as_secs(),
                    "IDLE connection failed, will retry"
                );
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);
            }
        }
    }
}

/// Run a single IDLE session. Returns Err on connection/protocol errors.
///
/// The async-imap IDLE API follows an ownership pattern:
/// `Session` → `.idle()` consumes session → `Handle` → `.init()` sends IDLE
/// → `.wait()` listens → `.done()` sends DONE and returns `Session`.
async fn run_idle_session(
    user_hash: &str,
    folder: &str,
    creds: &ImapCredentials,
    transport: &Arc<MailTransport>,
    sync_worker_manager: Arc<crate::realtime::worker::SyncWorkerManager>,
) -> Result<(), String> {
    use imap::client::connect;

    let mut session = connect(creds, &transport.imap_connect_host, &transport.imap_connector)
        .await
        .map_err(|e| format!("IDLE connect failed: {e}"))?;

    session
        .select_condstore(folder)
        .await
        .map_err(|e| format!("IDLE SELECT failed: {e}"))?;

    tracing::info!(user_hash = %user_hash, folder = %folder, "IDLE session started");

    // Re-enter IDLE every 25 minutes (RFC recommends max 29 min).
    let idle_timeout = std::time::Duration::from_secs(25 * 60);

    loop {
        // `.idle()` consumes the session, wrapping it in a Handle.
        let mut idle_handle = session.idle();

        // Initialize the IDLE command with the server.
        idle_handle
            .init()
            .await
            .map_err(|e| format!("IDLE init failed: {e}"))?;

        // Start listening for server notifications.
        // IMPORTANT: `interrupt` must be kept alive — dropping it triggers
        // ManualInterrupt and ends the wait immediately.
        let (idle_wait, interrupt) = idle_handle.wait();

        // Wait for the server to send an unsolicited response, or timeout.
        let result = tokio::time::timeout(idle_timeout, idle_wait).await;

        // Drop the interrupt handle now that we're done waiting.
        drop(interrupt);

        // Send DONE to end IDLE and get the session back.
        session = idle_handle
            .done()
            .await
            .map_err(|e| format!("IDLE done failed: {e}"))?;

        match result {
            Ok(Ok(idle_response)) => {
                use async_imap::extensions::idle::IdleResponse;
                match idle_response {
                    IdleResponse::NewData(_) => {
                        tracing::info!(
                            user_hash = %user_hash,
                            folder = %folder,
                            "IDLE: new data from server"
                        );

                        // Don't reconcile here - just wake the user's sync
                        // worker, which owns the CONDSTORE/full-scan fetch,
                        // cache writes, and vacation/filter trigger. Keeps
                        // there being exactly one place that writes the
                        // cache instead of a second, IDLE-local copy of the
                        // same logic.
                        sync_worker_manager
                            .ensure_worker(user_hash.to_string(), creds.clone())
                            .notify_one();
                    }
                    IdleResponse::Timeout => {
                        tracing::debug!(
                            user_hash = %user_hash,
                            folder = %folder,
                            "IDLE: server-side timeout, re-entering"
                        );
                    }
                    IdleResponse::ManualInterrupt => {
                        tracing::debug!(
                            user_hash = %user_hash,
                            folder = %folder,
                            "IDLE: manual interrupt, re-entering"
                        );
                    }
                }
            }
            Ok(Err(e)) => {
                // IDLE protocol error.
                return Err(format!("IDLE error: {e}"));
            }
            Err(_) => {
                // Our timeout — re-enter IDLE to keep the connection alive.
                tracing::debug!(
                    user_hash = %user_hash,
                    folder = %folder,
                    "IDLE keepalive timeout, re-entering"
                );
            }
        }
    }
}

/// Apply vacation responder and filter rules to messages with uid > max_uid_before in INBOX.
/// Runs as a background task - any error is logged and silently dropped.
#[allow(clippy::too_many_arguments)]
/// Owned copy of `db::filters::MessageContext`'s fields, used to carry a
/// message's filter-matching context into a `with_user_db` closure (which
/// must be `'static`, so it can't hold borrows into `new_messages`).
struct OwnedMessageContext {
    from_address: String,
    to_addresses: String,
    cc_addresses: String,
    subject: String,
    body_snippet: String,
    size: u32,
    has_attachments: bool,
    is_reply: bool,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_new_messages(
    user_hash: &str,
    config: &AppConfig,
    creds: &ImapCredentials,
    smtp_client: &Arc<dyn SmtpClient>,
    imap_client: &Arc<dyn ImapClient>,
    transport: &MailTransport,
    event_bus: &EventBus,
    db_pool_manager: &db::pool::DbPoolManager,
    max_uid_before: u32,
) {
    struct InitialData {
        new_messages: Vec<db::messages::CachedMessage>,
        vacation: db::vacation::VacationResponder,
        rules: Vec<db::filters::FilterRule>,
    }

    let initial = db::pool::with_user_db(db_pool_manager, user_hash, move |conn| {
        // Fetch new messages from the DB cache.
        let new_messages = db::messages::get_messages_after_uid(conn, "INBOX", max_uid_before)?;
        if new_messages.is_empty() {
            return Ok(InitialData { new_messages, vacation: db::vacation::VacationResponder {
                enabled: false,
                subject: String::new(),
                body: String::new(),
                start_date: None,
                end_date: None,
                reply_interval_hours: 24,
            }, rules: Vec::new() });
        }

        let vacation = db::vacation::get_vacation(conn).unwrap_or_else(|_| db::vacation::VacationResponder {
            enabled: false,
            subject: String::new(),
            body: String::new(),
            start_date: None,
            end_date: None,
            reply_interval_hours: 24,
        });

        let rules = db::filters::list_filters(conn).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "filters: failed to list rules");
            Vec::new()
        });

        Ok(InitialData { new_messages, vacation, rules })
    })
    .await;

    let InitialData { new_messages, vacation, rules } = match initial {
        Ok(data) => data,
        Err(e) => {
            tracing::warn!(error = %e, "process_new_messages: failed to open DB or query new messages");
            return;
        }
    };

    if new_messages.is_empty() {
        return;
    }

    // --- Vacation responder ---
    if vacation.enabled {
        let today = chrono_today();
        let in_range = vacation_in_date_range(&vacation, &today);
        if in_range {
            let smtp_host = config.smtp_host.as_deref().unwrap_or_default();
            if !smtp_host.is_empty() {
                let smtp_creds = SmtpCredentials {
                    host: smtp_host.to_string(),
                    connect_host: transport.smtp_connect_host.clone(),
                    port: config.smtp_port,
                    tls: config.tls_enabled,
                    email: creds.email.clone(),
                    password: creds.password.clone(),
                    tls_params: transport.smtp_tls_params.clone(),
                };
                for msg in &new_messages {
                    // Don't reply to own address, empty sender, or automated mail.
                    if msg.from_address.is_empty()
                        || msg.from_address.eq_ignore_ascii_case(&creds.email)
                        || is_automated_sender(&msg.from_address)
                    {
                        continue;
                    }
                    let should_reply = db::pool::with_user_db(db_pool_manager, user_hash, {
                        let from_address = msg.from_address.clone();
                        let reply_interval_hours = vacation.reply_interval_hours;
                        move |conn| db::vacation::should_reply_and_record(conn, &from_address, reply_interval_hours)
                    })
                    .await;
                    match should_reply {
                        Ok(true) => {
                            let reply = SendableMessage {
                                from: creds.email.clone(),
                                to: vec![msg.from_address.clone()],
                                cc: vec![],
                                bcc: vec![],
                                subject: vacation.subject.clone(),
                                text_body: vacation.body.clone(),
                                html_body: None,
                                in_reply_to: None,
                                references: None,
                                attachments: vec![],
                                auto_submitted: true,
                                pgp: None,
                            };
                            if let Err(e) = smtp_client.send_message(&smtp_creds, &reply).await {
                                tracing::warn!(
                                    error = %e,
                                    to = %msg.from_address,
                                    "vacation: failed to send reply"
                                );
                            }
                        }
                        Ok(false) => {}
                        Err(e) => tracing::warn!(error = %e, "vacation: DB error"),
                    }
                }
                let _ = db::pool::with_user_db(db_pool_manager, user_hash, {
                    let reply_interval_hours = vacation.reply_interval_hours;
                    move |conn| db::vacation::purge_old_replies(conn, reply_interval_hours)
                })
                .await;
            }
        }
    }

    // --- Filter rules ---
    // UIDs that a filter moved or deleted out of INBOX - excluded from the
    // NewMail notification below since they're no longer there to see.
    let mut removed_uids: HashSet<u32> = HashSet::new();

    let sieve_active = config.sieve_host.is_some();

    if !rules.is_empty() {
    'msg: for msg in &new_messages {
        let matched = db::pool::with_user_db(db_pool_manager, user_hash, {
            let ctx = OwnedMessageContext {
                from_address: msg.from_address.clone(),
                to_addresses: msg.to_addresses.clone(),
                cc_addresses: msg.cc_addresses.clone(),
                subject: msg.subject.clone(),
                body_snippet: msg.snippet.clone(),
                size: msg.size,
                has_attachments: msg.has_attachments,
                is_reply: msg.in_reply_to.is_some(),
            };
            move |conn| db::filters::matching_rules(
                conn,
                &db::filters::MessageContext {
                    from_address: &ctx.from_address,
                    to_addresses: &ctx.to_addresses,
                    cc_addresses: &ctx.cc_addresses,
                    subject: &ctx.subject,
                    body_snippet: &ctx.body_snippet,
                    size: ctx.size,
                    has_attachments: ctx.has_attachments,
                    is_reply: ctx.is_reply,
                },
            )
        })
        .await;
        let matched = match matched {
            Ok(m) => m,
            Err(e) => { tracing::warn!(error = %e, "filters: matching_rules error"); continue; }
        };

        for rule in matched {
            if sieve_active && crate::sieve::is_sieve_capable(&rule) {
                continue;
            }
            for action in &rule.actions {
                match action.action_type.as_str() {
                    "mark_read" => {
                        if let Err(e) = imap_client
                            .add_flags(creds, "INBOX", msg.uid, &["\\Seen"])
                            .await
                        {
                            tracing::warn!(error = %e, uid = msg.uid, "filter mark_read failed");
                        } else {
                            let uid = msg.uid;
                            let _ = db::pool::with_user_db(db_pool_manager, user_hash, move |conn| {
                                db::messages::update_message_flags(conn, "INBOX", uid, "\\Seen")
                            })
                            .await;
                        }
                    }
                    "mark_starred" => {
                        if let Err(e) = imap_client
                            .add_flags(creds, "INBOX", msg.uid, &["\\Flagged"])
                            .await
                        {
                            tracing::warn!(error = %e, uid = msg.uid, "filter mark_starred failed");
                        } else {
                            let uid = msg.uid;
                            let _ = db::pool::with_user_db(db_pool_manager, user_hash, move |conn| {
                                db::messages::update_message_flags(conn, "INBOX", uid, "\\Flagged")
                            })
                            .await;
                        }
                    }
                    "move" => {
                        if let Some(ref target) = action.action_value {
                            if let Err(e) = imap_client
                                .move_message(creds, "INBOX", msg.uid, target)
                                .await
                            {
                                tracing::warn!(error = %e, uid = msg.uid, target = %target, "filter move failed");
                            } else {
                                let uid = msg.uid;
                                let _ = db::pool::with_user_db(db_pool_manager, user_hash, move |conn| {
                                    db::messages::delete_message(conn, "INBOX", uid)
                                })
                                .await;
                                removed_uids.insert(msg.uid);
                                // Message is no longer in INBOX; skip remaining actions.
                                continue 'msg;
                            }
                        }
                    }
                    "delete" => {
                        if let Err(e) = imap_client
                            .move_message(creds, "INBOX", msg.uid, "Trash")
                            .await
                        {
                            tracing::warn!(error = %e, uid = msg.uid, "filter delete failed");
                        } else {
                            let uid = msg.uid;
                            let _ = db::pool::with_user_db(db_pool_manager, user_hash, move |conn| {
                                db::messages::delete_message(conn, "INBOX", uid)
                            })
                            .await;
                            removed_uids.insert(msg.uid);
                            continue 'msg;
                        }
                    }
                    "tag" => {
                        if let Some(ref tag_id) = action.action_value {
                            let uid = msg.uid;
                            let tag_id = tag_id.clone();
                            let _ = db::pool::with_user_db(db_pool_manager, user_hash, move |conn| {
                                db::tags::add_tag_to_message(conn, &tag_id, uid, "INBOX")
                            })
                            .await;
                        }
                    }
                    "forward" => {
                        if let Some(ref forward_to) = action.action_value {
                            let smtp_host = config.smtp_host.as_deref().unwrap_or_default();
                            if !smtp_host.is_empty() {
                                let smtp_creds = SmtpCredentials {
                                    host: smtp_host.to_string(),
                                    connect_host: transport.smtp_connect_host.clone(),
                                    port: config.smtp_port,
                                    tls: config.tls_enabled,
                                    email: creds.email.clone(),
                                    password: creds.password.clone(),
                                    tls_params: transport.smtp_tls_params.clone(),
                                };
                                let fwd = SendableMessage {
                                    from: creds.email.clone(),
                                    to: vec![forward_to.clone()],
                                    cc: vec![],
                                    bcc: vec![],
                                    subject: format!("Fwd: {}", msg.subject),
                                    text_body: format!(
                                        "---------- Forwarded message ----------\nFrom: {}\nSubject: {}\n\n{}",
                                        msg.from_address, msg.subject, msg.snippet
                                    ),
                                    html_body: None,
                                    in_reply_to: None,
                                    references: None,
                                    attachments: vec![],
                                    auto_submitted: false,
                                    pgp: None,
                                };
                                if let Err(e) = smtp_client.send_message(&smtp_creds, &fwd).await {
                                    tracing::warn!(error = %e, uid = msg.uid, to = %forward_to, "filter forward failed");
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            if rule.stop_processing {
                break;
            }
        }
    }
    }

    // --- Notify: only messages that survived filtering count as "new mail" ---
    let survivors: Vec<&db::messages::CachedMessage> = new_messages
        .iter()
        .filter(|m| !removed_uids.contains(&m.uid))
        .collect();

    if let Some(latest) = survivors.last() {
        event_bus
            .publish(
                user_hash,
                MailEvent::NewMail {
                    folder: "INBOX".to_string(),
                    count: survivors.len() as u32,
                    latest_sender: Some(latest.from_address.clone()).filter(|s| !s.is_empty()),
                    latest_subject: Some(latest.subject.clone()),
                },
            )
            .await;
    }
}

fn chrono_today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Returns true for senders that should never receive an auto-reply.
/// Covers RFC 3834 automated-sender conventions and common bounce patterns.
fn is_automated_sender(from: &str) -> bool {
    let from = from.to_lowercase();
    let local = from.split('@').next().unwrap_or("");
    // Exact local-part matches.
    matches!(local, "mailer-daemon" | "postmaster" | "noreply" | "no-reply"
        | "donotreply" | "do-not-reply" | "auto-reply" | "auto_reply" | "autoreply")
    // Prefix patterns common in bounce/bulk addresses.
    || local.starts_with("bounce")
    || local.starts_with("return")
    || local.starts_with("prvs=")   // BATV bounce validation token
    // Substring patterns in the full address.
    || from.contains("noreply")
    || from.contains("no-reply")
    || from.contains("donotreply")
    || from.contains("auto-reply")
    || from.contains("bounces+")
    || from.contains("+caf_=")      // Google forwarding bounce marker
}

fn vacation_in_date_range(vacation: &db::vacation::VacationResponder, today: &str) -> bool {
    if let Some(ref start) = vacation.start_date
        && today < start.as_str()
    {
        return false;
    }
    if let Some(ref end) = vacation.end_date
        && today > end.as_str()
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::vacation::VacationResponder;

    // --- is_automated_sender ---

    #[test]
    fn automated_exact_local_parts() {
        for addr in &[
            "mailer-daemon@example.com",
            "postmaster@example.com",
            "noreply@example.com",
            "no-reply@example.com",
            "donotreply@example.com",
            "do-not-reply@example.com",
            "auto-reply@example.com",
            "auto_reply@example.com",
            "autoreply@example.com",
        ] {
            assert!(is_automated_sender(addr), "expected automated: {addr}");
        }
    }

    #[test]
    fn automated_prefix_patterns() {
        assert!(is_automated_sender("bounce+abc@example.com"));
        assert!(is_automated_sender("bounces+xyz@lists.example.com"));
        assert!(is_automated_sender("return-1234@example.com"));
        assert!(is_automated_sender("prvs=tag=addr@example.com"));
    }

    #[test]
    fn automated_substring_in_address() {
        assert!(is_automated_sender("notifications+noreply@github.com"));
        assert!(is_automated_sender("support+no-reply@service.com"));
        assert!(is_automated_sender("user+bounces+token@googlegroups.com"));
        assert!(is_automated_sender("me+caf_=dest@example.com"));
    }

    #[test]
    fn automated_case_insensitive() {
        assert!(is_automated_sender("MAILER-DAEMON@EXAMPLE.COM"));
        assert!(is_automated_sender("NoReply@Example.Com"));
        assert!(is_automated_sender("Bounce+Token@example.com"));
    }

    #[test]
    fn real_senders_not_automated() {
        for addr in &[
            "alice@example.com",
            "bob.smith@company.org",
            "support@helpdesk.com",
            "newsletter@news.example.com",
            "info@company.com",
            // "reply" alone should not match
            "reply@example.com",
        ] {
            assert!(!is_automated_sender(addr), "expected real sender: {addr}");
        }
    }

    // --- vacation_in_date_range ---

    fn base_vacation() -> VacationResponder {
        VacationResponder {
            enabled: true,
            subject: "OOO".to_string(),
            body: String::new(),
            start_date: None,
            end_date: None,
            reply_interval_hours: 24,
        }
    }

    #[test]
    fn no_date_constraints_always_in_range() {
        let v = base_vacation();
        assert!(vacation_in_date_range(&v, "2026-01-01"));
        assert!(vacation_in_date_range(&v, "2099-12-31"));
    }

    #[test]
    fn start_date_gates_early_dates() {
        let v = VacationResponder { start_date: Some("2026-07-01".to_string()), ..base_vacation() };
        assert!(!vacation_in_date_range(&v, "2026-06-30"));
        assert!(vacation_in_date_range(&v, "2026-07-01"));
        assert!(vacation_in_date_range(&v, "2026-07-02"));
    }

    #[test]
    fn end_date_gates_late_dates() {
        let v = VacationResponder { end_date: Some("2026-07-31".to_string()), ..base_vacation() };
        assert!(vacation_in_date_range(&v, "2026-07-31"));
        assert!(!vacation_in_date_range(&v, "2026-08-01"));
    }

    #[test]
    fn both_dates_form_closed_range() {
        let v = VacationResponder {
            start_date: Some("2026-07-01".to_string()),
            end_date: Some("2026-07-14".to_string()),
            ..base_vacation()
        };
        assert!(!vacation_in_date_range(&v, "2026-06-30"));
        assert!(vacation_in_date_range(&v, "2026-07-01"));
        assert!(vacation_in_date_range(&v, "2026-07-14"));
        assert!(!vacation_in_date_range(&v, "2026-07-15"));
    }
}
