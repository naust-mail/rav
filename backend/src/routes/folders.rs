use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Serialize;

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::db;
use crate::error::AppError;
use crate::folder_cipher::FolderId;
use crate::imap::client::{ImapClient, ImapCredentials};
use crate::routes::messages::types::MessageSummary;

/// How many recent messages to embed per folder in the list response.
const FOLDER_PREVIEW_LIMIT: u32 = 20;

/// Response envelope for `GET /api/folders`.
#[derive(Serialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
struct FoldersResponse {
    folders: Vec<FolderEntry>,
}

/// A single folder in the response, including a preview of its most recent messages.
#[derive(Serialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
struct FolderEntry {
    /// Opaque encrypted folder ID for use in subsequent API requests.
    id: FolderId,
    name: String,
    delimiter: Option<String>,
    attributes: Vec<String>,
    is_subscribed: bool,
    total_count: u32,
    unread_count: u32,
    /// Top N most recent messages, ready to seed the client cache.
    recent_messages: Vec<MessageSummary>,
}

/// How many seconds the folder list cache is considered fresh.
const FOLDER_LIST_TTL_SECS: u32 = 30;

/// `GET /api/folders`
///
/// Lists all IMAP folders for the authenticated user, syncing the result
/// into the per-user SQLite cache.  If the cache was refreshed within
/// `FOLDER_LIST_TTL_SECS` seconds, IMAP is skipped entirely.
pub async fn list_folders(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
) -> Result<Response, AppError> {
    // If the folder cache was updated recently, skip the IMAP round-trip.
    let cache_fresh = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        db::folders::is_folder_cache_fresh(conn, FOLDER_LIST_TTL_SECS)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    if !cache_fresh {
        let imap_host = config
            .imap_host
            .as_deref()
            .ok_or_else(|| AppError::ServiceUnavailable("Mail server not configured".to_string()))?;

        let creds = ImapCredentials {
            host: imap_host.to_string(),
            port: config.imap_port,
            tls: config.tls_enabled,
            email: session.email.clone(),
            password: session.password.clone(),
        };

        // Fetch folders from IMAP server.
        let imap_folders = imap_client
            .list_folders(&creds)
            .await
            .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;

        db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
            // Sync each folder into SQLite cache.
            // Use INSERT OR IGNORE to create new folders without triggering CASCADE on
            // existing ones, then UPDATE the metadata fields separately.
            for folder in &imap_folders {
                let flags_csv = folder.attributes.join(",");
                db::folders::insert_folder_if_new(
                    conn,
                    &folder.name,
                    folder.delimiter.as_deref(),
                    &flags_csv,
                )?;
            }

            // Remove stale folders that no longer exist on the server.
            let current_names: Vec<String> = imap_folders.iter().map(|f| f.name.clone()).collect();
            db::folders::remove_stale_folders(conn, &current_names)?;

            // Touch updated_at on all folders so the cache TTL resets.
            db::folders::touch_all_folders(conn)?;

            Ok(())
        })
        .await
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    }

    struct FoldersPage {
        cached: Vec<db::folders::CachedFolder>,
        folder_previews: Vec<(String, Vec<db::messages::ThreadedMessage>)>,
        tags_map: std::collections::HashMap<(u32, String), Vec<db::tags::MessageTag>>,
    }

    let FoldersPage { cached, folder_previews, tags_map } = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        // Refresh unread counts from cached messages — but skip folders whose
        // messages cache has been invalidated (messages_updated_at IS NULL).
        // Those folders have a manually adjusted unread_count (via adjust_unread_count)
        // that should be preserved until the folder's messages are resynced from IMAP.
        let all_folders = db::folders::get_all_folders(conn)?;
        for f in &all_folders {
            let invalidated = db::folders::is_folder_messages_invalidated(conn, &f.name)?;
            if !invalidated {
                db::folders::refresh_unread_count(conn, &f.name)?;
            }
        }

        // Read back from cache to get the refreshed counts.
        let cached = db::folders::get_all_folders(conn)?;

        // Fetch threaded message previews for every folder, then batch-fetch their tags.
        let mut folder_previews: Vec<(String, Vec<db::messages::ThreadedMessage>)> = Vec::new();
        for f in &cached {
            let previews = db::messages::get_threaded_messages(conn, &f.name, 0, FOLDER_PREVIEW_LIMIT)?;
            folder_previews.push((f.name.clone(), previews));
        }

        // One tags lookup covering all preview messages across all folders.
        let all_refs: Vec<(u32, &str)> = folder_previews
            .iter()
            .flat_map(|(_, msgs)| msgs.iter().map(|t| (t.msg.uid, t.msg.folder.as_str())))
            .collect();
        let tags_map = db::tags::get_tags_for_messages(conn, &all_refs).unwrap_or_default();

        Ok(FoldersPage { cached, folder_previews, tags_map })
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    let cipher = crate::folder_cipher::FolderCipher::new(&session.folder_key);

    let folders: Vec<FolderEntry> = cached
        .into_iter()
        .zip(folder_previews)
        .map(|(f, (_, previews))| {
            let attributes: Vec<String> = if f.flags.is_empty() {
                vec![]
            } else {
                f.flags.split(',').map(|s| s.to_string()).collect()
            };
            let recent_messages: Vec<MessageSummary> = previews
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
            FolderEntry {
                id: cipher.encrypt(&f.name),
                name: f.name,
                delimiter: f.delimiter,
                attributes,
                is_subscribed: f.is_subscribed,
                total_count: f.total_count,
                unread_count: f.unread_count,
                recent_messages,
            }
        })
        .collect();

    Ok(Json(FoldersResponse { folders }).into_response())
}
