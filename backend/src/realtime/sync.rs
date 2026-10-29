use std::collections::{HashMap, HashSet};

use crate::db;
#[cfg(test)]
use crate::db::folders::UpsertFolderParams;
use crate::db::messages::{CacheMessageBodyParams, UpsertMessageParams};
use crate::email_theme;
use crate::imap::client::{ImapClient, ImapCredentials};
use crate::imap::types::ImapMessageHeader;
use crate::realtime::events::{EventBus, MailEvent};
use crate::search::engine::{IndexableMessage, SearchEngine};
use crate::search::engine::UserIndex;

/// How often to run a sync check (seconds).
/// STATUS checks are cheap (no SELECT), so 30s is a tight safety net
/// for non-INBOX folders that don't have IDLE.
/// How often `SyncWorkerManager`'s keepalive loop pokes a connected user's
/// worker, reused here so both sides agree on one cadence.
pub(crate) const SYNC_INTERVAL_SECS: u64 = 30;

/// The app-wide services a sync cycle needs, bundled so `run_sync` and its
/// helpers take one param instead of threading the same 6 dependencies
/// through every call.
#[derive(Clone, Copy)]
pub(crate) struct SyncCtx<'a> {
    pub user_hash: &'a str,
    pub creds: &'a ImapCredentials,
    pub imap_client: &'a dyn ImapClient,
    pub event_bus: &'a EventBus,
    pub search_engine: &'a SearchEngine,
    pub db_pool_manager: &'a db::pool::DbPoolManager,
}

/// Reconcile all of a user's folders using a 3-tier strategy per folder:
/// 1. STATUS pre-check (cheap, no SELECT)
/// 2. CONDSTORE incremental fetch (only changed flags)
/// 3. Full fetch fallback (when CONDSTORE unavailable)
///
/// Called by `SyncWorkerManager`'s worker loop in response to a wake-up
/// (IDLE new data, the periodic keepalive, or a stale-folder request) —
/// this is the single place that owns writing the per-user cache.
pub(crate) async fn run_sync(ctx: SyncCtx<'_>) -> Result<(), String> {
    let SyncCtx { user_hash, creds, imap_client, event_bus, search_engine, db_pool_manager } = ctx;
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
        // ── Tier 3: Full fetch fallback ──────────────────────────────
        let folder_changed = if status.highest_modseq > 0 && cached_modseq > 0 {
            sync_condstore(ctx, folder_name, cached_modseq, &status).await?
        } else {
            sync_full(ctx, folder_name, &status).await?
        };
        if folder_changed {
            any_changes = true;
        }

        // ── Gap fill: older messages missing from the cache ──────────
        // Tiers 2/3 only fetch mail newer than the cache, so an interrupted
        // first load would otherwise leave the folder incomplete for good.
        // Only for folders that have been loaded at least once (cached > 0),
        // so never-opened folders aren't downloaded in the background.
        let cached_count = db::pool::with_user_db(db_pool_manager, user_hash, {
            let folder_name = folder_name.clone();
            move |conn| Ok(db::messages::count_messages(conn, &folder_name).unwrap_or(0))
        })
        .await
        .map_err(|e| format!("DB error: {e}"))?;
        if cached_count > 0 && status.exists > cached_count && backfill_missing(ctx, folder_name).await? {
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
                db::messages::cache_message_body(conn, &folder, uid, CacheMessageBodyParams { html: text_html.as_deref(), text: text_plain.as_deref(), attachments_json: att_json.as_deref(), raw_headers: Some(&raw_headers), email_theme: detected_theme })
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
/// Store fetched headers in the cache. Best-effort per message: a row that
/// fails to write is skipped (a later sync or backfill picks it up again).
fn upsert_headers(conn: &rusqlite::Connection, folder_name: &str, headers: &[ImapMessageHeader]) {
    for header in headers {
        let from_address = header.from.first().map(|a| a.address.as_str()).unwrap_or("");
        let from_name = header.from.first().and_then(|a| a.name.as_deref()).unwrap_or("");
        let to_json = serde_json::to_string(&header.to).unwrap_or_else(|_| "[]".to_string());
        let cc_json = serde_json::to_string(&header.cc).unwrap_or_else(|_| "[]".to_string());
        let subject = header.subject.as_deref().unwrap_or("");
        let date = header.date.as_deref().unwrap_or("");
        let flags_csv = header.flags.join(",");

        if db::messages::upsert_message(conn, folder_name, header.uid, UpsertMessageParams { message_id: header.message_id.as_deref(), in_reply_to: header.in_reply_to.as_deref(), references_header: header.references.as_deref(), subject, from_address, from_name, to_json: &to_json, cc_json: &cc_json, date, date_epoch: header.date_epoch, flags_csv: &flags_csv, size: header.size, has_attachments: header.has_attachments, snippet: "", reaction: header.reaction.as_deref() }).is_err() {
            continue;
        }

        let _ = db::contacts::populate_known_addresses(conn, from_address, from_name, &to_json, &cc_json);
    }
}

/// Add fetched headers to the user's search index (skips Spam/Junk/Trash).
fn index_headers(search_engine: &SearchEngine, user_hash: &str, folder_name: &str, headers: &[ImapMessageHeader]) {
    if UserIndex::is_excluded_folder(folder_name) {
        return;
    }
    let Ok(user_index) = search_engine.open_user_index(user_hash) else {
        return;
    };
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
            upsert_headers(conn, &folder_name, &headers);
            Ok(())
        }
    })
    .await;

    if let Err(e) = upsert_result {
        tracing::warn!(error = %e, "Failed to open DB for new message upsert");
        return (0, None, None);
    }

    index_headers(search_engine, user_hash, folder_name, &headers);

    let latest = headers.iter().max_by_key(|h| h.uid).expect("headers non-empty");
    let latest_sender = latest
        .from
        .first()
        .map(|a| a.name.clone().unwrap_or_else(|| a.address.clone()));
    let latest_subject = latest.subject.clone();

    (headers.len() as u32, latest_sender, latest_subject)
}

