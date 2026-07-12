use std::collections::{HashMap, HashSet};

use crate::db;
use crate::email_theme;
use crate::imap::client::{ImapClient, ImapCredentials};
use crate::realtime::events::{EventBus, MailEvent};
use crate::search::engine::{IndexableMessage, SearchEngine};
use crate::search::engine::UserIndex;

/// How often to run a sync check (seconds).
/// STATUS checks are cheap (no SELECT), so 30s is a tight safety net
/// for non-INBOX folders that don't have IDLE.
/// How often `SyncWorkerManager`'s keepalive loop pokes a connected user's
/// worker, reused here so both sides agree on one cadence.
pub(crate) const SYNC_INTERVAL_SECS: u64 = 30;

/// Reconcile all of a user's folders using a 3-tier strategy per folder:
/// 1. STATUS pre-check (cheap, no SELECT)
/// 2. CONDSTORE incremental fetch (only changed flags)
/// 3. Full fetch fallback (when CONDSTORE unavailable)
///
/// Called by `SyncWorkerManager`'s worker loop in response to a wake-up
/// (IDLE new data, the periodic keepalive, or a stale-folder request) —
/// this is the single place that owns writing the per-user cache.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_sync(
    user_hash: &str,
    creds: &ImapCredentials,
    imap_client: &dyn ImapClient,
    event_bus: &EventBus,
    search_engine: &SearchEngine,
    db_pool_manager: &db::pool::DbPoolManager,
) -> Result<(), String> {
    // Collect folder metadata via a pooled connection.
    let folder_snapshots: Vec<FolderSnapshot> = db::pool::with_user_db(db_pool_manager, user_hash, |conn| {
        let cached_folders = db::folders::get_all_folders(conn)?;
        Ok(cached_folders
            .into_iter()
            .map(|f| FolderSnapshot {
                name: f.name,
                uid_validity: f.uid_validity,
                highest_modseq: f.highest_modseq,
            })
            .collect::<Vec<_>>())
    })
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    let mut any_changes = false;

    for folder in &folder_snapshots {
        let folder_name = &folder.name;

        // ── Tier 1: STATUS pre-check (no SELECT needed) ──────────────
        let status = match imap_client.folder_status_extended(creds, folder_name).await {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(
                    folder = %folder_name,
                    error = %e,
                    "Skipping folder sync (STATUS failed)"
                );
                continue;
            }
        };

        // Check UIDVALIDITY — if changed, the folder was rebuilt.
        if folder.uid_validity != 0 && status.uid_validity != folder.uid_validity {
            tracing::info!(
                folder = %folder_name,
                old_validity = folder.uid_validity,
                new_validity = status.uid_validity,
                "UIDVALIDITY changed, invalidating folder"
            );
            let _ = db::pool::with_user_db(db_pool_manager, user_hash, {
                let folder_name = folder_name.clone();
                move |conn| db::folders::invalidate_folder_freshness(conn, &folder_name)
            })
            .await;
            any_changes = true;
            event_bus
                .publish(user_hash, MailEvent::FolderUpdated { folder: Some(folder_name.to_string()) })
                .await;
            continue;
        }

        // If CONDSTORE is supported (highest_modseq > 0) and matches cached → skip.
        let cached_modseq = folder.highest_modseq;
        if status.highest_modseq > 0 && cached_modseq > 0 && status.highest_modseq == cached_modseq {
            let cached_count = db::pool::with_user_db(db_pool_manager, user_hash, {
                let folder_name = folder_name.clone();
                move |conn| Ok(db::messages::count_messages(conn, &folder_name).unwrap_or(0))
            })
            .await
            .map_err(|e| format!("DB error: {e}"))?;
            if status.exists == cached_count {
                tracing::debug!(
                    folder = %folder_name,
                    modseq = status.highest_modseq,
                    "Skipping unchanged folder"
                );
                continue;
            }
        }

        // ── Tier 2: CONDSTORE incremental fetch ──────────────────────
        if status.highest_modseq > 0 && cached_modseq > 0 {
            let folder_changed = sync_condstore(
                user_hash, creds, imap_client, folder_name, cached_modseq, &status, event_bus, search_engine, db_pool_manager,
            ).await?;
            if folder_changed {
                any_changes = true;
            }
            continue;
        }

        // ── Tier 3: Full fetch fallback ──────────────────────────────
        let folder_changed = sync_full(
            user_hash, creds, imap_client, folder_name, &status, event_bus, search_engine, db_pool_manager,
        ).await?;
        if folder_changed {
            any_changes = true;
        }
    }

    if any_changes {
        event_bus.publish(user_hash, MailEvent::FolderUpdated { folder: None }).await;
    }

    // ── Deep index phase: fetch & index bodies if enabled ────────────
    if let Err(e) = index_message_bodies(user_hash, creds, imap_client, search_engine, db_pool_manager).await {
        tracing::debug!(error = %e, "Deep index phase skipped or failed");
    }

    Ok(())
}

