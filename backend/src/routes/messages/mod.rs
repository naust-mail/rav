use std::sync::Arc;

use axum::extract::{Path, Query};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

pub mod types;
use types::*;

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::db;
use crate::email_theme;
use crate::error::AppError;
use crate::folder_cipher::FolderId;
use crate::imap::client::{ImapClient, ImapCredentials};
use crate::realtime::events::{EventBus, MailEvent};
use crate::realtime::worker::SyncWorkerManager;
use crate::search::engine::{IndexableMessage, SearchEngine, UserIndex};

// ---------------------------------------------------------------------------
// Helper: build IMAP credentials from session + config
// ---------------------------------------------------------------------------

fn build_creds(session: &SessionState, config: &AppConfig) -> Result<ImapCredentials, AppError> {
    let imap_host = config
        .imap_host
        .as_deref()
        .ok_or_else(|| AppError::ServiceUnavailable("Mail server not configured".to_string()))?;

    Ok(ImapCredentials {
        host: imap_host.to_string(),
        port: config.imap_port,
        tls: config.tls_enabled,
        email: session.email.clone(),
        password: session.password.clone(),
    })
}

// ---------------------------------------------------------------------------
// Helper: validate IMAP flags from client input
// ---------------------------------------------------------------------------

/// Returns `true` if `flag` is a syntactically valid IMAP flag.
///
/// Accepts RFC 3501 system flags (`\Seen`, `\Answered`, `\Flagged`,
/// `\Deleted`, `\Draft`, `\Recent`) and keyword atoms (`Junk`,
/// `$Forwarded`, `$MDNSent`, etc.).
///
/// This is a restricted safe subset of the RFC 3501 §9 atom grammar:
/// an optional leading `\` followed by `[A-Za-z0-9$_\-.+]`, capped at
/// 64 characters. The cap is well above any flag seen in practice and
/// keeps the set narrow enough to exclude whitespace, parentheses, and
/// control characters without needing to enumerate every RFC exclusion.
fn is_valid_flag(flag: &str) -> bool {
    if flag.is_empty() || flag.len() > 64 {
        return false;
    }
    // Strip optional leading backslash (system flags like \Seen).
    let atom = flag.strip_prefix('\\').unwrap_or(flag);
    !atom.is_empty()
        && atom
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'$' | b'_' | b'-' | b'+' | b'.'))
}

fn validate_flags(flags: &[String]) -> Result<(), AppError> {
    for flag in flags {
        if !is_valid_flag(flag) {
            return Err(AppError::BadRequest("invalid flag".to_string()));
        }
    }
    Ok(())
}