/// UIDs requested per FETCH while backfilling.
const BACKFILL_CHUNK: usize = 500;

/// Missing messages backfilled per folder per sync cycle, so a large first
/// load doesn't hold up new mail in other folders. The rest follows on the
/// next cycles.
const BACKFILL_MAX_PER_CYCLE: usize = 5000;

/// Consecutive failed fetches after which a backfill stops until the next
/// cycle: by then the server is likely unreachable rather than one message
/// being unparseable. Must exceed log2(BACKFILL_CHUNK) + 2, the most failures
/// in a row that bisecting a chunk with one bad message can produce.
const BACKFILL_MAX_CONSECUTIVE_FAILURES: u32 = 20;

/// Format ascending UIDs as a compact IMAP UID set, e.g. `1:5,9,12:14`.
fn to_uid_set(uids: &[u32]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < uids.len() {
        let start = uids[i];
        let mut end = start;
        while i + 1 < uids.len() && uids[i + 1] == end + 1 {
            i += 1;
            end = uids[i];
        }
        parts.push(if start == end { start.to_string() } else { format!("{start}:{end}") });
        i += 1;
    }
    parts.join(",")
}

/// Fetch headers for `uids` (ascending) without letting one bad message
/// block the rest: a failed fetch is split in halves and retried, and a
/// single UID that still fails is fetched without BODYSTRUCTURE, or skipped
/// if that fails too. Returns the headers fetched and whether it gave up
/// after too many consecutive failures.
async fn fetch_headers_resilient(
    imap_client: &dyn ImapClient,
    creds: &ImapCredentials,
    folder_name: &str,
    uids: &[u32],
    consecutive_failures: &mut u32,
) -> (Vec<ImapMessageHeader>, bool) {
    let mut headers = Vec::new();
    let mut pending: Vec<&[u32]> = vec![uids];

    while let Some(set) = pending.pop() {
        match imap_client.fetch_headers(creds, folder_name, &to_uid_set(set)).await {
            Ok(fetched) => {
                *consecutive_failures = 0;
                headers.extend(fetched);
                continue;
            }
            Err(e) => {
                *consecutive_failures += 1;
                if *consecutive_failures >= BACKFILL_MAX_CONSECUTIVE_FAILURES {
                    tracing::warn!(folder = %folder_name, error = %e, "Backfill: too many failed fetches, retrying next cycle");
                    return (headers, true);
                }
                if set.len() > 1 {
                    let (first, second) = set.split_at(set.len() / 2);
                    pending.push(first);
                    pending.push(second);
                    continue;
                }
            }
        }

        let uid = set[0];
        match imap_client.fetch_headers_basic(creds, folder_name, &uid.to_string()).await {
            Ok(fetched) => {
                *consecutive_failures = 0;
                tracing::warn!(folder = %folder_name, uid = uid, "Backfill: fetched header without BODYSTRUCTURE (unparseable), attachment flag unavailable");
                headers.extend(fetched);
            }
            Err(e) => {
                tracing::warn!(folder = %folder_name, uid = uid, error = %e, "Backfill: skipping message that cannot be fetched");
            }
        }
    }

    (headers, false)
}