/// Batch size for deep indexing per sync cycle.
const DEEP_INDEX_BATCH: u32 = 10;

/// If the `deep_index` preference is enabled, fetch bodies for messages that
/// don't have a cached body yet, cache them, and re-index with body text.
async fn index_message_bodies(
    user_hash: &str,
    creds: &ImapCredentials,
    imap_client: &dyn ImapClient,
    search_engine: &SearchEngine,
    db_pool_manager: &db::pool::DbPoolManager,
) -> Result<(), String> {
    enum PreCheck {
        DeepIndexDisabled,
        Unindexed(Vec<(String, u32)>),
    }

    let pre_check = db::pool::with_user_db(db_pool_manager, user_hash, |conn| {
        // Check if deep_index is enabled.
        let prefs = db::display_preferences::get_preferences(conn)?;
        if !prefs.deep_index {
            return Ok(PreCheck::DeepIndexDisabled);
        }

        Ok(PreCheck::Unindexed(db::messages::get_unindexed_messages(conn, DEEP_INDEX_BATCH)?))
    })
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    let unindexed = match pre_check {
        PreCheck::DeepIndexDisabled => return Ok(()),
        PreCheck::Unindexed(u) => u,
    };

    if unindexed.is_empty() {
        return Ok(());
    }

    tracing::debug!(count = unindexed.len(), "Deep indexing message bodies");

    let user_index = search_engine
        .open_user_index(user_hash)
        .map_err(|e| format!("Search index error: {e}"))?;

    for (folder, uid) in &unindexed {
        if UserIndex::is_excluded_folder(folder) {
            continue;
        }

        let body = match imap_client.fetch_body(creds, folder, *uid).await {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!(folder = %folder, uid = uid, error = %e, "Deep index: failed to fetch body");
                continue;
            }
        };

        // Cache the body in the DB.
        {
            let att_json = serde_json::to_string(&body.attachments).ok();
            let detected_theme = body.text_html
                .as_ref()
                .and_then(|h| email_theme::detect_email_theme(h))
                .map(|t| t.as_i32());

            let folder = folder.clone();
            let text_html = body.text_html.clone();
            let text_plain = body.text_plain.clone();
            let raw_headers = body.raw_headers.clone();
            let uid = *uid;
            db::pool::with_user_db(db_pool_manager, user_hash, move |conn| {
                db::messages::cache_message_body(
                    conn,
                    &folder,
                    uid,
                    text_html.as_deref(),
                    text_plain.as_deref(),
                    att_json.as_deref(),
                    Some(&raw_headers),
                    detected_theme,
                )
            })
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        }

        // Re-index with body text.
        if let Some(ref text) = body.text_plain {
            let msg = db::pool::with_user_db(db_pool_manager, user_hash, {
                let folder = folder.clone();
                let uid = *uid;
                move |conn| db::messages::get_single_message(conn, &folder, uid)
            })
            .await
            .map_err(|e| format!("DB error: {e}"))?;
            if let Some(msg) = msg {
                let indexable = IndexableMessage {
                    uid: msg.uid,
                    folder: msg.folder,
                    subject: msg.subject,
                    from_address: msg.from_address,
                    from_name: msg.from_name,
                    to_addresses: msg.to_addresses,
                    body_text: text.clone(),
                    date_epoch: msg.date_epoch,
                    has_attachments: msg.has_attachments,
                };
                let _ = user_index.index_message(&indexable);
            }
        }
    }

    tracing::debug!(count = unindexed.len(), "Deep index batch complete");
    Ok(())
}