fn cipher_for(session: &SessionState) -> crate::folder_cipher::FolderCipher {
    crate::folder_cipher::FolderCipher::new(&session.folder_key)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// How many seconds a folder's message cache is considered fresh.
const FOLDER_MESSAGES_TTL_SECS: u32 = 30;

/// `GET /api/folders/:folder/messages?page=0&per_page=50`
///
/// Returns paginated messages using a cache-first strategy:
/// 1. If the folder was synced within `FOLDER_MESSAGES_TTL_SECS`, serve from cache.
/// 2. Otherwise do a lightweight IMAP SELECT to check for new messages and sync
///    only what's new.
/// Result of the pre-IMAP cache check in `list_messages`.
struct InitialFolderCheck {
    folder_fresh: bool,
    cached_uid_validity: u32,
    cached_count: u32,
    folder_invalidated: bool,
}

/// What `list_messages` should do after the post-IMAP-status DB update, once
/// it knows whether a sync is needed and (if so) which kind.
enum SyncOutcome {
    /// No cached messages at all: caller should fetch recent headers via IMAP
    /// and then call `finish_cold_start_sync`.
    ColdStart,
    /// Some cached data exists; the sync worker was poked to catch up.
    StalePoke,
    /// Cache is already up to date.
    NoSyncNeeded,
}

#[allow(clippy::too_many_arguments)]
pub async fn list_messages(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(search_engine): Extension<Arc<SearchEngine>>,
    Extension(event_bus): Extension<Arc<EventBus>>,
    Extension(sync_worker_manager): Extension<Arc<SyncWorkerManager>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(folder_id): Path<FolderId>,
    Query(query): Query<ListMessagesQuery>,
) -> Result<Response, AppError> {
    let folder = cipher_for(&session).decrypt(&folder_id)?;
    let mut syncing = false;

    // Check the cache and, if the folder isn't fresh, gather what we need to
    // decide whether/how to sync. Kept in its own pooled connection block
    // since the IMAP round-trip below can't happen while holding a connection.
    let initial = {
        let folder = folder.clone();
        db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
            let folder_fresh = db::folders::is_folder_fresh(conn, &folder, FOLDER_MESSAGES_TTL_SECS)?;
            if folder_fresh {
                return Ok(InitialFolderCheck {
                    folder_fresh: true,
                    cached_uid_validity: 0,
                    cached_count: 0,
                    folder_invalidated: false,
                });
            }

            let cached_folder = db::folders::get_folder(conn, &folder)?;
            let cached_uid_validity = cached_folder.as_ref().map(|f| f.uid_validity).unwrap_or(0);

            // Ensure the folder exists in the folders table (for FK constraint).
            if cached_folder.is_none() {
                db::folders::upsert_folder(conn, &folder, None, None, "", true, 0, 0, 0, 0)?;
            }

            let cached_count = db::messages::count_messages(conn, &folder)?;
            let folder_invalidated = db::folders::is_folder_messages_invalidated(conn, &folder).unwrap_or(false);

            Ok(InitialFolderCheck {
                folder_fresh: false,
                cached_uid_validity,
                cached_count,
                folder_invalidated,
            })
        })
        .await
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?
    };

    if !initial.folder_fresh {
        let creds = build_creds(&session, &config)?;

        // Do a lightweight IMAP SELECT to get folder status.
        let status = imap_client
            .folder_status(&creds, &folder)
            .await
            .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;

        tracing::info!(
            folder = %folder,
            cached_uid_validity = initial.cached_uid_validity,
            imap_uid_validity = status.uid_validity,
            cached_count = initial.cached_count,
            imap_exists = status.exists,
            "list_messages: folder status check"
        );

        let needs_full_sync = initial.cached_uid_validity != 0
            && initial.cached_uid_validity != status.uid_validity;

        // Apply the UIDVALIDITY/no-sync-needed bookkeeping and figure out
        // which sync path to take. The cold-start path needs another IMAP
        // round-trip before it can finish writing, so it's completed outside
        // this connection block.
        let outcome = {
            let folder = folder.clone();
            db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
                if needs_full_sync {
                    tracing::info!(folder = %folder, "UIDVALIDITY changed, clearing cache");
                    db::messages::delete_folder_messages(conn, &folder)?;
                }

                let needs_sync = needs_full_sync
                    || initial.cached_count == 0
                    || status.exists != initial.cached_count
                    || initial.folder_invalidated;

                if !needs_sync {
                    // No sync needed but still update the timestamp so TTL resets.
                    db::folders::update_folder_status(conn, &folder, status.uid_validity, status.exists)?;
                    return Ok(SyncOutcome::NoSyncNeeded);
                }

                // Cold start: no cached messages at all — fetch the ~100 most
                // recent synchronously for a fast first paint, then
                // background-sync the rest.
                if (needs_full_sync || initial.cached_count == 0) && status.exists > 0 {
                    return Ok(SyncOutcome::ColdStart);
                }

                // Stale cache: we have some cached data. This is the same
                // "find new mail past the cache" job the sync worker already
                // does on every wake-up, so just poke it instead of running
                // a second, uncoordinated fetch here.
                db::folders::update_folder_status(conn, &folder, status.uid_validity, status.exists)?;
                Ok(SyncOutcome::StalePoke)
            })
            .await
            .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?
        };

        match outcome {
            SyncOutcome::ColdStart => {
                let recent_start = status.uid_next.saturating_sub(100).max(1);
                let recent_range = format!("{recent_start}:*");

                tracing::info!(folder = %folder, uid_range = %recent_range, "Partial sync: fetching recent headers");

                let headers = imap_client
                    .fetch_headers(&creds, &folder, &recent_range)
                    .await
                    .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;

                tracing::info!(folder = %folder, fetched = headers.len(), "Partial sync: fetched recent headers");

                {
                    let folder = folder.clone();
                    let search_engine = search_engine.clone();
                    let user_hash = session.user_hash.clone();
                    db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
                        upsert_and_index_headers(conn, &folder, &headers, &search_engine, &user_hash)?;

                        // Update folder metadata with UIDVALIDITY and partial count.
                        db::folders::update_folder_status(conn, &folder, status.uid_validity, status.exists)?;
                        db::folders::refresh_unread_count(conn, &folder)?;
                        Ok(())
                    })
                    .await
                    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
                }

                // Spawn background task to fetch the remaining older headers.
                if recent_start > 1 {
                    let remaining_range = format!("1:{}", recent_start - 1);
                    tokio::spawn(sync_remaining_headers(BackgroundSyncParams {
                        creds,
                        folder: folder.clone(),
                        imap_client: imap_client.clone(),
                        db_pool_manager: db_pool_manager.clone(),
                        user_hash: session.user_hash.clone(),
                        search_engine: search_engine.clone(),
                        event_bus: event_bus.clone(),
                        uid_range: remaining_range,
                        uid_validity: status.uid_validity,
                        exists: status.exists,
                    }));
                    syncing = true;
                }
            }
            SyncOutcome::StalePoke => {
                sync_worker_manager
                    .ensure_worker(session.user_hash.clone(), creds)
                    .notify_one();
                syncing = true;
            }
            SyncOutcome::NoSyncNeeded => {}
        }
    }

    // Query paginated threaded results from cache.
    struct PageResult {
        threaded: Vec<db::messages::ThreadedMessage>,
        total_count: u32,
        tags_map: std::collections::HashMap<(u32, String), Vec<db::tags::MessageTag>>,
    }

    let PageResult { threaded, total_count, tags_map } = {
        let folder = folder.clone();
        db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
            let threaded = db::messages::get_threaded_messages(conn, &folder, query.page, query.per_page)?;
            let total_count = db::messages::count_threads(conn, &folder)?;

            // For Drafts, keep total_count in sync with thread count so the folder
            // badge matches the list (raw IMAP EXISTS inflates it when phantom
            // copies exist).
            if let Ok(Some(f)) = db::folders::get_folder(conn, &folder)
                && f.flags.contains("\\Drafts") {
                    let _ = db::folders::set_folder_total_count(conn, &folder, total_count);
                }

            // Batch-fetch tags for all messages in this page.
            let message_refs: Vec<(u32, &str)> = threaded.iter().map(|t| (t.msg.uid, t.msg.folder.as_str())).collect();
            let tags_map = db::tags::get_tags_for_messages(conn, &message_refs).unwrap_or_default();

            Ok(PageResult { threaded, total_count, tags_map })
        })
        .await
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?
    };

    tracing::info!(
        folder = %folder,
        total_count = total_count,
        page_messages = threaded.len(),
        page = query.page,
        per_page = query.per_page,
        "list_messages: returning results"
    );

    let cipher = cipher_for(&session);
    let summaries: Vec<MessageSummary> = threaded
        .into_iter()
        .map(|t| {
            let msg_tags = tags_map
                .get(&(t.msg.uid, t.msg.folder.clone()))
                .cloned()
                .unwrap_or_default();
            MessageSummary {
                uid: t.msg.uid,
                folder_id: cipher.encrypt(&t.msg.folder),
                folder_name: t.msg.folder.clone(),
                subject: t.msg.subject,
                from_address: t.msg.from_address,
                from_name: t.msg.from_name,
                to_addresses: t.msg.to_addresses,
                date: t.msg.date,
                flags: t.msg.flags,
                size: t.msg.size,
                has_attachments: t.msg.has_attachments,
                snippet: t.msg.snippet,
                reaction: t.msg.reaction,
                tags: msg_tags,
                thread_count: t.thread_count,
                unread_count: t.unread_count,
            }
        })
        .collect();

    Ok(Json(ListMessagesResponse {
        messages: summaries,
        total_count,
        page: query.page,
        per_page: query.per_page,
        syncing,
    })
    .into_response())
}