/// Fetch headers for messages that exist on the server but not in the cache,
/// newest first, in batches stored as they arrive. Returns whether anything
/// was stored.
async fn backfill_missing(ctx: SyncCtx<'_>, folder_name: &str) -> Result<bool, String> {
    let SyncCtx { user_hash, creds, imap_client, event_bus, search_engine, db_pool_manager } = ctx;

    let server_uids = match imap_client.fetch_uids_and_flags(creds, folder_name).await {
        Ok(state) => state,
        Err(e) => {
            tracing::debug!(folder = %folder_name, error = %e, "Backfill skipped (UID list fetch failed)");
            return Ok(false);
        }
    };

    let cached: HashSet<u32> = db::pool::with_user_db(db_pool_manager, user_hash, {
        let folder_name = folder_name.to_string();
        move |conn| db::messages::get_all_uids_and_flags(conn, &folder_name)
    })
    .await
    .map_err(|e| format!("DB error: {e}"))?
    .into_iter()
    .map(|(uid, _)| uid)
    .collect();

    let mut missing: Vec<u32> = server_uids
        .into_iter()
        .map(|(uid, _)| uid)
        .filter(|uid| !cached.contains(uid))
        .collect();
    if missing.is_empty() {
        return Ok(false);
    }
    let total_missing = missing.len();
    missing.sort_unstable_by(|a, b| b.cmp(a));
    missing.truncate(BACKFILL_MAX_PER_CYCLE);

    tracing::info!(folder = %folder_name, missing = total_missing, this_cycle = missing.len(), "Backfill: fetching missing message headers");

    let mut stored = 0usize;
    let mut consecutive_failures = 0u32;
    for chunk in missing.chunks(BACKFILL_CHUNK) {
        let mut chunk = chunk.to_vec();
        chunk.sort_unstable();
        let (headers, gave_up) =
            fetch_headers_resilient(imap_client, creds, folder_name, &chunk, &mut consecutive_failures).await;

        if !headers.is_empty() {
            let upsert_result = db::pool::with_user_db(db_pool_manager, user_hash, {
                let folder_name = folder_name.to_string();
                let headers = headers.clone();
                move |conn| {
                    upsert_headers(conn, &folder_name, &headers);
                    Ok(())
                }
            })
            .await;
            if let Err(e) = upsert_result {
                tracing::warn!(folder = %folder_name, error = %e, "Backfill: failed to open DB, retrying next cycle");
                break;
            }
            index_headers(search_engine, user_hash, folder_name, &headers);
            stored += headers.len();
        }

        if gave_up {
            break;
        }
    }

    if stored > 0 {
        let _ = db::pool::with_user_db(db_pool_manager, user_hash, {
            let folder_name = folder_name.to_string();
            move |conn| db::folders::refresh_unread_count(conn, &folder_name)
        })
        .await;
        event_bus
            .publish(user_hash, MailEvent::FolderUpdated { folder: Some(folder_name.to_string()) })
            .await;
    }

    tracing::info!(folder = %folder_name, stored = stored, remaining = total_missing - stored, "Backfill: stored message headers");
    Ok(stored > 0)
}