/// Fetch headers for UIDs >= `from_uid`, upsert them into the cache, and
/// index them for search. Returns (new_count, latest_sender, latest_subject)
/// for the highest-UID message fetched, for use in a `FolderStateChanged` event.
async fn fetch_and_store_new(
    user_hash: &str,
    creds: &ImapCredentials,
    imap_client: &dyn ImapClient,
    folder_name: &str,
    from_uid: u32,
    search_engine: &SearchEngine,
    db_pool_manager: &db::pool::DbPoolManager,
) -> (u32, Option<String>, Option<String>) {
    let uid_range = format!("{from_uid}:*");
    let headers = match imap_client.fetch_headers(creds, folder_name, &uid_range).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(folder = %folder_name, error = %e, "Failed to fetch new message headers");
            return (0, None, None);
        }
    };

    // "UID:*" can return the last message in the folder even if it's below
    // from_uid (per RFC 3501 when the range's upper bound doesn't exist).
    let headers: Vec<_> = headers.into_iter().filter(|h| h.uid >= from_uid).collect();
    if headers.is_empty() {
        return (0, None, None);
    }

    let upsert_result = db::pool::with_user_db(db_pool_manager, user_hash, {
        let folder_name = folder_name.to_string();
        let headers = headers.clone();
        move |conn| {
            for header in &headers {
                let from_address = header.from.first().map(|a| a.address.as_str()).unwrap_or("");
                let from_name = header.from.first().and_then(|a| a.name.as_deref()).unwrap_or("");
                let to_json = serde_json::to_string(&header.to).unwrap_or_else(|_| "[]".to_string());
                let cc_json = serde_json::to_string(&header.cc).unwrap_or_else(|_| "[]".to_string());
                let subject = header.subject.as_deref().unwrap_or("");
                let date = header.date.as_deref().unwrap_or("");
                let flags_csv = header.flags.join(",");

                if db::messages::upsert_message(
                    conn, &folder_name, header.uid,
                    header.message_id.as_deref(), header.in_reply_to.as_deref(),
                    header.references.as_deref(), subject, from_address, from_name,
                    &to_json, &cc_json, date, header.date_epoch, &flags_csv, header.size,
                    header.has_attachments, "", header.reaction.as_deref(),
                ).is_err() {
                    continue;
                }

                let _ = db::contacts::populate_known_addresses(conn, from_address, from_name, &to_json, &cc_json);
            }
            Ok(())
        }
    })
    .await;

    if let Err(e) = upsert_result {
        tracing::warn!(error = %e, "Failed to open DB for new message upsert");
        return (0, None, None);
    }

    if !UserIndex::is_excluded_folder(folder_name)
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
                    folder: folder_name.to_string(),
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

    let latest = headers.iter().max_by_key(|h| h.uid).expect("headers non-empty");
    let latest_sender = latest
        .from
        .first()
        .map(|a| a.name.clone().unwrap_or_else(|| a.address.clone()));
    let latest_subject = latest.subject.clone();

    (headers.len() as u32, latest_sender, latest_subject)
}

/// Lightweight snapshot of a cached folder for the sync loop.
struct FolderSnapshot {
    name: String,
    uid_validity: u32,
    highest_modseq: u64,
}

