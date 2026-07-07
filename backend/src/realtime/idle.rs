use std::sync::Arc;

use dashmap::DashMap;
use futures::StreamExt;
use tokio::task::JoinHandle;

use crate::config::AppConfig;
use crate::imap::client::{ImapClient, ImapCredentials};
use crate::imap::connection::ImapStream;
use crate::imap::parse::{decode_rfc2047, flag_to_string, has_attachments, imap_address_to_email};
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
    #[allow(clippy::too_many_arguments)]
    pub async fn start_idle(
        &self,
        user_hash: String,
        folder: String,
        creds: ImapCredentials,
        event_bus: Arc<EventBus>,
        config: Arc<AppConfig>,
        transport: Arc<MailTransport>,
        smtp_client: Arc<dyn SmtpClient>,
        imap_client: Arc<dyn ImapClient>,
    ) {
        let key = (user_hash.clone(), folder.clone());
        if self.tasks.contains_key(&key) {
            return;
        }

        let task_user_hash = user_hash.clone();
        let task_folder = folder.clone();

        let handle = tokio::spawn(async move {
            idle_loop(
                &task_user_hash, &task_folder, &creds, &event_bus,
                &config, &transport, smtp_client, imap_client,
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
/// IDLE mode. When the server notifies of changes, it publishes an event
/// to the EventBus and re-enters IDLE. If the connection drops, it
/// reconnects with exponential backoff.
#[allow(clippy::too_many_arguments)]
async fn idle_loop(
    user_hash: &str,
    folder: &str,
    creds: &ImapCredentials,
    event_bus: &EventBus,
    config: &AppConfig,
    transport: &Arc<MailTransport>,
    smtp_client: Arc<dyn SmtpClient>,
    imap_client: Arc<dyn ImapClient>,
) {
    let mut backoff = std::time::Duration::from_secs(1);
    let max_backoff = std::time::Duration::from_secs(60);

    loop {
        match run_idle_session(
            user_hash, folder, creds, event_bus, config, transport,
            smtp_client.clone(), imap_client.clone(),
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
#[allow(clippy::too_many_arguments)]
async fn run_idle_session(
    user_hash: &str,
    folder: &str,
    creds: &ImapCredentials,
    event_bus: &EventBus,
    config: &AppConfig,
    transport: &Arc<MailTransport>,
    smtp_client: Arc<dyn SmtpClient>,
    imap_client: Arc<dyn ImapClient>,
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

                        // CONDSTORE sync is fast (~200-500ms), so run it
                        // BEFORE publishing. This ensures the DB is up-to-date
                        // when the frontend refetches.
                        let result;
                        (session, result) = fetch_new_after_idle(
                            session, user_hash, folder, config,
                        )
                        .await;

                        // Publish the correct event based on what changed.
                        if result.new_count > 0 {
                            // Spawn vacation + filter processing as a background
                            // task so it doesn't delay re-entering IDLE.
                            if folder == "INBOX" {
                                let bkg_user_hash = user_hash.to_string();
                                let bkg_config = config.clone();
                                let bkg_creds = creds.clone();
                                let bkg_smtp = smtp_client.clone();
                                let bkg_imap = imap_client.clone();
                                let bkg_transport = transport.clone();
                                let bkg_max_uid = result.max_uid_before;
                                tokio::spawn(async move {
                                    process_new_messages(
                                        &bkg_user_hash,
                                        &bkg_config,
                                        &bkg_creds,
                                        &bkg_smtp,
                                        &bkg_imap,
                                        &bkg_transport,
                                        bkg_max_uid,
                                    ).await;
                                });
                            }

                            event_bus
                                .publish(
                                    user_hash,
                                    MailEvent::NewMessages {
                                        folder: folder.to_string(),
                                        count: result.new_count,
                                        latest_sender: result.latest_sender,
                                        latest_subject: result.latest_subject,
                                    },
                                )
                                .await;
                        } else if result.flags_updated > 0 || result.deleted_count > 0 {
                            event_bus
                                .publish(
                                    user_hash,
                                    MailEvent::FlagsChanged {
                                        folder: folder.to_string(),
                                    },
                                )
                                .await;
                        }
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

/// Summary of what changed during a post-IDLE reconciliation.
struct ReconcileResult {
    new_count: u32,
    flags_updated: u32,
    deleted_count: u32,
    latest_sender: Option<String>,
    latest_subject: Option<String>,
    /// Highest UID that was in the DB before this sync - used to find new messages.
    max_uid_before: u32,
}

impl ReconcileResult {
    fn empty() -> Self {
        Self {
            new_count: 0,
            flags_updated: 0,
            deleted_count: 0,
            latest_sender: None,
            latest_subject: None,
            max_uid_before: 0,
        }
    }
}

/// Reconcile the local cache after IDLE detects new data.
///
/// Uses CONDSTORE when available for fast incremental sync (~200-500ms),
/// falling back to a full UID scan when CONDSTORE is not supported.
///
/// If anything fails, it logs a warning and returns the session unchanged —
/// the periodic sync loop provides a safety net.
async fn fetch_new_after_idle(
    mut session: async_imap::Session<ImapStream>,
    user_hash: &str,
    folder: &str,
    config: &AppConfig,
) -> (async_imap::Session<ImapStream>, ReconcileResult) {
    // Read cached folder state from DB.
    let (cached_modseq, max_cached_uid, cached_count) =
        match db::pool::open_user_db(&config.data_dir, user_hash) {
            Ok(conn) => {
                let modseq = db::folders::get_folder(&conn, folder)
                    .ok()
                    .flatten()
                    .map(|f| f.highest_modseq)
                    .unwrap_or(0);
                let max_uid = db::messages::max_uid(&conn, folder).unwrap_or(0);
                let count = db::messages::count_messages(&conn, folder).unwrap_or(0);
                (modseq, max_uid, count)
            }
            Err(e) => {
                tracing::warn!(error = %e, "IDLE fetch: failed to open DB");
                return (session, ReconcileResult::empty());
            }
        };

    // Re-SELECT with CONDSTORE to get fresh mailbox state.
    let mailbox = match session.select_condstore(folder).await {
        Ok(mb) => mb,
        Err(e) => {
            tracing::warn!(error = %e, "IDLE fetch: select_condstore failed");
            return (session, ReconcileResult::empty());
        }
    };

    let server_exists = mailbox.exists;
    let server_modseq = mailbox.highest_modseq.unwrap_or(0);
    let uid_validity = mailbox.uid_validity.unwrap_or(0);

    // Early exit: nothing changed (CONDSTORE modseq and count both match).
    if server_modseq > 0 && server_modseq == cached_modseq && server_exists == cached_count {
        tracing::debug!(
            user_hash = %user_hash,
            folder = %folder,
            modseq = server_modseq,
            "IDLE fetch: nothing changed (modseq match)"
        );
        return (session, ReconcileResult::empty());
    }

    // ── CONDSTORE fast path ──────────────────────────────────────────
    if cached_modseq > 0 && server_modseq > 0 {
        let fetch_query = format!("(UID FLAGS) (CHANGEDSINCE {})", cached_modseq);
        let changed_items: Option<Vec<(u32, Vec<String>)>> =
            match session.uid_fetch("1:*", &fetch_query).await {
                Ok(stream) => {
                    let mut stream = std::pin::pin!(stream);
                    let mut items = Vec::new();
                    while let Some(result) = stream.next().await {
                        match result {
                            Ok(fetch) => {
                                if let Some(uid) = fetch.uid {
                                    let flags: Vec<String> =
                                        fetch.flags().map(|f| flag_to_string(&f)).collect();
                                    items.push((uid, flags));
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    "IDLE fetch: error reading CHANGEDSINCE stream"
                                );
                                break;
                            }
                        }
                    }
                    Some(items)
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "IDLE fetch: CHANGEDSINCE fetch failed, falling back to full scan"
                    );
                    None
                }
            };

        if let Some(changed) = changed_items {
            // Separate flag changes (uid ≤ max) from new messages (uid > max).
            let mut flag_changes: Vec<(u32, Vec<String>)> = Vec::new();
            let mut has_new = false;
            for (uid, flags) in changed {
                if uid > max_cached_uid {
                    has_new = true;
                } else {
                    flag_changes.push((uid, flags));
                }
            }

            // Apply flag changes to DB.
            let mut flags_updated = 0u32;
            if !flag_changes.is_empty()
                && let Ok(conn) = db::pool::open_user_db(&config.data_dir, user_hash)
            {
                for (uid, flags) in &flag_changes {
                    let mut sorted = flags.clone();
                    sorted.sort();
                    let flags_csv = sorted.join(",");
                    if db::messages::update_message_flags(&conn, folder, *uid, &flags_csv)
                        .is_ok()
                    {
                        flags_updated += 1;
                    }
                }
            }

            // Fetch full headers for new messages.
            let mut new_count = 0u32;
            if has_new {
                let uid_range = format!("{}:*", max_cached_uid + 1);
                new_count = fetch_and_store_headers(
                    &mut session, user_hash, folder, config, &uid_range, max_cached_uid,
                )
                .await;
            }

            // Detect deletions if server count < cached count.
            let mut deleted_count = 0u32;
            if server_exists < cached_count {
                deleted_count =
                    fetch_uids_and_remove_deleted(&mut session, user_hash, folder, config).await;
            }

            // Look up the latest message's sender/subject if new messages arrived.
            let (latest_sender, latest_subject) = if new_count > 0 {
                lookup_latest_message(config, user_hash, folder)
            } else {
                (None, None)
            };

            // Update folder sync status with new modseq.
            if let Ok(conn) = db::pool::open_user_db(&config.data_dir, user_hash) {
                let _ = db::folders::update_folder_sync_status(
                    &conn, folder, uid_validity, server_exists, server_modseq,
                );
                let _ = db::folders::refresh_unread_count(&conn, folder);
            }

            if flags_updated > 0 || new_count > 0 || deleted_count > 0 {
                tracing::info!(
                    user_hash = %user_hash,
                    folder = %folder,
                    flags_updated = flags_updated,
                    new = new_count,
                    deleted = deleted_count,
                    "Reconciled cache after IDLE (CONDSTORE)"
                );
            }

            return (session, ReconcileResult {
                new_count,
                flags_updated,
                deleted_count,
                latest_sender,
                latest_subject,
                max_uid_before: max_cached_uid,
            });
        }
        // CHANGEDSINCE fetch failed — fall through to full scan below.
    }

    // ── Fallback: full UID scan (no CONDSTORE) ───────────────────────
    let cached_uids: std::collections::HashSet<u32> =
        match db::pool::open_user_db(&config.data_dir, user_hash) {
            Ok(conn) => {
                let uids_flags =
                    db::messages::get_all_uids_and_flags(&conn, folder).unwrap_or_default();
                uids_flags.iter().map(|(uid, _)| *uid).collect()
            }
            Err(e) => {
                tracing::warn!(error = %e, "IDLE fetch: failed to open DB for fallback");
                return (session, ReconcileResult::empty());
            }
        };

    // Fetch all UIDs from server.
    let imap_uids: Option<std::collections::HashSet<u32>> =
        match session.uid_fetch("1:*", "UID").await {
            Ok(stream) => {
                let mut stream = std::pin::pin!(stream);
                let mut uids = std::collections::HashSet::new();
                while let Some(result) = stream.next().await {
                    match result {
                        Ok(fetch) => {
                            if let Some(uid) = fetch.uid {
                                uids.insert(uid);
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "IDLE fetch: error reading UID stream");
                            break;
                        }
                    }
                }
                Some(uids)
            }
            Err(e) => {
                tracing::warn!(error = %e, "IDLE fetch: uid_fetch(UIDs) failed");
                None
            }
        };

    let Some(imap_uids) = imap_uids else {
        return (session, ReconcileResult::empty());
    };

    // Fetch headers for new messages.
    let has_new = imap_uids.iter().any(|&uid| uid > max_cached_uid);
    let mut new_count = 0u32;
    if has_new {
        let uid_range = format!("{}:*", max_cached_uid + 1);
        new_count = fetch_and_store_headers(
            &mut session, user_hash, folder, config, &uid_range, max_cached_uid,
        )
        .await;
    }

    // Delete cached UIDs no longer on server.
    let mut deleted_count = 0u32;
    let deleted_uids: Vec<u32> = cached_uids
        .iter()
        .copied()
        .filter(|uid| !imap_uids.contains(uid))
        .collect();
    if !deleted_uids.is_empty()
        && let Ok(conn) = db::pool::open_user_db(&config.data_dir, user_hash)
    {
        for uid in &deleted_uids {
            if db::messages::delete_message(&conn, folder, *uid).is_ok() {
                deleted_count += 1;
            }
        }
    }

    // Update folder sync status.
    if let Ok(conn) = db::pool::open_user_db(&config.data_dir, user_hash) {
        let _ = db::folders::update_folder_sync_status(
            &conn, folder, uid_validity, server_exists, server_modseq,
        );
        let _ = db::folders::refresh_unread_count(&conn, folder);
    }

    // Look up the latest message's sender/subject if new messages arrived.
    let (latest_sender, latest_subject) = if new_count > 0 {
        lookup_latest_message(config, user_hash, folder)
    } else {
        (None, None)
    };

    if new_count > 0 || deleted_count > 0 {
        tracing::info!(
            user_hash = %user_hash,
            folder = %folder,
            new = new_count,
            deleted = deleted_count,
            "Reconciled cache after IDLE (full scan)"
        );
    }

    (session, ReconcileResult {
        new_count,
        flags_updated: 0,
        deleted_count,
        latest_sender,
        latest_subject,
        max_uid_before: max_cached_uid,
    })
}

/// Look up the most recent message in the DB for sender/subject details.
fn lookup_latest_message(
    config: &AppConfig,
    user_hash: &str,
    folder: &str,
) -> (Option<String>, Option<String>) {
    let Ok(conn) = db::pool::open_user_db(&config.data_dir, user_hash) else {
        return (None, None);
    };
    let max_uid = db::messages::max_uid(&conn, folder).unwrap_or(0);
    if max_uid == 0 {
        return (None, None);
    }
    match db::messages::get_single_message(&conn, folder, max_uid) {
        Ok(Some(msg)) => {
            let sender = if !msg.from_name.is_empty() {
                Some(msg.from_name)
            } else if !msg.from_address.is_empty() {
                Some(msg.from_address)
            } else {
                None
            };
            let subject = if !msg.subject.is_empty() {
                Some(msg.subject)
            } else {
                None
            };
            (sender, subject)
        }
        _ => (None, None),
    }
}

/// Fetch full headers for messages in the given UID range and store them in the DB.
/// Skips messages with UID ≤ `min_uid` (handles the `*` wildcard returning the last message).
/// Returns the number of new messages stored.
async fn fetch_and_store_headers(
    session: &mut async_imap::Session<ImapStream>,
    user_hash: &str,
    folder: &str,
    config: &AppConfig,
    uid_range: &str,
    min_uid: u32,
) -> u32 {
    let fetches: Option<Vec<_>> = match session
        .uid_fetch(uid_range, crate::imap::client::HEADER_FETCH_ITEMS)
        .await
    {
        Ok(stream) => {
            let mut stream = std::pin::pin!(stream);
            let mut items = Vec::new();
            while let Some(result) = stream.next().await {
                match result {
                    Ok(fetch) => items.push(fetch),
                    Err(e) => {
                        tracing::warn!(error = %e, "IDLE fetch: error reading header stream");
                        break;
                    }
                }
            }
            Some(items)
        }
        Err(e) => {
            tracing::warn!(error = %e, "IDLE fetch: uid_fetch(headers) failed");
            None
        }
    };

    let mut new_count = 0u32;
    if let Some(fetches) = fetches
        && let Ok(conn) = db::pool::open_user_db(&config.data_dir, user_hash)
    {
        for fetch in &fetches {
            let uid = match fetch.uid {
                Some(u) if u > min_uid => u,
                Some(_) => continue, // UID "*" can return the last message
                None => continue,
            };

            // Parse threading headers.
            let raw_header_bytes = fetch.header();
            let parsed_threading = raw_header_bytes
                .and_then(|raw| mail_parser::MessageParser::default().parse(raw));

            let (subject, from, to, cc, date) = if let Some(env) = fetch.envelope() {
                let subject = env
                    .subject
                    .as_ref()
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .map(decode_rfc2047);

                let from: Vec<_> = env
                    .from
                    .as_ref()
                    .map(|addrs| addrs.iter().map(imap_address_to_email).collect())
                    .unwrap_or_default();

                let to: Vec<_> = env
                    .to
                    .as_ref()
                    .map(|addrs| addrs.iter().map(imap_address_to_email).collect())
                    .unwrap_or_default();

                let cc: Vec<_> = env
                    .cc
                    .as_ref()
                    .map(|addrs| addrs.iter().map(imap_address_to_email).collect())
                    .unwrap_or_default();

                let date = env
                    .date
                    .as_ref()
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .map(|s| s.to_string());

                (subject, from, to, cc, date)
            } else {
                (None, vec![], vec![], vec![], None)
            };

            let message_id = parsed_threading
                .as_ref()
                .and_then(|p| p.message_id().map(|s| format!("<{s}>")));
            let in_reply_to = parsed_threading.as_ref().and_then(|p| {
                let val = p.in_reply_to();
                val.as_text().map(|s| format!("<{s}>"))
            });
            let references = parsed_threading.as_ref().and_then(|p| {
                let val = p.references();
                val.as_text_list()
                    .map(|list| {
                        list.iter()
                            .map(|s| format!("<{s}>"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .or_else(|| val.as_text().map(|s| format!("<{s}>")))
            });

            // Detect Outlook/Exchange reaction emails from headers.
            let reaction = raw_header_bytes.and_then(|raw| {
                let header_str = std::str::from_utf8(raw).ok()?;
                let lower = header_str.to_lowercase();
                let is_reaction = lower.contains("content-class: activitynotification")
                    || lower.contains("urn:content-class:reaction");
                if !is_reaction {
                    return None;
                }
                let subj = subject.as_deref().unwrap_or("").to_lowercase();
                let emoji = match subj.trim() {
                    s if s.contains("like") => "\u{1f44d}",
                    s if s.contains("heart") || s.contains("love") => "\u{2764}\u{fe0f}",
                    s if s.contains("laugh") => "\u{1f604}",
                    s if s.contains("surprised") || s.contains("wow") => "\u{1f62e}",
                    s if s.contains("sad") => "\u{1f622}",
                    s if s.contains("angry") => "\u{1f620}",
                    _ => "\u{1f44d}",
                };
                Some(emoji.to_string())
            });

            let flags: Vec<String> = fetch.flags().map(|f| flag_to_string(&f)).collect();
            let has_attach = fetch
                .bodystructure()
                .map(|bs| has_attachments(bs))
                .unwrap_or(false);
            let size = fetch.size.unwrap_or(0);

            let from_address = from.first().map(|a| a.address.as_str()).unwrap_or("");
            let from_name = from.first().and_then(|a| a.name.as_deref()).unwrap_or("");
            let to_json = serde_json::to_string(&to).unwrap_or_else(|_| "[]".to_string());
            let cc_json = serde_json::to_string(&cc).unwrap_or_else(|_| "[]".to_string());
            let subject_str = subject.as_deref().unwrap_or("");
            let date_str = date.as_deref().unwrap_or("");
            let date_epoch = {
                let from_header = db::messages::parse_date_epoch(date_str);
                if from_header > 0 {
                    from_header
                } else {
                    fetch.internal_date().map(|d| d.timestamp()).unwrap_or(0)
                }
            };
            let flags_csv = flags.join(",");

            if let Err(e) = db::messages::upsert_message(
                &conn,
                folder,
                uid,
                message_id.as_deref(),
                in_reply_to.as_deref(),
                references.as_deref(),
                subject_str,
                from_address,
                from_name,
                &to_json,
                &cc_json,
                date_str,
                date_epoch,
                &flags_csv,
                size,
                has_attach,
                "",
                reaction.as_deref(),
            ) {
                tracing::warn!(uid = uid, error = %e, "IDLE fetch: failed to upsert message");
            } else {
                // Populate denormalized known_addresses table.
                if let Err(e) = db::contacts::populate_known_addresses(
                    &conn, from_address, from_name, &to_json, &cc_json,
                ) {
                    tracing::warn!(uid = uid, error = %e, "IDLE fetch: failed to populate known addresses");
                }
                new_count += 1;
            }
        }
    }

    new_count
}

/// Fetch all UIDs from the server and remove cached messages no longer present.
/// Used for deletion detection when server count < cached count.
/// Returns the number of deleted messages.
async fn fetch_uids_and_remove_deleted(
    session: &mut async_imap::Session<ImapStream>,
    user_hash: &str,
    folder: &str,
    config: &AppConfig,
) -> u32 {
    let imap_uids: Option<std::collections::HashSet<u32>> =
        match session.uid_fetch("1:*", "UID").await {
            Ok(stream) => {
                let mut stream = std::pin::pin!(stream);
                let mut uids = std::collections::HashSet::new();
                while let Some(result) = stream.next().await {
                    match result {
                        Ok(fetch) => {
                            if let Some(uid) = fetch.uid {
                                uids.insert(uid);
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "IDLE fetch: error reading UID stream for deletion check"
                            );
                            break;
                        }
                    }
                }
                Some(uids)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "IDLE fetch: uid_fetch(UIDs) failed for deletion check"
                );
                None
            }
        };

    let Some(imap_uids) = imap_uids else {
        return 0;
    };

    let mut deleted_count = 0u32;
    if let Ok(conn) = db::pool::open_user_db(&config.data_dir, user_hash) {
        let cached = db::messages::get_all_uids_and_flags(&conn, folder).unwrap_or_default();
        for (uid, _) in &cached {
            if !imap_uids.contains(uid)
                && db::messages::delete_message(&conn, folder, *uid).is_ok()
            {
                deleted_count += 1;
            }
        }
    }

    deleted_count
}

/// Apply vacation responder and filter rules to messages with uid > max_uid_before in INBOX.
/// Runs as a background task - any error is logged and silently dropped.
async fn process_new_messages(
    user_hash: &str,
    config: &AppConfig,
    creds: &ImapCredentials,
    smtp_client: &Arc<dyn SmtpClient>,
    imap_client: &Arc<dyn ImapClient>,
    transport: &MailTransport,
    max_uid_before: u32,
) {
    let conn = match db::pool::open_user_db(&config.data_dir, user_hash) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "process_new_messages: failed to open DB");
            return;
        }
    };

    // Fetch new messages from the DB cache.
    let new_messages = match db::messages::get_messages_after_uid(&conn, "INBOX", max_uid_before) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "process_new_messages: failed to query new messages");
            return;
        }
    };

    if new_messages.is_empty() {
        return;
    }

    // --- Vacation responder ---
    let vacation = db::vacation::get_vacation(&conn).unwrap_or_else(|_| db::vacation::VacationResponder {
        enabled: false,
        subject: String::new(),
        body: String::new(),
        start_date: None,
        end_date: None,
        reply_interval_hours: 24,
    });

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
                    match db::vacation::should_reply_and_record(
                        &conn, &msg.from_address, vacation.reply_interval_hours,
                    ) {
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
                let _ = db::vacation::purge_old_replies(&conn, vacation.reply_interval_hours);
            }
        }
    }

    // --- Filter rules ---
    let rules = match db::filters::list_filters(&conn) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "filters: failed to list rules");
            return;
        }
    };

    if rules.is_empty() {
        return;
    }

    let sieve_active = config.sieve_host.is_some();

    'msg: for msg in &new_messages {
        let matched = db::filters::matching_rules(
            &conn,
            &db::filters::MessageContext {
                from_address: &msg.from_address,
                to_addresses: &msg.to_addresses,
                cc_addresses: &msg.cc_addresses,
                subject: &msg.subject,
                body_snippet: &msg.snippet,
                size: msg.size,
                has_attachments: msg.has_attachments,
                is_reply: msg.in_reply_to.is_some(),
            },
        );
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
                            let _ = db::messages::update_message_flags(
                                &conn, "INBOX", msg.uid, "\\Seen",
                            );
                        }
                    }
                    "mark_starred" => {
                        if let Err(e) = imap_client
                            .add_flags(creds, "INBOX", msg.uid, &["\\Flagged"])
                            .await
                        {
                            tracing::warn!(error = %e, uid = msg.uid, "filter mark_starred failed");
                        } else {
                            let _ = db::messages::update_message_flags(
                                &conn, "INBOX", msg.uid, "\\Flagged",
                            );
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
                                let _ = db::messages::delete_message(&conn, "INBOX", msg.uid);
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
                            let _ = db::messages::delete_message(&conn, "INBOX", msg.uid);
                            continue 'msg;
                        }
                    }
                    "tag" => {
                        if let Some(ref tag_id) = action.action_value {
                            let _ = db::tags::add_tag_to_message(&conn, tag_id, msg.uid, "INBOX");
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