// ---------------------------------------------------------------------------
// Helpers for eager loading
// ---------------------------------------------------------------------------

/// Upsert fetched headers into the database and index them for search.
fn upsert_and_index_headers(
    conn: &rusqlite::Connection,
    folder: &str,
    headers: &[crate::imap::types::ImapMessageHeader],
    search_engine: &Arc<SearchEngine>,
    user_hash: &str,
) -> Result<(), String> {
    for header in headers {
        let from_address = header.from.first().map(|a| a.address.as_str()).unwrap_or("");
        let from_name = header.from.first().and_then(|a| a.name.as_deref()).unwrap_or("");
        let to_json = serde_json::to_string(&header.to).unwrap_or_else(|_| "[]".to_string());
        let cc_json = serde_json::to_string(&header.cc).unwrap_or_else(|_| "[]".to_string());
        let subject = header.subject.as_deref().unwrap_or("");
        let date = header.date.as_deref().unwrap_or("");
        let flags_csv = header.flags.join(",");

        db::messages::upsert_message(
            conn, folder, header.uid,
            header.message_id.as_deref(), header.in_reply_to.as_deref(),
            header.references.as_deref(), subject, from_address, from_name,
            &to_json, &cc_json, date, header.date_epoch, &flags_csv, header.size,
            header.has_attachments, "", header.reaction.as_deref(),
        )
        .map_err(|e| format!("Database error: {e}"))?;

        // Populate denormalized known_addresses table.
        db::contacts::populate_known_addresses(conn, from_address, from_name, &to_json, &cc_json)
            .map_err(|e| format!("Known addresses error: {e}"))?;
    }

    // Index for search (skip Spam/Junk/Trash).
    if !headers.is_empty()
        && !UserIndex::is_excluded_folder(folder)
        && let Ok(user_index) = search_engine.open_user_index(user_hash)
    {
        let indexable: Vec<IndexableMessage> = headers
            .iter()
            .map(|h| {
                let from_address = h.from.first().map(|a| a.address.as_str()).unwrap_or("");
                let from_name = h.from.first().and_then(|a| a.name.as_deref()).unwrap_or("");
                let subject = h.subject.as_deref().unwrap_or("");
                let to_json = serde_json::to_string(&h.to).unwrap_or_else(|_| "[]".to_string());
                IndexableMessage {
                    uid: h.uid,
                    folder: folder.to_string(),
                    subject: subject.to_string(),
                    from_address: from_address.to_string(),
                    from_name: from_name.to_string(),
                    to_addresses: to_json,
                    body_text: String::new(),
                    date_epoch: h.date_epoch,
                    has_attachments: h.has_attachments,
                }
            })
            .collect();
        let _ = user_index.index_messages_batch(&indexable);
    }

    Ok(())
}

/// Parameters for a background sync task, bundled to avoid too-many-arguments.
struct BackgroundSyncParams {
    creds: ImapCredentials,
    folder: String,
    imap_client: Arc<dyn ImapClient>,
    db_pool_manager: Arc<db::pool::DbPoolManager>,
    user_hash: String,
    search_engine: Arc<SearchEngine>,
    event_bus: Arc<EventBus>,
    uid_range: String,
    uid_validity: u32,
    exists: u32,
}