/// Lightweight snapshot of a cached folder for the sync loop.
struct FolderSnapshot {
    name: String,
    uid_validity: u32,
    highest_modseq: u64,
}

/// CONDSTORE path: fetch only changed flags, detect deletions via count comparison.
async fn sync_condstore(
    ctx: SyncCtx<'_>,
    folder_name: &str,
    cached_modseq: u64,
    status: &crate::imap::types::FolderStatusExtended,
) -> Result<bool, String> {
    let SyncCtx { user_hash, creds, imap_client, event_bus, search_engine, db_pool_manager } = ctx;
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
            return sync_full(ctx, folder_name, status).await;
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
async fn sync_full(
    ctx: SyncCtx<'_>,
    folder_name: &str,
    status: &crate::imap::types::FolderStatusExtended,
) -> Result<bool, String> {
    let SyncCtx { user_hash, creds, imap_client, event_bus, search_engine, db_pool_manager } = ctx;
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
            password: crate::test_support::FAKE_PASSWORD.to_string(),
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
            db::folders::upsert_folder(conn, UpsertFolderParams { name: "INBOX", delimiter: None, parent: None, flags_csv: "", is_subscribed: true, total_count: 1, unread_count: 0, uid_validity: 1, highest_modseq })?;
            db::messages::upsert_message(conn, "INBOX", 1, UpsertMessageParams { message_id: None, in_reply_to: None, references_header: None, subject: "Existing", from_address: "carol@example.com", from_name: "Carol", to_json: "[]", cc_json: "[]", date: "2024-01-01T00:00:00Z", date_epoch: 1, flags_csv: "", size: 512, has_attachments: false, snippet: "", reaction: None })?;
            Ok(())
        })
        .await
        .unwrap();

        user_hash
    }

    #[test]
    fn to_uid_set_collapses_consecutive_runs() {
        assert_eq!(to_uid_set(&[1, 2, 3, 5, 7, 8]), "1:3,5,7:8");
        assert_eq!(to_uid_set(&[42]), "42");
        assert_eq!(to_uid_set(&[]), "");
    }

    /// Seed INBOX with only its newest message cached (uid `newest`), like a
    /// first load whose background fetch of older headers failed.
    async fn seed_user_with_newest_message(data_dir: &std::path::Path, newest: u32, highest_modseq: u64) -> String {
        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        crate::auth::user_data::provision_user_data(data_dir.to_str().unwrap(), &user_hash).unwrap();

        let db_pool_manager = test_db_pool_manager(data_dir);
        db::pool::with_user_db(&db_pool_manager, &user_hash, move |conn| {
            db::folders::upsert_folder(conn, UpsertFolderParams { name: "INBOX", delimiter: None, parent: None, flags_csv: "", is_subscribed: true, total_count: 1, unread_count: 0, uid_validity: 1, highest_modseq })?;
            db::messages::upsert_message(conn, "INBOX", newest, UpsertMessageParams { message_id: None, in_reply_to: None, references_header: None, subject: "Newest", from_address: "carol@example.com", from_name: "Carol", to_json: "[]", cc_json: "[]", date: "2024-01-01T00:00:00Z", date_epoch: newest as i64, flags_csv: "", size: 512, has_attachments: false, snippet: "", reaction: None })?;
            Ok(())
        })
        .await
        .unwrap();

        user_hash
    }

    /// A folder stuck with only its newest messages cached (unchanged
    /// MODSEQ, nothing newer on the server) must have its older messages
    /// backfilled, across several batches and despite one message whose
    /// full header fetch always fails.
    #[tokio::test]
    async fn sync_backfills_older_messages_missing_from_cache() {
        let data_dir = TempDir::new().unwrap();
        let user_hash = seed_user_with_newest_message(data_dir.path(), 1200, 5).await;
        let db_pool_manager = test_db_pool_manager(data_dir.path());
        let creds = test_creds();
        let event_bus = Arc::new(EventBus::new());
        let search_engine = Arc::new(crate::search::engine::SearchEngine::new(data_dir.path().to_path_buf()));

        let headers: Vec<ImapMessageHeader> = (1..=1200)
            .map(|uid| ImapMessageHeader { has_attachments: uid == 437, ..new_header(uid, "Old mail", "Dave") })
            .collect();
        let mock = MockImapClient::new()
            .with_headers(headers)
            .with_unparseable_uids(vec![437])
            .with_folder_status_extended(FolderStatusExtended {
                uid_validity: 1,
                exists: 1200,
                uid_next: 1201,
                unseen: 0,
                highest_modseq: 5,
            });
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);

        run_sync(SyncCtx { user_hash: &user_hash, creds: &creds, imap_client: imap_client.as_ref(), event_bus: &event_bus, search_engine: &search_engine, db_pool_manager: &db_pool_manager })
            .await
            .expect("sync should succeed");

        let (count, bad) = db::pool::with_user_db(&db_pool_manager, &user_hash, |conn| {
            Ok((db::messages::count_messages(conn, "INBOX")?, db::messages::get_single_message(conn, "INBOX", 437)?))
        })
        .await
        .unwrap();
        assert_eq!(count, 1200, "all older messages should have been backfilled");
        let bad = bad.expect("unparseable message should be stored via the basic fetch");
        assert!(!bad.has_attachments, "basic fetch has no BODYSTRUCTURE, so no attachment flag");
    }

    /// Folders the user never opened (nothing cached) are not downloaded in
    /// the background.
    #[tokio::test]
    async fn sync_does_not_backfill_never_loaded_folders() {
        let data_dir = TempDir::new().unwrap();
        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        crate::auth::user_data::provision_user_data(data_dir.path().to_str().unwrap(), &user_hash).unwrap();
        let db_pool_manager = test_db_pool_manager(data_dir.path());
        db::pool::with_user_db(&db_pool_manager, &user_hash, |conn| {
            db::folders::upsert_folder(conn, UpsertFolderParams { name: "INBOX", delimiter: None, parent: None, flags_csv: "", is_subscribed: true, total_count: 0, unread_count: 0, uid_validity: 1, highest_modseq: 0 })
        })
        .await
        .unwrap();
        let creds = test_creds();
        let event_bus = Arc::new(EventBus::new());
        let search_engine = Arc::new(crate::search::engine::SearchEngine::new(data_dir.path().to_path_buf()));

        let mock = MockImapClient::new().with_headers((1..=10).map(|uid| new_header(uid, "Mail", "Dave")).collect());
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);

        run_sync(SyncCtx { user_hash: &user_hash, creds: &creds, imap_client: imap_client.as_ref(), event_bus: &event_bus, search_engine: &search_engine, db_pool_manager: &db_pool_manager })
            .await
            .expect("sync should succeed");

        let count = db::pool::with_user_db(&db_pool_manager, &user_hash, |conn| db::messages::count_messages(conn, "INBOX"))
            .await
            .unwrap();
        assert_eq!(count, 0);
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

        run_sync(SyncCtx { user_hash: &user_hash, creds: &creds, imap_client: imap_client.as_ref(), event_bus: &event_bus, search_engine: &search_engine, db_pool_manager: &db_pool_manager })
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

        run_sync(SyncCtx { user_hash: &user_hash, creds: &creds, imap_client: imap_client.as_ref(), event_bus: &event_bus, search_engine: &search_engine, db_pool_manager: &db_pool_manager })
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