/// CONDSTORE path: fetch only changed flags, detect deletions via count comparison.
#[allow(clippy::too_many_arguments)]
async fn sync_condstore(
    user_hash: &str,
    creds: &ImapCredentials,
    imap_client: &dyn ImapClient,
    folder_name: &str,
    cached_modseq: u64,
    status: &crate::imap::types::FolderStatusExtended,
    event_bus: &EventBus,
    search_engine: &SearchEngine,
    db_pool_manager: &db::pool::DbPoolManager,
) -> Result<bool, String> {
    let mut folder_changed = false;

    // Fetch only messages whose flags changed since our cached modseq.
    let (changed, new_modseq) = match imap_client.fetch_changed_flags(creds, folder_name, cached_modseq).await {
        Ok(result) => result,
        Err(e) => {
            tracing::debug!(
                folder = %folder_name,
                error = %e,
                "CONDSTORE fetch failed, falling back to full sync"
            );
            return sync_full(user_hash, creds, imap_client, folder_name, status, event_bus, search_engine, db_pool_manager).await;
        }
    };

    // Apply changed flags.
    db::pool::with_user_db(db_pool_manager, user_hash, {
        let folder_name = folder_name.to_string();
        let changed = changed.clone();
        move |conn| {
            for (uid, flags) in &changed {
                let mut sorted = flags.clone();
                sorted.sort();
                let flags_csv = sorted.join(",");
                let _ = db::messages::update_message_flags(conn, &folder_name, *uid, &flags_csv);
            }
            Ok(())
        }
    })
    .await
    .map_err(|e| format!("DB error: {e}"))?;
    if !changed.is_empty() {
        folder_changed = true;
    }

    // Detect deletions: if server count < cached count, some messages were removed.
    let cached_count = db::pool::with_user_db(db_pool_manager, user_hash, {
        let folder_name = folder_name.to_string();
        move |conn| Ok(db::messages::count_messages(conn, &folder_name).unwrap_or(0))
    })
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    if status.exists < cached_count {
        // Need to fetch all UIDs to find which ones were deleted.
        if let Ok(imap_state) = imap_client.fetch_uids_and_flags(creds, folder_name).await {
            db::pool::with_user_db(db_pool_manager, user_hash, {
                let folder_name = folder_name.to_string();
                move |conn| {
                    let imap_uids: HashSet<u32> = imap_state.iter().map(|(uid, _)| *uid).collect();
                    let cached = db::messages::get_all_uids_and_flags(conn, &folder_name)
                        .unwrap_or_default();
                    let mut any_deleted = false;
                    for (uid, _) in &cached {
                        if !imap_uids.contains(uid) {
                            let _ = db::messages::delete_message(conn, &folder_name, *uid);
                            any_deleted = true;
                            tracing::debug!(
                                folder = %folder_name,
                                uid = uid,
                                "Removed deleted message from cache (CONDSTORE path)"
                            );
                        }
                    }
                    Ok(any_deleted)
                }
            })
            .await
            .map_err(|e| format!("DB error: {e}"))
            .map(|any_deleted| {
                if any_deleted {
                    folder_changed = true;
                }
            })?;
        }
    }

    // Detect new messages.
    let max_cached_uid = db::pool::with_user_db(db_pool_manager, user_hash, {
        let folder_name = folder_name.to_string();
        move |conn| Ok(db::messages::max_uid(conn, &folder_name).unwrap_or(0))
    })
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    if status.uid_next > max_cached_uid + 1 {
        let (new_count, latest_sender, latest_subject) = fetch_and_store_new(
            user_hash, creds, imap_client, folder_name, max_cached_uid + 1, search_engine, db_pool_manager,
        ).await;

        let _ = db::pool::with_user_db(db_pool_manager, user_hash, {
            let folder_name = folder_name.to_string();
            move |conn| db::folders::refresh_unread_count(conn, &folder_name)
        })
        .await;
        folder_changed = true;
        event_bus
            .publish(
                user_hash,
                MailEvent::FolderStateChanged {
                    folder: folder_name.to_string(),
                    count: new_count,
                    latest_sender,
                    latest_subject,
                },
            )
            .await;
    }

    // Update stored modseq.
    let final_modseq = if new_modseq > 0 { new_modseq } else { status.highest_modseq };
    let _ = db::pool::with_user_db(db_pool_manager, user_hash, {
        let folder_name = folder_name.to_string();
        let uid_validity = status.uid_validity;
        let exists = status.exists;
        move |conn| db::folders::update_folder_sync_status(conn, &folder_name, uid_validity, exists, final_modseq)
    })
    .await;

    if folder_changed {
        event_bus
            .publish(
                user_hash,
                MailEvent::FlagsChanged {
                    folder: folder_name.to_string(),
                },
            )
            .await;
    }

    Ok(folder_changed)
}