/// Background task: fetch remaining headers for a folder and notify the
/// frontend via EventBus when done. All errors are logged but non-fatal.
async fn sync_remaining_headers(params: BackgroundSyncParams) {
    let BackgroundSyncParams {
        creds, folder, imap_client, db_pool_manager, user_hash,
        search_engine, event_bus, uid_range, uid_validity, exists,
    } = params;
    let result = sync_remaining_headers_inner(
        creds, folder.clone(), imap_client, db_pool_manager, user_hash.clone(),
        search_engine, uid_range, uid_validity, exists,
    )
    .await;

    match result {
        Ok(()) => {
            tracing::info!(folder = %folder, "Background sync: complete");
            event_bus.publish(&user_hash, MailEvent::FolderUpdated { folder: Some(folder.clone()) }).await;
        }
        Err(e) => {
            tracing::warn!(folder = %folder, error = %e, "Background sync: failed (will retry on next request)");
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn sync_remaining_headers_inner(
    creds: ImapCredentials,
    folder: String,
    imap_client: Arc<dyn ImapClient>,
    db_pool_manager: Arc<db::pool::DbPoolManager>,
    user_hash: String,
    search_engine: Arc<SearchEngine>,
    uid_range: String,
    uid_validity: u32,
    exists: u32,
) -> Result<(), String> {
    tracing::info!(folder = %folder, uid_range = %uid_range, "Background sync: fetching remaining headers");

    let headers = imap_client
        .fetch_headers(&creds, &folder, &uid_range)
        .await
        .map_err(|e| format!("IMAP error: {e}"))?;

    tracing::info!(folder = %folder, fetched = headers.len(), "Background sync: fetched headers");

    if headers.is_empty() {
        return Ok(());
    }

    let pool_user_hash = user_hash.clone();
    db::pool::with_user_db(&db_pool_manager, &pool_user_hash, move |conn| {
            for header in &headers {
                let from_address = header.from.first().map(|a| a.address.as_str()).unwrap_or("");
                let from_name = header.from.first().and_then(|a| a.name.as_deref()).unwrap_or("");
                let to_json = serde_json::to_string(&header.to).unwrap_or_else(|_| "[]".to_string());
                let cc_json = serde_json::to_string(&header.cc).unwrap_or_else(|_| "[]".to_string());
                let subject = header.subject.as_deref().unwrap_or("");
                let date = header.date.as_deref().unwrap_or("");
                let flags_csv = header.flags.join(",");

                db::messages::upsert_message(
                    conn, &folder, header.uid,
                    header.message_id.as_deref(), header.in_reply_to.as_deref(),
                    header.references.as_deref(), subject, from_address, from_name,
                    &to_json, &cc_json, date, header.date_epoch, &flags_csv, header.size,
                    header.has_attachments, "", header.reaction.as_deref(),
                )
                .map_err(|e| format!("Database error: {e}"))?;

                // Populate denormalized known_addresses table.
                db::contacts::populate_known_addresses(conn, from_address, from_name, &to_json, &cc_json)
                    .map_err(|e| format!("Known addresses error: {e}"))?;
            }

            // Index for search.
            if !UserIndex::is_excluded_folder(&folder)
                && let Ok(user_index) = search_engine.open_user_index(&user_hash)
            {
                let indexable: Vec<IndexableMessage> = headers
                    .iter()
                    .map(|h| {
                        let from_address = h.from.first().map(|a| a.address.as_str()).unwrap_or("");
                        let from_name = h.from.first().and_then(|a| a.name.as_deref()).unwrap_or("");
                        let subject = h.subject.as_deref().unwrap_or("");
                        let to_json = serde_json::to_string(&h.to).unwrap_or_else(|_| "[]".to_string());
                        IndexableMessage {
                            uid: h.uid,
                            folder: folder.clone(),
                            subject: subject.to_string(),
                            from_address: from_address.to_string(),
                            from_name: from_name.to_string(),
                            to_addresses: to_json,
                            body_text: String::new(),
                            date_epoch: h.date_epoch,
                            has_attachments: h.has_attachments,
                        }
                    })
                    .collect();
                let _ = user_index.index_messages_batch(&indexable);
            }

            // Update folder status and unread count.
            db::folders::update_folder_status(conn, &folder, uid_validity, exists)
                .map_err(|e| format!("Database error: {e}"))?;
            db::folders::refresh_unread_count(conn, &folder)
                .map_err(|e| format!("Database error: {e}"))?;

            Ok(())
        })
        .await
}

/// Request body for `POST /api/messages/by-message-id`.
#[derive(serde::Deserialize)]
pub(crate) struct ByMessageIdRequest {
    pub(crate) message_id: String,
}

/// Resolved message body, either read from cache or freshly fetched from IMAP.
struct BodyData {
    body_html: Option<String>,
    body_text: Option<String>,
    attachments: Vec<AttachmentMeta>,
    raw_headers: String,
    email_theme: Option<i32>,
    pgp_status: Option<crate::imap::types::PgpMessageStatus>,
}

/// Builds a fallback `CachedMessage` by parsing `raw_headers` directly, used
/// when the message header hasn't been synced into the messages table yet
/// (e.g. DB was cleared and sync is still running).
fn build_synthetic_message(uid: u32, folder: &str, raw_headers: &str, attachments_len: usize) -> db::messages::CachedMessage {
    let parsed = mail_parser::MessageParser::default().parse(raw_headers.as_bytes());
    let subject = parsed.as_ref().and_then(|p| p.subject().map(|s| s.to_string())).unwrap_or_default();
    let (from_address, from_name) = parsed.as_ref()
        .and_then(|p| p.from())
        .and_then(|addr| match addr {
            mail_parser::Address::List(addrs) => addrs.first().map(|a| {
                (
                    a.address.as_ref().map(|s| s.to_string()).unwrap_or_default(),
                    a.name.as_ref().map(|s| s.to_string()).unwrap_or_default(),
                )
            }),
            _ => None,
        })
        .unwrap_or_default();
    let date = parsed.as_ref().and_then(|p| p.date()).map(|d| {
        let sign = if d.tz_before_gmt { '-' } else { '+' };
        format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
            d.year, d.month, d.day, d.hour, d.minute, d.second, sign, d.tz_hour, d.tz_minute)
    }).unwrap_or_default();

    db::messages::CachedMessage {
        uid,
        folder: folder.to_string(),
        message_id: parsed.as_ref().and_then(|p| p.message_id().map(|s| format!("<{s}>"))),
        in_reply_to: parsed.as_ref().and_then(|p| p.in_reply_to().as_text().map(|s| format!("<{s}>"))),
        references_header: parsed.as_ref().and_then(|p| {
            let val = p.references();
            val.as_text_list()
                .map(|list| list.iter().map(|s| format!("<{s}>")).collect::<Vec<_>>().join(" "))
                .or_else(|| val.as_text().map(|s| format!("<{s}>")))
        }),
        subject,
        from_address,
        from_name,
        to_addresses: String::from("[]"),
        cc_addresses: String::from("[]"),
        date_epoch: crate::db::messages::parse_date_epoch(&date),
        date,
        flags: String::new(),
        size: 0,
        has_attachments: attachments_len > 0,
        snippet: String::new(),
        reaction: None,
    }
}

/// `POST /api/messages/by-message-id`
///
/// Looks up a message by its Message-ID header in the local cache, then fetches
/// and returns the full message detail. Used to reconstruct reply quote context
/// when reopening a draft that has `in_reply_to` set.
/// The message_id is in the POST body to keep it out of server access logs.
pub async fn get_message_by_message_id(
    session: Extension<SessionState>,
    config: Extension<Arc<AppConfig>>,
    imap_client: Extension<Arc<dyn ImapClient>>,
    search_engine: Extension<Arc<SearchEngine>>,
    db_pool_manager: Extension<Arc<db::pool::DbPoolManager>>,
    link_proxy: Option<Extension<Arc<crate::link_proxy::LinkProxySecret>>>,
    Json(body): Json<ByMessageIdRequest>,
) -> Result<Response, AppError> {
    let message_id = body.message_id.clone();
    let (folder, uid) = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::messages::find_by_message_id(conn, &message_id)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?
    .ok_or_else(|| AppError::NotFound("Message not found".to_string()))?;

    // Encrypt the folder so get_message can decrypt it cleanly through the normal path.
    let encrypted_folder = cipher_for(&session).encrypt(&folder);
    get_message(session, config, imap_client, search_engine, db_pool_manager, link_proxy, Path((encrypted_folder, uid))).await
}

/// `GET /api/messages/:folder/:uid`
///
/// Returns the full message detail including body and attachment metadata.
/// Fetches from IMAP and caches if not already cached.
pub async fn get_message(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(search_engine): Extension<Arc<SearchEngine>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    link_proxy: Option<Extension<Arc<crate::link_proxy::LinkProxySecret>>>,
    Path((folder_id, uid)): Path<(FolderId, u32)>,
) -> Result<Response, AppError> {
    let folder = cipher_for(&session).decrypt(&folder_id)?;
    let creds = build_creds(&session, &config)?;

    // Check SQLite cache first. If it's usable, resolve fully here (sync-only,
    // no IMAP round-trip needed).
    enum CachedBodyOutcome {
        Ready(BodyData),
        NeedsFetch,
    }

    let outcome = {
        let folder = folder.clone();
        db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
            let cached_body = db::messages::get_cached_body(conn, &folder, uid)?;

            // Treat a cache hit with missing attachments_json as stale (pre-V006
            // cache). Re-fetch from IMAP so attachments and inline images are
            // properly resolved.
            let usable_cache = cached_body.filter(|c| c.attachments_json.is_some());

            let Some(cached) = usable_cache else {
                return Ok(CachedBodyOutcome::NeedsFetch);
            };

            let attachments: Vec<AttachmentMeta> = cached
                .attachments_json
                .as_deref()
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or_default();
            let theme = cached.email_theme;
            let (email_theme, html, text) = if theme.is_none() && let Some(ref html) = cached.html {
                let detected = email_theme::detect_email_theme(html).map(|t| t.as_i32());
                if let Some(t) = detected {
                    let _ = db::messages::update_email_theme(conn, &folder, uid, t);
                }
                (detected, cached.html, cached.text)
            } else {
                (theme, cached.html, cached.text)
            };

            Ok(CachedBodyOutcome::Ready(BodyData {
                body_html: html,
                body_text: text,
                attachments,
                raw_headers: cached.raw_headers.unwrap_or_default(),
                email_theme,
                pgp_status: None,
            }))
        })
        .await
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?
    };

    let BodyData { body_html, body_text, attachments, raw_headers, email_theme, pgp_status } = match outcome {
        CachedBodyOutcome::Ready(data) => data,
        CachedBodyOutcome::NeedsFetch => {
            // Fetch from IMAP.
            let body = imap_client
                .fetch_body(&creds, &folder, uid)
                .await
                .map_err(|e| match e {
                    crate::imap::client::ImapError::MessageNotFound { .. } => AppError::NotFound(format!("Message UID {uid} not found in folder {folder}")),
                    other => AppError::ServiceUnavailable(format!("IMAP error: {other}")),
                });

            let body = match body {
                Ok(b) => b,
                Err(app_err) => {
                    // Message exists in local DB but IMAP couldn't serve it
                    // (deleted server-side, or IMAP unreachable) — remove the
                    // stale cache entry, search index entry, and invalidate
                    // the folder so the list refreshes.
                    let folder2 = folder.clone();
                    let _ = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
                        let _ = db::messages::delete_message(conn, &folder2, uid);
                        let _ = db::folders::invalidate_folder_freshness(conn, &folder2);
                        Ok::<(), String>(())
                    })
                    .await;
                    if let Ok(user_index) = search_engine.open_user_index(&session.user_hash) {
                        let _ = user_index.delete_message(uid, &folder);
                    }
                    return Err(app_err);
                }
            };

            // Use HTML directly (frontend sandbox handles security).
            let sanitized_html = body.text_html.clone();

            let attachment_meta: Vec<AttachmentMeta> = body
                .attachments
                .iter()
                .enumerate()
                .map(|(i, a)| AttachmentMeta {
                    id: i.to_string(),
                    filename: a.filename.clone(),
                    content_type: a.content_type.clone(),
                    size: a.size,
                    content_id: a.content_id.clone(),
                })
                .collect();

            // Rewrite cid: URLs in the HTML to inline data URIs so the
            // sandboxed iframe can display embedded images without needing
            // network access.
            let resolved_html = sanitized_html.map(|mut html| {
                for att in &body.attachments {
                    if let Some(ref cid) = att.content_id {
                        let cid_url = format!("cid:{cid}");
                        if html.contains(&cid_url) {
                            use base64::Engine;
                            let b64 = base64::engine::general_purpose::STANDARD.encode(&att.data);
                            let data_uri = format!("data:{};base64,{}", att.content_type, b64);
                            html = html.replace(&cid_url, &data_uri);
                        }
                    }
                }
                html
            });

            // Serialize attachment metadata for caching.
            let att_json = serde_json::to_string(&attachment_meta).ok();

            let detected_theme = resolved_html
                .as_ref()
                .and_then(|h| email_theme::detect_email_theme(h))
                .map(|t| t.as_i32());

            {
                let folder = folder.clone();
                let resolved_html = resolved_html.clone();
                let text_plain = body.text_plain.clone();
                let raw_headers = body.raw_headers.clone();
                db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
                    db::messages::cache_message_body(
                        conn,
                        &folder,
                        uid,
                        resolved_html.as_deref(),
                        text_plain.as_deref(),
                        att_json.as_deref(),
                        Some(&raw_headers),
                        detected_theme,
                    )
                })
                .await
                .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
            }

            BodyData {
                body_html: resolved_html,
                body_text: body.text_plain,
                attachments: attachment_meta,
                raw_headers: body.raw_headers,
                email_theme: detected_theme,
                pgp_status: body.pgp_status,
            }
        }
    };

    // Get the message header from cache (use efficient single-message lookup).
    // If the header hasn't been synced yet (e.g. DB was cleared and sync is
    // still running), fall back to parsing the raw headers we already fetched.
    let (msg, thread_messages) = {
        let folder = folder.clone();
        let raw_headers_for_fallback = raw_headers.clone();
        let attachments_len = attachments.len();
        db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
            let msg = db::messages::get_single_message(conn, &folder, uid)?;
            let msg = match msg {
                Some(m) => m,
                None => build_synthetic_message(uid, &folder, &raw_headers_for_fallback, attachments_len),
            };
            let thread_messages = if let Some(ref message_id) = msg.message_id {
                db::messages::get_full_thread(conn, message_id, msg.references_header.as_deref()).unwrap_or_default()
            } else {
                vec![]
            };
            Ok((msg, thread_messages))
        })
        .await
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?
    };

    // Re-index message with full body text for search.
    // Skip indexing for Spam/Junk/Trash folders.
    if let Some(ref text) = body_text
        && !UserIndex::is_excluded_folder(&folder)
        && let Ok(user_index) = search_engine.open_user_index(&session.user_hash)
    {
        let indexable = IndexableMessage {
            uid: msg.uid,
            folder: msg.folder.clone(),
            subject: msg.subject.clone(),
            from_address: msg.from_address.clone(),
            from_name: msg.from_name.clone(),
            to_addresses: msg.to_addresses.clone(),
            body_text: text.clone(),
            date_epoch: msg.date_epoch,
            has_attachments: msg.has_attachments,
        };
        let _ = user_index.index_message(&indexable);
    }

    let resp_cipher = cipher_for(&session);
    let thread: Vec<ThreadMessage> = thread_messages
        .into_iter()
        .map(|m| ThreadMessage {
            uid: m.uid,
            folder_id: resp_cipher.encrypt(&m.folder),
            folder_name: m.folder.clone(),
            message_id: m.message_id,
            in_reply_to: m.in_reply_to,
            subject: m.subject,
            from_address: m.from_address,
            from_name: m.from_name,
            to_addresses: m.to_addresses,
            cc_addresses: m.cc_addresses,
            date: m.date,
            flags: m.flags,
            size: m.size,
            has_attachments: m.has_attachments,
            snippet: m.snippet,
        })
        .collect();

    // If link proxy is enabled, rewrite all http(s) hrefs in the HTML body.
    // We do this at response time (not cache time) so toggling the feature
    // doesn't leave stale proxy or non-proxy links in the cache.
    // Skip rewriting for PGP-encrypted messages: there is no HTML to rewrite,
    // and the decrypted content must never pass through the proxy.
    use crate::imap::types::PgpStatusKind;
    let is_encrypted = pgp_status
        .as_ref()
        .is_some_and(|s| s.kind == PgpStatusKind::Encrypted);
    const LINK_PROXY_MAX_CHARS: usize = 200_000;
    let body_html = match (body_html, link_proxy) {
        (Some(html), Some(Extension(secret))) if !is_encrypted => {
            let base = config.base_path.as_deref().unwrap_or("");
            let slice = if html.len() > LINK_PROXY_MAX_CHARS {
                &html[..html.floor_char_boundary(LINK_PROXY_MAX_CHARS)]
            } else {
                &html
            };
            Some(crate::link_proxy::rewrite_html_links(slice, &secret.0, &session.user_hash, base))
        }
        (html, _) => html,
    };

    Ok(Json(MessageDetailResponse {
        uid: msg.uid,
        folder_id: resp_cipher.encrypt(&msg.folder),
        folder_name: msg.folder.clone(),
        subject: msg.subject,
        from_address: msg.from_address,
        from_name: msg.from_name,
        to_addresses: parse_address_list(&msg.to_addresses),
        cc_addresses: parse_address_list(&msg.cc_addresses),
        date: msg.date,
        flags: parse_flags(&msg.flags),
        has_attachments: msg.has_attachments,
        html: body_html,
        text: body_text,
        raw_headers,
        attachments,
        thread,
        email_theme: email_theme.map(|t| match t {
            0 => EmailTheme::Light,
            1 => EmailTheme::Dark,
            2 => EmailTheme::Transparent,
            3 => EmailTheme::Adaptive,
            _ => EmailTheme::Light,
        }),
        pgp_status,
    })
    .into_response())
}

/// `PATCH /api/messages/:folder/:uid/flags`
///
/// Updates message flags on the IMAP server and in the SQLite cache.
pub async fn update_flags(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path((folder_id, uid)): Path<(FolderId, u32)>,
    Json(body): Json<UpdateFlagsRequest>,
) -> Result<Response, AppError> {
    let folder = cipher_for(&session).decrypt(&folder_id)?;
    validate_flags(&body.flags)?;

    let creds = build_creds(&session, &config)?;

    // Convert flags to &str slices for the IMAP client.
    let flag_refs: Vec<&str> = body.flags.iter().map(|s| s.as_str()).collect();

    if body.add {
        imap_client
            .add_flags(&creds, &folder, uid, &flag_refs)
            .await
            .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;
    } else {
        imap_client
            .remove_flags(&creds, &folder, uid, &flag_refs)
            .await
            .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;
    }

    // Update SQLite cache: read current flags, add/remove, write back.
    db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        let current_flags_csv: String = conn
            .query_row(
                "SELECT flags FROM messages WHERE folder = ?1 AND uid = ?2",
                rusqlite::params![&folder, uid],
                |row| row.get(0),
            )
            .unwrap_or_default();

        let mut current_flags: Vec<String> = if current_flags_csv.is_empty() {
            vec![]
        } else {
            current_flags_csv.split(',').map(|s| s.to_string()).collect()
        };

        if body.add {
            for flag in &body.flags {
                if !current_flags.contains(flag) {
                    current_flags.push(flag.clone());
                }
            }
        } else {
            current_flags.retain(|f| !body.flags.contains(f));
        }

        let new_flags_csv = current_flags.join(",");
        db::messages::update_message_flags(conn, &folder, uid, &new_flags_csv)?;

        // Refresh unread count after flag change.
        db::folders::refresh_unread_count(conn, &folder)?;

        Ok(())
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(serde_json::json!({ "status": "ok" })).into_response())
}