/// Full fetch fallback: fetch all UIDs+FLAGS and reconcile (original behavior).
#[allow(clippy::too_many_arguments)]
async fn sync_full(
    user_hash: &str,
    creds: &ImapCredentials,
    imap_client: &dyn ImapClient,
    folder_name: &str,
    status: &crate::imap::types::FolderStatusExtended,
    event_bus: &EventBus,
    search_engine: &SearchEngine,
    db_pool_manager: &db::pool::DbPoolManager,
) -> Result<bool, String> {
    let cached = db::pool::with_user_db(db_pool_manager, user_hash, {
        let folder_name = folder_name.to_string();
        move |conn| db::messages::get_all_uids_and_flags(conn, &folder_name)
    })
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    if cached.is_empty() {
        let _ = db::pool::with_user_db(db_pool_manager, user_hash, {
            let folder_name = folder_name.to_string();
            let uid_validity = status.uid_validity;
            let exists = status.exists;
            let highest_modseq = status.highest_modseq;
            move |conn| db::folders::update_folder_sync_status(conn, &folder_name, uid_validity, exists, highest_modseq)
        })
        .await;
        return Ok(false);
    }

    let max_cached_uid = cached.iter().map(|(uid, _)| *uid).max().unwrap_or(0);

    let imap_state = match imap_client.fetch_uids_and_flags(creds, folder_name).await {
        Ok(state) => state,
        Err(e) => {
            tracing::debug!(
                folder = %folder_name,
                error = %e,
                "Skipping folder sync (full fetch failed)"
            );
            return Ok(false);
        }
    };

    let imap_map: HashMap<u32, String> = imap_state
        .into_iter()
        .map(|(uid, flags)| {
            let mut sorted = flags;
            sorted.sort();
            (uid, sorted.join(","))
        })
        .collect();

    let mut folder_changed = db::pool::with_user_db(db_pool_manager, user_hash, {
        let folder_name = folder_name.to_string();
        let cached = cached.clone();
        let imap_map = imap_map.clone();
        let uid_validity = status.uid_validity;
        let exists = status.exists;
        let highest_modseq = status.highest_modseq;
        move |conn| {
            let mut folder_changed = false;
            for (uid, cached_flags_csv) in &cached {
                match imap_map.get(uid) {
                    None => {
                        let _ = db::messages::delete_message(conn, &folder_name, *uid);
                        folder_changed = true;
                        tracing::debug!(
                            folder = %folder_name,
                            uid = uid,
                            "Removed deleted message from cache"
                        );
                    }
                    Some(imap_flags_csv) => {
                        let mut cached_sorted: Vec<&str> = cached_flags_csv.split(',').collect();
                        cached_sorted.sort();
                        let cached_normalized = cached_sorted.join(",");

                        if cached_normalized != *imap_flags_csv {
                            let _ = db::messages::update_message_flags(
                                conn,
                                &folder_name,
                                *uid,
                                imap_flags_csv,
                            );
                            folder_changed = true;
                        }
                    }
                }
            }

            let _ = db::folders::update_folder_sync_status(
                conn, &folder_name, uid_validity, exists, highest_modseq,
            );
            Ok(folder_changed)
        }
    })
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    // Detect new messages (present on server, absent from cache).
    let has_new = imap_map.keys().any(|uid| *uid > max_cached_uid);
    if has_new {
        let (new_count, latest_sender, latest_subject) = fetch_and_store_new(
            user_hash, creds, imap_client, folder_name, max_cached_uid + 1, search_engine, db_pool_manager,
        ).await;

        if new_count > 0 {
            let _ = db::pool::with_user_db(db_pool_manager, user_hash, {
                let folder_name = folder_name.to_string();
                move |conn| db::folders::refresh_unread_count(conn, &folder_name)
            })
            .await;
            folder_changed = true;
            event_bus
                .publish(
                    user_hash,
                    MailEvent::FolderStateChanged {
                        folder: folder_name.to_string(),
                        count: new_count,
                        latest_sender,
                        latest_subject,
                    },
                )
                .await;
        }
    }

    if folder_changed {
        event_bus
            .publish(
                user_hash,
                MailEvent::FlagsChanged {
                    folder: folder_name.to_string(),
                },
            )
            .await;
    }

    Ok(folder_changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::imap::client::mock::MockImapClient;
    use crate::imap::client::{EmailAddress, ImapMessageHeader};
    use crate::imap::types::FolderStatusExtended;
    use tempfile::TempDir;

    fn test_db_pool_manager(data_dir: &std::path::Path) -> Arc<db::pool::DbPoolManager> {
        Arc::new(db::pool::DbPoolManager::new(
            data_dir.to_str().unwrap().to_string(),
            4,
            std::time::Duration::from_secs(600),
            500,
        ))
    }

    fn test_creds() -> ImapCredentials {
        ImapCredentials {
            host: "imap.example.com".to_string(),
            port: 993,
            tls: true,
            email: "alice@example.com".to_string(),
            password: "hunter2".to_string(),
        }
    }

    fn new_header(uid: u32, subject: &str, sender: &str) -> ImapMessageHeader {
        ImapMessageHeader {
            uid,
            subject: Some(subject.to_string()),
            from: vec![EmailAddress {
                name: Some(sender.to_string()),
                address: format!("{sender}@example.com"),
            }],
            to: vec![],
            date: Some("2024-01-01T10:00:00Z".to_string()),
            date_epoch: uid as i64,
            flags: vec![],
            has_attachments: false,
            size: 1024,
            message_id: None,
            in_reply_to: None,
            references: None,
            cc: vec![],
            reaction: None,
        }
    }

    /// Seed a fresh user DB with an INBOX folder containing one cached
    /// message (uid 1), and return the user_hash used.
    async fn seed_user_with_one_message(data_dir: &std::path::Path, highest_modseq: u64) -> String {
        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        crate::auth::user_data::provision_user_data(data_dir.to_str().unwrap(), &user_hash).unwrap();

        let db_pool_manager = test_db_pool_manager(data_dir);
        db::pool::with_user_db(&db_pool_manager, &user_hash, move |conn| {
            db::folders::upsert_folder(conn, "INBOX", None, None, "", true, 1, 0, 1, highest_modseq)?;
            db::messages::upsert_message(
                conn, "INBOX", 1, None, None, None, "Existing", "carol@example.com", "Carol",
                "[]", "[]", "2024-01-01T00:00:00Z", 1, "", 512, false, "", None,
            )?;
            Ok(())
        })
        .await
        .unwrap();

        user_hash
    }

    /// Full-scan tier (no CONDSTORE support cached): a genuinely new message
    /// on the server (uid 2, absent from cache) must be fetched, stored, and
    /// announced via a real FolderStateChanged event - not silently dropped.
    #[tokio::test]
    async fn sync_full_fetches_and_announces_new_messages() {
        let data_dir = TempDir::new().unwrap();
        let user_hash = seed_user_with_one_message(data_dir.path(), 0).await;
        let db_pool_manager = test_db_pool_manager(data_dir.path());
        let creds = test_creds();
        let event_bus = Arc::new(EventBus::new());
        let search_engine = Arc::new(crate::search::engine::SearchEngine::new(data_dir.path().to_path_buf()));

        let mut rx = event_bus.subscribe(&user_hash).await;

        let mock = MockImapClient::new().with_headers(vec![
            new_header(1, "Existing", "Carol"),
            new_header(2, "Fresh mail", "Dave"),
        ]);
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);

        run_sync(&user_hash, &creds, imap_client.as_ref(), &event_bus, &search_engine, &db_pool_manager)
            .await
            .expect("sync should succeed");

        // The new message must actually be in the cache now, not just flagged stale.
        let msg = db::pool::with_user_db(&db_pool_manager, &user_hash, |conn| {
            db::messages::get_single_message(conn, "INBOX", 2)
        })
        .await
        .unwrap();
        assert!(msg.is_some(), "new message should have been fetched and cached");
        assert_eq!(msg.unwrap().subject, "Fresh mail");

        // A real FolderStateChanged event must have been published, not a count:0 poke.
        let mut saw_real_new_messages = false;
        while let Ok(event) = rx.try_recv() {
            if let MailEvent::FolderStateChanged { count, latest_subject, .. } = event {
                assert!(count > 0, "FolderStateChanged event must carry a real count, not a phantom 0");
                assert_eq!(latest_subject.as_deref(), Some("Fresh mail"));
                saw_real_new_messages = true;
            }
        }
        assert!(saw_real_new_messages, "expected a FolderStateChanged event for the new message");
    }

    /// CONDSTORE tier: uid_next advancing past the cached max must trigger a
    /// real header fetch, not the old count:0 "poke" that produced phantom
    /// notifications for already-moved/deleted messages.
    #[tokio::test]
    async fn sync_condstore_fetches_and_announces_new_messages() {
        let data_dir = TempDir::new().unwrap();
        let user_hash = seed_user_with_one_message(data_dir.path(), 5).await;
        let db_pool_manager = test_db_pool_manager(data_dir.path());
        let creds = test_creds();
        let event_bus = Arc::new(EventBus::new());
        let search_engine = Arc::new(crate::search::engine::SearchEngine::new(data_dir.path().to_path_buf()));

        let mut rx = event_bus.subscribe(&user_hash).await;

        let mock = MockImapClient::new()
            .with_headers(vec![new_header(1, "Existing", "Carol"), new_header(2, "Fresh mail", "Dave")])
            .with_folder_status_extended(FolderStatusExtended {
                uid_validity: 1,
                exists: 2,
                uid_next: 3,
                unseen: 1,
                highest_modseq: 6,
            });
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);

        run_sync(&user_hash, &creds, imap_client.as_ref(), &event_bus, &search_engine, &db_pool_manager)
            .await
            .expect("sync should succeed");

        let msg = db::pool::with_user_db(&db_pool_manager, &user_hash, |conn| {
            db::messages::get_single_message(conn, "INBOX", 2)
        })
        .await
        .unwrap();
        assert!(msg.is_some(), "new message should have been fetched and cached on the CONDSTORE path");

        let mut saw_real_new_messages = false;
        while let Ok(event) = rx.try_recv() {
            if let MailEvent::FolderStateChanged { count, .. } = event {
                assert!(count > 0, "CONDSTORE path must not publish a phantom count:0 FolderStateChanged event");
                saw_real_new_messages = true;
            }
        }
        assert!(saw_real_new_messages, "expected a FolderStateChanged event for the new message");
    }
}