/// `PATCH /api/messages/:folder/flags/bulk`
///
/// Updates flags on multiple messages in one IMAP command and one DB
/// transaction, instead of one request per message.
pub async fn bulk_update_flags(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(folder_id): Path<FolderId>,
    Json(body): Json<BulkUpdateFlagsRequest>,
) -> Result<Response, AppError> {
    let folder = cipher_for(&session).decrypt(&folder_id)?;
    validate_flags(&body.flags)?;

    if body.uids.is_empty() {
        return Ok(Json(BulkMessageOpResponse { failed_uids: vec![] }).into_response());
    }

    let creds = build_creds(&session, &config)?;
    let flag_refs: Vec<&str> = body.flags.iter().map(|s| s.as_str()).collect();

    // IMAP UID commands silently skip UIDs that don't exist in the mailbox,
    // so it's safe to send the full (possibly-bogus) UID set in one command.
    if body.add {
        imap_client
            .add_flags_bulk(&creds, &folder, &body.uids, &flag_refs)
            .await
            .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;
    } else {
        imap_client
            .remove_flags_bulk(&creds, &folder, &body.uids, &flag_refs)
            .await
            .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;
    }

    let uids = body.uids.clone();
    let add = body.add;
    let flags = body.flags.clone();
    let failed_uids = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("Transaction error: {e}"))?;

        let found = db::messages::filter_existing_uids(&tx, &folder, &uids)?;
        let mut failed_uids: Vec<u32> = uids.iter().copied().filter(|u| !found.contains(u)).collect();
        failed_uids.sort_unstable();

        for uid in uids.iter().filter(|u| found.contains(u)) {
            let current_flags_csv: String = tx
                .query_row(
                    "SELECT flags FROM messages WHERE folder = ?1 AND uid = ?2",
                    rusqlite::params![&folder, uid],
                    |row| row.get(0),
                )
                .unwrap_or_default();

            let mut current_flags: Vec<String> = if current_flags_csv.is_empty() {
                vec![]
            } else {
                current_flags_csv.split(',').map(|s| s.to_string()).collect()
            };

            if add {
                for flag in &flags {
                    if !current_flags.contains(flag) {
                        current_flags.push(flag.clone());
                    }
                }
            } else {
                current_flags.retain(|f| !flags.contains(f));
            }

            let new_flags_csv = current_flags.join(",");
            db::messages::update_message_flags(&tx, &folder, *uid, &new_flags_csv)?;
        }

        db::folders::refresh_unread_count(&tx, &folder)?;

        tx.commit()
            .map_err(|e| format!("Transaction commit error: {e}"))?;
        Ok(failed_uids)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(BulkMessageOpResponse { failed_uids }).into_response())
}

/// `POST /api/messages/move`
///
/// Moves a message from one folder to another on the IMAP server and
/// removes it from the source folder in SQLite cache.
pub async fn move_message_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(body): Json<MoveMessageRequest>,
) -> Result<Response, AppError> {
    let cipher = cipher_for(&session);
    let from_folder = cipher.decrypt(&body.from_folder)?;
    let to_folder = cipher.decrypt(&body.to_folder)?;
    let creds = build_creds(&session, &config)?;

    // Move on IMAP server.
    imap_client
        .move_message(&creds, &from_folder, body.uid, &to_folder)
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;

    db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        // Check if the message was unread before removing it from the source
        // cache, so we can adjust the destination folder's unread count.
        let was_unread = db::messages::get_single_message(conn, &from_folder, body.uid)?
            .map(|m| !m.flags.contains("\\Seen"))
            .unwrap_or(false);

        // Delete from source folder cache. We don't keep the row in the
        // destination because the UID changes after an IMAP MOVE, and a
        // stale UID would cause 404s when trying to fetch the message body.
        db::messages::delete_message(conn, &from_folder, body.uid)?;

        // Refresh source folder unread count (now accurate since the row is gone).
        db::folders::refresh_unread_count(conn, &from_folder)?;

        // Bump destination folder unread count if the moved message was unread.
        if was_unread {
            db::folders::adjust_unread_count(conn, &to_folder, 1)?;
        }

        // Invalidate destination folder cache so the next list request forces
        // an IMAP resync and picks up the moved message with its new UID.
        db::folders::invalidate_folder_freshness(conn, &to_folder)?;

        Ok(())
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(serde_json::json!({ "status": "ok" })).into_response())
}

/// `POST /api/messages/move/bulk`
///
/// Moves multiple messages from one folder to another in one IMAP command
/// and one DB transaction, instead of one request per message.
pub async fn bulk_move_messages(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(body): Json<BulkMoveMessagesRequest>,
) -> Result<Response, AppError> {
    let cipher = cipher_for(&session);
    let from_folder = cipher.decrypt(&body.from_folder)?;
    let to_folder = cipher.decrypt(&body.to_folder)?;

    if body.uids.is_empty() {
        return Ok(Json(BulkMessageOpResponse { failed_uids: vec![] }).into_response());
    }

    let creds = build_creds(&session, &config)?;

    // Move on IMAP server. IMAP UID commands silently skip UIDs that don't
    // exist in the mailbox, so it's safe to send the full (possibly-bogus)
    // UID set in one command.
    imap_client
        .move_message_bulk(&creds, &from_folder, &body.uids, &to_folder)
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;

    let uids = body.uids.clone();
    let failed_uids = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("Transaction error: {e}"))?;

        let found = db::messages::filter_existing_uids(&tx, &from_folder, &uids)?;
        let mut failed_uids: Vec<u32> = uids.iter().copied().filter(|u| !found.contains(u)).collect();
        failed_uids.sort_unstable();

        let mut unread_moved = 0i32;
        for uid in uids.iter().filter(|u| found.contains(u)) {
            let was_unread = db::messages::get_single_message(&tx, &from_folder, *uid)?
                .map(|m| !m.flags.contains("\\Seen"))
                .unwrap_or(false);
            if was_unread {
                unread_moved += 1;
            }

            // Delete from source folder cache. We don't keep the row in the
            // destination because the UID changes after an IMAP MOVE, and a
            // stale UID would cause 404s when trying to fetch the message body.
            db::messages::delete_message(&tx, &from_folder, *uid)?;
        }

        // Refresh source folder unread count (now accurate since the rows are gone).
        db::folders::refresh_unread_count(&tx, &from_folder)?;

        // Bump destination folder unread count by however many moved messages were unread.
        if unread_moved > 0 {
            db::folders::adjust_unread_count(&tx, &to_folder, unread_moved)?;
        }

        // Invalidate destination folder cache so the next list request forces
        // an IMAP resync and picks up the moved messages with their new UIDs.
        db::folders::invalidate_folder_freshness(&tx, &to_folder)?;

        tx.commit()
            .map_err(|e| format!("Transaction commit error: {e}"))?;
        Ok(failed_uids)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(BulkMessageOpResponse { failed_uids }).into_response())
}

/// `GET /api/messages/:folder/:uid/attachments/:attachment_id`
///
/// Downloads an attachment by its index from the message.
pub async fn download_attachment(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Path((folder_id, uid, attachment_id)): Path<(FolderId, u32, String)>,
) -> Result<Response, AppError> {
    let folder = cipher_for(&session).decrypt(&folder_id)?;
    let creds = build_creds(&session, &config)?;

    // Parse the attachment index.
    let index: usize = attachment_id
        .parse()
        .map_err(|_| AppError::BadRequest(format!("Invalid attachment id: {attachment_id}")))?;

    // Fetch the full message body from IMAP.
    let body = imap_client
        .fetch_body(&creds, &folder, uid)
        .await
        .map_err(|e| match e {
            crate::imap::client::ImapError::MessageNotFound { .. } => {
                AppError::NotFound(format!("Message UID {uid} not found in folder {folder}"))
            }
            other => AppError::ServiceUnavailable(format!("IMAP error: {other}")),
        })?;

    // Find the attachment by index.
    let attachment = body
        .attachments
        .into_iter()
        .nth(index)
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Attachment {attachment_id} not found on message UID {uid}"
            ))
        })?;

    // Build the response with appropriate headers.
    let raw_filename = attachment
        .filename
        .unwrap_or_else(|| format!("attachment_{index}"));
    // Strip ASCII control characters (U+0000-U+001F and U+007F). MIME
    // decoders can surface them via quoted-pair escapes in malformed emails,
    // and HeaderValue rejects those bytes. HeaderValue::from_str routes
    // through is_valid (accepts 0x09, 0x20-0x7E, 0x80-0xFF; rejects
    // 0x00-0x08, 0x0A-0x1F, 0x7F), so non-ASCII UTF-8 bytes are fine.
    let filename: String = raw_filename
        .chars()
        .filter(|c| !c.is_ascii_control())
        .collect();
    // Fall back to a safe default if stripping left nothing (e.g. the
    // original filename was composed entirely of control characters).
    let filename = if filename.is_empty() {
        format!("attachment_{index}")
    } else {
        filename
    };
    let content_type = attachment.content_type;

    // Use inline disposition for types the browser can display natively
    // (PDF, images) so the preview works; use attachment for everything else.
    let is_inline = content_type == "application/pdf"
        || content_type.starts_with("image/")
        || content_type.starts_with("text/");
    let disposition = if is_inline {
        format!("inline; filename=\"{}\"", filename.replace('"', "\\\""))
    } else {
        format!("attachment; filename=\"{}\"", filename.replace('"', "\\\""))
    };

    Response::builder()
        .header("content-type", &content_type)
        .header("content-disposition", &disposition)
        .body(axum::body::Body::from(attachment.data))
        .map_err(|e| AppError::InternalError(format!("Failed to build response: {e}")))
}

/// `DELETE /api/messages/:folder/:uid`
///
/// Permanently removes a message from the IMAP server and SQLite cache.
pub async fn delete_message_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path((folder_id, uid)): Path<(FolderId, u32)>,
) -> Result<Response, AppError> {
    let folder = cipher_for(&session).decrypt(&folder_id)?;
    let creds = build_creds(&session, &config)?;

    // Expunge on IMAP server.
    imap_client
        .expunge_message(&creds, &folder, uid)
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;

    // Delete from SQLite cache.
    db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::messages::delete_message(conn, &folder, uid)?;
        // Refresh unread count for folder.
        db::folders::refresh_unread_count(conn, &folder)?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(serde_json::json!({ "status": "ok" })).into_response())
}

/// `POST /api/messages/:folder/delete/bulk`
///
/// Permanently removes multiple messages from the IMAP server and SQLite
/// cache in one IMAP command and one DB transaction, instead of one request
/// per message.
pub async fn bulk_delete_messages(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(folder_id): Path<FolderId>,
    Json(body): Json<BulkDeleteMessagesRequest>,
) -> Result<Response, AppError> {
    let folder = cipher_for(&session).decrypt(&folder_id)?;

    if body.uids.is_empty() {
        return Ok(Json(BulkMessageOpResponse { failed_uids: vec![] }).into_response());
    }

    let creds = build_creds(&session, &config)?;

    // Expunge on IMAP server. IMAP UID commands silently skip UIDs that
    // don't exist in the mailbox, so it's safe to send the full
    // (possibly-bogus) UID set in one command.
    imap_client
        .expunge_message_bulk(&creds, &folder, &body.uids)
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;

    let uids = body.uids.clone();
    let failed_uids = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("Transaction error: {e}"))?;

        let found = db::messages::filter_existing_uids(&tx, &folder, &uids)?;
        let mut failed_uids: Vec<u32> = uids.iter().copied().filter(|u| !found.contains(u)).collect();
        failed_uids.sort_unstable();

        for uid in uids.iter().filter(|u| found.contains(u)) {
            db::messages::delete_message(&tx, &folder, *uid)?;
        }
        db::folders::refresh_unread_count(&tx, &folder)?;

        tx.commit()
            .map_err(|e| format!("Transaction commit error: {e}"))?;
        Ok(failed_uids)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(BulkMessageOpResponse { failed_uids }).into_response())
}

/// `POST /api/folders/{folder}/mark-all-read`
///
/// Marks every message in the folder as read via IMAP UID STORE 1:* +FLAGS (\Seen),
/// then updates the SQLite cache and refreshes the unread count.
pub async fn mark_all_read(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(folder_id): Path<FolderId>,
) -> Result<Response, AppError> {
    let folder = cipher_for(&session).decrypt(&folder_id)?;
    let creds = build_creds(&session, &config)?;

    imap_client
        .mark_all_read(&creds, &folder)
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;

    db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        conn.execute(
            "UPDATE messages SET flags = CASE WHEN flags = '' THEN '\\Seen' ELSE flags || ',\\Seen' END WHERE folder = ?1 AND flags NOT LIKE '%\\Seen%'",
            rusqlite::params![&folder],
        ).map_err(|e| format!("Database error: {e}"))?;

        db::folders::refresh_unread_count(conn, &folder)?;

        Ok(())
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(serde_json::json!({ "status": "ok" })).into_response())
}
