use rusqlite::{Connection, params};
use serde::Serialize;

#[cfg(test)]
use crate::db::folders::UpsertFolderParams;

// mail_parser is used for RFC2822 date parsing in parse_date_epoch.

/// A cached email message header, mirroring the query-visible columns of the
/// `messages` table.
#[derive(Debug, Clone, Serialize)]
pub struct CachedMessage {
    pub uid: u32,
    pub folder: String,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references_header: Option<String>,
    pub subject: String,
    pub from_address: String,
    pub from_name: String,
    pub to_addresses: String,
    pub cc_addresses: String,
    pub date: String,
    pub flags: String,
    pub size: u32,
    pub has_attachments: bool,
    pub snippet: String,
    pub reaction: Option<String>,
    pub date_epoch: i64,
}

// ---------------------------------------------------------------------------
// Helper to map a row to CachedMessage (used in multiple queries)
// ---------------------------------------------------------------------------

fn row_to_cached_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<CachedMessage> {
    let has_attachments_int: i32 = row.get(12)?;
    Ok(CachedMessage {
        uid: row.get(0)?,
        folder: row.get(1)?,
        message_id: row.get(2)?,
        in_reply_to: row.get(3)?,
        references_header: row.get(4)?,
        subject: row.get(5)?,
        from_address: row.get(6)?,
        from_name: row.get(7)?,
        to_addresses: row.get(8)?,
        cc_addresses: row.get(9)?,
        date: row.get(10)?,
        flags: row.get(11)?,
        size: row.get(13)?,
        has_attachments: has_attachments_int != 0,
        snippet: row.get(14)?,
        reaction: row.get(15)?,
        date_epoch: row.get(16)?,
    })
}

/// The SELECT column list used by all queries that return `CachedMessage`.
const MSG_SELECT_COLS: &str =
    "uid, folder, message_id, in_reply_to, references_header,
     subject, from_address, from_name, to_addresses, cc_addresses,
     date, flags, has_attachments, size, snippet, reaction, date_epoch";

/// Compute the canonical thread_id for a message.
///
/// Uses the first message-id in the References chain (oldest ancestor per RFC 2822),
/// falling back to In-Reply-To, then the message's own Message-ID.
pub fn compute_thread_id(
    message_id: Option<&str>,
    in_reply_to: Option<&str>,
    references_header: Option<&str>,
) -> Option<String> {
    if let Some(refs) = references_header
        && let Some(first) = refs.split_whitespace().next()
        && !first.is_empty()
    {
        return Some(first.to_string());
    }
    if let Some(irt) = in_reply_to.filter(|s| !s.is_empty()) {
        return Some(irt.to_string());
    }
    message_id.filter(|s| !s.is_empty()).map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parameters for [`upsert_message`].
pub struct UpsertMessageParams<'a> {
    pub message_id: Option<&'a str>,
    pub in_reply_to: Option<&'a str>,
    pub references_header: Option<&'a str>,
    pub subject: &'a str,
    pub from_address: &'a str,
    pub from_name: &'a str,
    pub to_json: &'a str,
    pub cc_json: &'a str,
    pub date: &'a str,
    pub date_epoch: i64,
    pub flags_csv: &'a str,
    pub size: u32,
    pub has_attachments: bool,
    pub snippet: &'a str,
    pub reaction: Option<&'a str>,
}

/// Insert or replace a message header row.
pub fn upsert_message(
    conn: &Connection,
    folder: &str,
    uid: u32,
    p: UpsertMessageParams,
) -> Result<(), String> {
    let thread_id = compute_thread_id(p.message_id, p.in_reply_to, p.references_header);
    conn.execute(
        "INSERT OR REPLACE INTO messages
            (uid, folder, message_id, in_reply_to, references_header,
             subject, from_address, from_name, to_addresses, cc_addresses,
             date, flags, size, has_attachments, snippet, date_epoch, reaction, thread_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            uid,
            folder,
            p.message_id,
            p.in_reply_to,
            p.references_header,
            p.subject,
            p.from_address,
            p.from_name,
            p.to_json,
            p.cc_json,
            p.date,
            p.flags_csv,
            p.size,
            p.has_attachments as i32,
            p.snippet,
            p.date_epoch,
            p.reaction,
            thread_id,
        ],
    )
    .map_err(|e| format!("Failed to upsert message: {e}"))?;
    Ok(())
}

/// Delete all messages in a folder (used when UIDVALIDITY changes).
pub fn delete_folder_messages(conn: &Connection, folder: &str) -> Result<usize, String> {
    let deleted = conn
        .execute("DELETE FROM messages WHERE folder = ?1", params![folder])
        .map_err(|e| format!("Failed to delete folder messages: {e}"))?;
    Ok(deleted)
}

/// Return the highest cached UID for a folder, or 0 if no messages are cached.
pub fn max_uid(conn: &Connection, folder: &str) -> Result<u32, String> {
    conn.query_row(
        "SELECT COALESCE(MAX(uid), 0) FROM messages WHERE folder = ?1",
        params![folder],
        |row| row.get(0),
    )
    .map_err(|e| format!("Failed to get max uid: {e}"))
}

/// Parse a date string to a Unix epoch timestamp (seconds).
/// Tries ISO 8601 first, then RFC2822. Returns 0 on parse failure.
pub(crate) fn parse_date_epoch(date: &str) -> i64 {
    if date.is_empty() {
        return 0;
    }
    // Try ISO 8601 format first (used in test fixtures and some IMAP servers).
    if let Ok(epoch) = parse_iso8601_to_epoch(date) {
        return epoch;
    }
    // Try mail-parser's RFC2822 date parsing (standard IMAP date format).
    if let Some(dt) = mail_parser::DateTime::parse_rfc822(date) {
        let epoch = datetime_to_epoch(&dt);
        // Sanity check: mail_parser can misparse non-RFC2822 dates.
        if dt.year >= 1970 && dt.month >= 1 && dt.month <= 12 && epoch > 0 {
            return epoch;
        }
    }
    0
}

/// Convert a mail_parser::DateTime to a Unix epoch timestamp.
fn datetime_to_epoch(dt: &mail_parser::DateTime) -> i64 {
    // Days from year 1970 to the start of the given year
    let year = dt.year as i64;
    let mut days: i64 = 0;
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }
    for y in (year..1970).rev() {
        days -= if is_leap_year(y) { 366 } else { 365 };
    }
    // Days from start of year to start of month
    let month_days: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let month = (dt.month as usize).saturating_sub(1).min(11);
    for (m, &d) in month_days.iter().enumerate().take(month) {
        days += d;
        if m == 1 && is_leap_year(year) {
            days += 1;
        }
    }
    days += (dt.day as i64) - 1;

    let secs = days * 86400 + (dt.hour as i64) * 3600 + (dt.minute as i64) * 60 + (dt.second as i64);
    // tz_before_gmt=true means the offset is negative (behind UTC)
    let tz_offset_secs = (dt.tz_hour as i64) * 3600 + (dt.tz_minute as i64) * 60;
    if dt.tz_before_gmt {
        secs + tz_offset_secs
    } else {
        secs - tz_offset_secs
    }
}

fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Parse ISO 8601 dates like "2024-01-01T10:00:00Z"
fn parse_iso8601_to_epoch(s: &str) -> Result<i64, ()> {
    // Simple parser for YYYY-MM-DDTHH:MM:SSZ
    let s = s.trim();
    if s.len() < 19 {
        return Err(());
    }
    let year: i64 = s[0..4].parse().map_err(|_| ())?;
    let month: u32 = s[5..7].parse().map_err(|_| ())?;
    let day: u32 = s[8..10].parse().map_err(|_| ())?;
    let hour: u32 = s[11..13].parse().map_err(|_| ())?;
    let minute: u32 = s[14..16].parse().map_err(|_| ())?;
    let second: u32 = s[17..19].parse().map_err(|_| ())?;

    let dt = mail_parser::DateTime {
        year: year as u16,
        month: month as u8,
        day: day as u8,
        hour: hour as u8,
        minute: minute as u8,
        second: second as u8,
        tz_before_gmt: false,
        tz_hour: 0,
        tz_minute: 0,
    };
    Ok(datetime_to_epoch(&dt))
}

/// Return a page of messages for a folder, ordered by date descending.
/// `page` is 0-indexed.
#[allow(dead_code)]
pub fn get_messages(
    conn: &Connection,
    folder: &str,
    page: u32,
    per_page: u32,
) -> Result<Vec<CachedMessage>, String> {
    let offset = page * per_page;
    let sql = format!(
        "SELECT {MSG_SELECT_COLS}
         FROM messages
         WHERE folder = ?1
         ORDER BY date_epoch DESC, uid DESC
         LIMIT ?2 OFFSET ?3"
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare get_messages: {e}"))?;

    let rows = stmt
        .query_map(params![folder, per_page, offset], |row| {
            row_to_cached_message(row)
        })
        .map_err(|e| format!("Failed to query messages: {e}"))?;

    let mut messages = Vec::new();
    for row in rows {
        messages.push(row.map_err(|e| format!("Failed to read message row: {e}"))?);
    }
    Ok(messages)
}

/// Return the total number of cached messages for a folder.
pub fn count_messages(conn: &Connection, folder: &str) -> Result<u32, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE folder = ?1",
        params![folder],
        |row| row.get(0),
    )
    .map_err(|e| format!("Failed to count messages: {e}"))
}

/// A row returned by the threaded list query: the latest message in each thread
/// plus per-thread aggregates.
#[derive(Debug, Clone)]
pub struct ThreadedMessage {
    pub msg: CachedMessage,
    /// Total number of messages in this thread (within the folder).
    pub thread_count: u32,
    /// Number of unread messages in this thread.
    pub unread_count: u32,
}

/// Return one row per thread for a folder, ordered by the thread's most-recent
/// message date descending. Each row contains the latest message's fields plus
/// thread_count and unread_count aggregates.
///
/// Uses SQLite window functions (available since 3.25, Sep 2018).
pub fn get_threaded_messages(
    conn: &Connection,
    folder: &str,
    page: u32,
    per_page: u32,
) -> Result<Vec<ThreadedMessage>, String> {
    let offset = page * per_page;
    // COALESCE(thread_id, CAST(uid AS TEXT)) ensures messages without a thread_id
    // each form their own single-message thread.
    let sql = format!(
        "SELECT uid, folder, message_id, in_reply_to, references_header,
                subject, from_address, from_name, to_addresses, cc_addresses,
                date, flags, has_attachments, size, snippet, reaction, date_epoch,
                thread_count, unread_count
         FROM (
           SELECT {MSG_SELECT_COLS},
                  COUNT(*) OVER (PARTITION BY COALESCE(thread_id, CAST(uid AS TEXT))) AS thread_count,
                  SUM(CASE WHEN flags NOT LIKE '%\\Seen%' THEN 1 ELSE 0 END)
                    OVER (PARTITION BY COALESCE(thread_id, CAST(uid AS TEXT))) AS unread_count,
                  ROW_NUMBER() OVER (
                    PARTITION BY COALESCE(thread_id, CAST(uid AS TEXT))
                    ORDER BY date_epoch DESC, uid DESC
                  ) AS rn,
                  MAX(date_epoch) OVER (PARTITION BY COALESCE(thread_id, CAST(uid AS TEXT))) AS thread_latest_epoch
           FROM messages
           WHERE folder = ?1
         )
         WHERE rn = 1
         ORDER BY thread_latest_epoch DESC, uid DESC
         LIMIT ?2 OFFSET ?3"
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare get_threaded_messages: {e}"))?;

    let rows = stmt
        .query_map(params![folder, per_page, offset], |row| {
            let msg = row_to_cached_message(row)?;
            let thread_count: u32 = row.get(17)?;
            let unread_count: u32 = row.get(18)?;
            Ok(ThreadedMessage { msg, thread_count, unread_count })
        })
        .map_err(|e| format!("Failed to query threaded messages: {e}"))?;

    let mut messages = Vec::new();
    for row in rows {
        messages.push(row.map_err(|e| format!("Failed to read threaded message row: {e}"))?);
    }
    Ok(messages)
}

/// Return the number of distinct threads in a folder (for pagination).
pub fn count_threads(conn: &Connection, folder: &str) -> Result<u32, String> {
    conn.query_row(
        "SELECT COUNT(DISTINCT COALESCE(thread_id, CAST(uid AS TEXT)))
         FROM messages WHERE folder = ?1",
        params![folder],
        |row| row.get(0),
    )
    .map_err(|e| format!("Failed to count threads: {e}"))
}

/// Update only the flags column for a specific message.
pub fn update_message_flags(
    conn: &Connection,
    folder: &str,
    uid: u32,
    flags_csv: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE messages SET flags = ?1 WHERE folder = ?2 AND uid = ?3",
        params![flags_csv, folder, uid],
    )
    .map_err(|e| format!("Failed to update message flags: {e}"))?;
    Ok(())
}

/// Parameters for [`cache_message_body`].
pub struct CacheMessageBodyParams<'a> {
    pub html: Option<&'a str>,
    pub text: Option<&'a str>,
    pub attachments_json: Option<&'a str>,
    pub raw_headers: Option<&'a str>,
    pub email_theme: Option<i32>,
}

/// Cache a message body (HTML and/or plain text) along with attachment
/// metadata (as a JSON string) and the raw RFC-822 headers.
pub fn cache_message_body(
    conn: &Connection,
    folder: &str,
    uid: u32,
    p: CacheMessageBodyParams,
) -> Result<(), String> {
    conn.execute(
        "UPDATE messages
         SET body_html = ?1, body_text = ?2, body_cached = 1,
             attachments_json = ?3, raw_headers = ?4, email_theme = ?5
         WHERE folder = ?6 AND uid = ?7",
        params![p.html, p.text, p.attachments_json, p.raw_headers, p.email_theme, folder, uid],
    )
    .map_err(|e| format!("Failed to cache message body: {e}"))?;
    Ok(())
}

pub fn update_email_theme(
    conn: &Connection,
    folder: &str,
    uid: u32,
    email_theme: i32,
) -> Result<(), String> {
    conn.execute(
        "UPDATE messages SET email_theme = ?1 WHERE folder = ?2 AND uid = ?3",
        params![email_theme, folder, uid],
    )
    .map_err(|e| format!("Failed to update email theme: {e}"))?;
    Ok(())
}

/// Cached body data including attachment metadata and raw headers.
pub struct CachedBody {
    pub html: Option<String>,
    pub text: Option<String>,
    pub attachments_json: Option<String>,
    pub raw_headers: Option<String>,
    pub email_theme: Option<i32>,
}

/// Return the cached body if `body_cached = 1`, otherwise `None`.
pub fn get_cached_body(
    conn: &Connection,
    folder: &str,
    uid: u32,
) -> Result<Option<CachedBody>, String> {
    let result = conn.query_row(
        "SELECT body_cached, body_html, body_text, attachments_json, raw_headers, email_theme
         FROM messages
         WHERE folder = ?1 AND uid = ?2",
        params![folder, uid],
        |row| {
            let cached: i32 = row.get(0)?;
            if cached == 1 {
                Ok(Some(CachedBody {
                    html: row.get(1)?,
                    text: row.get(2)?,
                    attachments_json: row.get(3)?,
                    raw_headers: row.get(4)?,
                    email_theme: row.get(5)?,
                }))
            } else {
                Ok(None)
            }
        },
    );

    match result {
        Ok(body) => Ok(body),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to get cached body: {e}")),
    }
}

/// Return a single message by folder and UID, or None if not found.
pub fn get_single_message(conn: &Connection, folder: &str, uid: u32) -> Result<Option<CachedMessage>, String> {
    let sql = format!(
        "SELECT {MSG_SELECT_COLS}
         FROM messages
         WHERE folder = ?1 AND uid = ?2"
    );

    let result = conn.query_row(&sql, params![folder, uid], |row| {
        row_to_cached_message(row)
    });

    match result {
        Ok(msg) => Ok(Some(msg)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to get single message: {e}")),
    }
}

/// Find the folder and UID for a message by its Message-ID header value.
/// Returns `None` if not in the local cache.
pub fn find_by_message_id(conn: &Connection, message_id: &str) -> Result<Option<(String, u32)>, String> {
    let result = conn.query_row(
        "SELECT folder, uid FROM messages WHERE message_id = ?1 LIMIT 1",
        params![message_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
    );
    match result {
        Ok(pair) => Ok(Some(pair)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to find message by id: {e}")),
    }
}

/// Return the subset of `uids` that have a cached row in `folder`.
///
/// Used by bulk message operations to tell the caller which requested UIDs
/// didn't correspond to an actual message (already deleted elsewhere, stale
/// client state, a bad ID), instead of silently doing nothing for them.
pub fn filter_existing_uids(
    conn: &Connection,
    folder: &str,
    uids: &[u32],
) -> Result<std::collections::HashSet<u32>, String> {
    if uids.is_empty() {
        return Ok(std::collections::HashSet::new());
    }

    let placeholders: Vec<String> = (2..=uids.len() + 1).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "SELECT uid FROM messages WHERE folder = ?1 AND uid IN ({})",
        placeholders.join(", ")
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| format!("prepare error: {e}"))?;
    let mut params: Vec<&dyn rusqlite::types::ToSql> = vec![&folder];
    params.extend(uids.iter().map(|u| u as &dyn rusqlite::types::ToSql));

    let rows = stmt
        .query_map(params.as_slice(), |row| row.get::<_, u32>(0))
        .map_err(|e| format!("Failed to query existing uids: {e}"))?;

    let mut found = std::collections::HashSet::with_capacity(uids.len());
    for row in rows {
        found.insert(row.map_err(|e| format!("Failed to read uid row: {e}"))?);
    }
    Ok(found)
}

/// Delete a single message by folder and UID.
pub fn delete_message(conn: &Connection, folder: &str, uid: u32) -> Result<(), String> {
    conn.execute(
        "DELETE FROM messages WHERE folder = ?1 AND uid = ?2",
        params![folder, uid],
    )
    .map_err(|e| format!("Failed to delete message: {e}"))?;
    Ok(())
}

/// Return `(folder, uid)` pairs for messages that don't have a cached body,
/// ordered by date_epoch DESC (most recent first), limited to `limit`.
/// Used by the deep-index background task to know which bodies to fetch.
pub fn get_unindexed_messages(conn: &Connection, limit: u32) -> Result<Vec<(String, u32)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT folder, uid FROM messages
             WHERE body_cached = 0
             ORDER BY date_epoch DESC
             LIMIT ?1",
        )
        .map_err(|e| format!("Failed to prepare get_unindexed_messages: {e}"))?;

    let rows = stmt
        .query_map(params![limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        })
        .map_err(|e| format!("Failed to query unindexed messages: {e}"))?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| format!("Failed to read unindexed row: {e}"))?);
    }
    Ok(result)
}

/// Return all (uid, flags_csv) pairs for a folder, for reconciliation.
/// Fetch all cached messages with uid > min_uid in a folder.
/// Used by the IDLE pipeline to find newly arrived messages for processing.
pub fn get_messages_after_uid(
    conn: &Connection,
    folder: &str,
    min_uid: u32,
) -> Result<Vec<CachedMessage>, String> {
    let sql = format!(
        "SELECT {MSG_SELECT_COLS} FROM messages WHERE folder = ?1 AND uid > ?2 ORDER BY uid ASC"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("prepare error: {e}"))?;
    let rows = stmt
        .query_map(rusqlite::params![folder, min_uid], row_to_cached_message)
        .map_err(|e| format!("query error: {e}"))?;
    let mut msgs = Vec::new();
    for row in rows {
        msgs.push(row.map_err(|e| format!("row error: {e}"))?);
    }
    Ok(msgs)
}

pub fn list_messages_in_folder(conn: &Connection, folder: &str) -> Result<Vec<CachedMessage>, String> {
    let sql = format!(
        "SELECT {MSG_SELECT_COLS} FROM messages WHERE folder = ?1 ORDER BY uid ASC"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("prepare error: {e}"))?;
    let rows = stmt
        .query_map(rusqlite::params![folder], row_to_cached_message)
        .map_err(|e| format!("query error: {e}"))?;
    let mut msgs = Vec::new();
    for row in rows {
        msgs.push(row.map_err(|e| format!("row error: {e}"))?);
    }
    Ok(msgs)
}

pub fn get_all_uids_and_flags(conn: &Connection, folder: &str) -> Result<Vec<(u32, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT uid, flags FROM messages WHERE folder = ?1")
        .map_err(|e| format!("Failed to prepare get_all_uids_and_flags: {e}"))?;
    let rows = stmt
        .query_map(params![folder], |row| {
            Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("Failed to query uids and flags: {e}"))?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| format!("Failed to read uid/flags row: {e}"))?);
    }
    Ok(result)
}

/// Find messages related to the given `target_message_id` for threading.
///
/// Returns messages where:
/// - `message_id` equals `target_message_id`, OR
/// - `in_reply_to` equals `target_message_id`, OR
/// - `references_header` contains `target_message_id`
pub fn get_thread_messages(
    conn: &Connection,
    target_message_id: &str,
) -> Result<Vec<CachedMessage>, String> {
    // Escape % and _ characters in the message_id to prevent LIKE injection.
    let escaped_target = target_message_id.replace("%", "\\%").replace("_", "\\_");
    let like_pattern = format!("%{}%", escaped_target);
    let sql = format!(
        "SELECT {MSG_SELECT_COLS}
         FROM messages
         WHERE message_id = ?1
            OR in_reply_to = ?1
            OR references_header LIKE ?2 ESCAPE '\\'
         ORDER BY date_epoch ASC"
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare get_thread_messages: {e}"))?;

    let rows = stmt
        .query_map(params![target_message_id, like_pattern], |row| {
            row_to_cached_message(row)
        })
        .map_err(|e| format!("Failed to query thread messages: {e}"))?;

    let mut messages = Vec::new();
    for row in rows {
        messages.push(row.map_err(|e| format!("Failed to read thread message row: {e}"))?);
    }
    Ok(messages)
}

/// Build a complete email thread by transitively walking the References chain.
///
/// Starting from `message_id` and the optional space-separated `references` string,
/// this function repeatedly queries related messages (via `get_thread_messages`) and
/// collects every message_id, in_reply_to, and references_header ID it encounters.
/// The process repeats until no new IDs are discovered (transitive closure).
///
/// The result is deduplicated by `(folder, uid)` and sorted by `date_epoch` ascending.
pub fn get_full_thread(
    conn: &Connection,
    message_id: &str,
    references: Option<&str>,
) -> Result<Vec<CachedMessage>, String> {
    use std::collections::HashSet;

    // Step 1: Seed the set of known message IDs.
    let mut known_ids: HashSet<String> = HashSet::new();
    known_ids.insert(message_id.to_string());
    if let Some(refs) = references {
        for id in refs.split_whitespace() {
            if !id.is_empty() {
                known_ids.insert(id.to_string());
            }
        }
    }

    // Step 2-4: Iteratively query until no new IDs are found.
    let mut processed_ids: HashSet<String> = HashSet::new();
    let mut all_messages: Vec<CachedMessage> = Vec::new();

    loop {
        // Find IDs we haven't queried yet.
        let to_query: Vec<String> = known_ids
            .difference(&processed_ids)
            .cloned()
            .collect();

        if to_query.is_empty() {
            break;
        }

        for id in &to_query {
            processed_ids.insert(id.clone());

            let found = get_thread_messages(conn, id)?;
            for msg in found {
                // Extract IDs from each found message and add to known set.
                if let Some(ref mid) = msg.message_id
                    && !mid.is_empty()
                {
                    known_ids.insert(mid.clone());
                }
                if let Some(ref irt) = msg.in_reply_to
                    && !irt.is_empty()
                {
                    known_ids.insert(irt.clone());
                }
                if let Some(ref refs) = msg.references_header {
                    for r in refs.split_whitespace() {
                        if !r.is_empty() {
                            known_ids.insert(r.to_string());
                        }
                    }
                }
                all_messages.push(msg);
            }
        }
    }

    // Step 5: Deduplicate by (folder, uid) pair.
    let mut seen: HashSet<(String, u32)> = HashSet::new();
    all_messages.retain(|msg| seen.insert((msg.folder.clone(), msg.uid)));

    // Step 6: Sort by date_epoch ascending.
    all_messages.sort_by_key(|msg| msg.date_epoch);

    Ok(all_messages)
}

/// Filter criteria for [`search_messages_sqlite`].
#[derive(Default)]
pub struct SearchFilters<'a> {
    pub folder: Option<&'a str>,
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
    pub has_attachment: Option<bool>,
    pub is_read: Option<bool>,
    pub is_flagged: Option<bool>,
}

/// Search the SQLite message cache using LIKE for text matches across
/// subject, from_name, from_address, and to_addresses.
/// This provides comprehensive results independent of the tantivy index state.
pub fn search_messages_sqlite(
    conn: &Connection,
    text: &str,
    filters: SearchFilters,
    limit: usize,
) -> Result<Vec<CachedMessage>, String> {
    let SearchFilters { folder, from, to, date_from, date_to, has_attachment, is_read, is_flagged } = filters;
    let mut conditions = Vec::new();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1u32;

    // Exclude Trash/Spam/Junk from search results (same as tantivy).
    conditions.push("LOWER(folder) NOT IN ('trash', 'spam', 'junk')".to_string());

    if !text.is_empty() {
        let pattern = format!("%{text}%");
        conditions.push(format!(
            "(subject LIKE ?{idx} OR from_name LIKE ?{idx} OR from_address LIKE ?{idx} OR to_addresses LIKE ?{idx})"
        ));
        param_values.push(Box::new(pattern));
        idx += 1;
    }

    if let Some(f) = folder {
        conditions.push(format!("folder = ?{idx}"));
        param_values.push(Box::new(f.to_string()));
        idx += 1;
    }

    if let Some(f) = from {
        if f.contains('@') {
            conditions.push(format!("from_address = ?{idx}"));
        } else {
            conditions.push(format!("from_name LIKE ?{idx}"));
        }
        let val = if f.contains('@') { f.to_string() } else { format!("%{f}%") };
        param_values.push(Box::new(val));
        idx += 1;
    }

    if let Some(t) = to {
        conditions.push(format!("to_addresses LIKE ?{idx}"));
        param_values.push(Box::new(format!("%{t}%")));
        idx += 1;
    }

    if let Some(df) = date_from {
        conditions.push(format!("date_epoch >= ?{idx}"));
        param_values.push(Box::new(df));
        idx += 1;
    }

    if let Some(dt) = date_to {
        conditions.push(format!("date_epoch <= ?{idx}"));
        param_values.push(Box::new(dt));
        idx += 1;
    }

    if let Some(ha) = has_attachment {
        conditions.push(format!("has_attachments = ?{idx}"));
        param_values.push(Box::new(ha as i32));
        idx += 1;
    }

    if let Some(read) = is_read {
        if read {
            conditions.push(format!("flags LIKE ?{idx}"));
        } else {
            conditions.push(format!("flags NOT LIKE ?{idx}"));
        }
        param_values.push(Box::new("%\\Seen%".to_string()));
        idx += 1;
    }

    if let Some(true) = is_flagged {
        conditions.push(format!("flags LIKE ?{idx}"));
        param_values.push(Box::new("%\\Flagged%".to_string()));
        idx += 1;
    }

    let _ = idx; // suppress unused warning

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT {MSG_SELECT_COLS}
         FROM messages
         {where_clause}
         ORDER BY date_epoch DESC
         LIMIT ?{}", param_values.len() + 1
    );

    param_values.push(Box::new(limit as i64));

    let params_refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare search query: {e}"))?;

    let rows = stmt
        .query_map(rusqlite::params_from_iter(params_refs), |row| {
            row_to_cached_message(row)
        })
        .map_err(|e| format!("Failed to execute search query: {e}"))?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row.map_err(|e| format!("Failed to read search row: {e}"))?);
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::folders::upsert_folder;
    use crate::db::pool::open_test_db;

    /// Helper: insert a folder so that the foreign key constraint is satisfied.
    fn ensure_folder(conn: &Connection, name: &str) {
        upsert_folder(conn, UpsertFolderParams { name, delimiter: Some("/"), parent: None, flags_csv: "", is_subscribed: true, total_count: 0, unread_count: 0, uid_validity: 1, highest_modseq: 0 }).unwrap();
    }

    /// Helper: insert a sample message.
    fn insert_sample(conn: &Connection, folder: &str, uid: u32, date: &str) {
        upsert_message(conn, folder, uid, UpsertMessageParams { message_id: Some(&format!("<msg-{uid}@example.com>")), in_reply_to: None, references_header: None, subject: &format!("Subject {uid}"), from_address: "alice@example.com", from_name: "Alice", to_json: "[]", cc_json: "[]", date, date_epoch: parse_date_epoch(date), flags_csv: "\\Seen", size: 1024, has_attachments: false, snippet: "snippet", reaction: None })
        .unwrap();
    }

    #[test]
    fn test_parse_date_epoch() {
        let e1 = parse_date_epoch("2024-01-01T10:00:00Z");
        let e2 = parse_date_epoch("2024-01-02T10:00:00Z");
        let e3 = parse_date_epoch("2024-01-03T10:00:00Z");
        assert!(e1 > 0, "epoch for date 1 should be > 0, got {e1}");
        assert!(e2 > e1, "date 2 ({e2}) should be after date 1 ({e1})");
        assert!(e3 > e2, "date 3 ({e3}) should be after date 2 ({e2})");

        // RFC2822 format.
        let e4 = parse_date_epoch("Mon, 1 Jan 2024 10:00:00 +0000");
        assert!(e4 > 0, "epoch for rfc2822 date should be > 0, got {e4}");
        assert_eq!(e1, e4, "ISO and RFC2822 should produce same epoch");

        // Empty returns 0.
        assert_eq!(parse_date_epoch(""), 0);
    }

    #[test]
    fn test_upsert_and_get_messages() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");

        insert_sample(&conn, "INBOX", 1, "2024-01-01T10:00:00Z");
        insert_sample(&conn, "INBOX", 2, "2024-01-02T10:00:00Z");
        insert_sample(&conn, "INBOX", 3, "2024-01-03T10:00:00Z");

        let msgs = get_messages(&conn, "INBOX", 0, 10).unwrap();
        assert_eq!(msgs.len(), 3);

        // Should be date DESC: uid 3, 2, 1.
        assert_eq!(msgs[0].uid, 3);
        assert_eq!(msgs[1].uid, 2);
        assert_eq!(msgs[2].uid, 1);

        // date_epoch must be populated on the retrieved struct.
        assert_eq!(msgs[2].date_epoch, parse_date_epoch("2024-01-01T10:00:00Z"));
        assert_eq!(msgs[1].date_epoch, parse_date_epoch("2024-01-02T10:00:00Z"));
        assert_eq!(msgs[0].date_epoch, parse_date_epoch("2024-01-03T10:00:00Z"));
    }

    #[test]
    fn test_upsert_stores_caller_provided_date_epoch() {
        // upsert_message must store the epoch it is given, not re-derive it from
        // the date string. This is the contract that lets the IMAP layer inject an
        // INTERNALDATE fallback without the DB layer overwriting it.
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");

        let sentinel_epoch: i64 = 9_999_999_999;
        upsert_message(&conn, "INBOX", 1, UpsertMessageParams { message_id: Some("<x@ex>"), in_reply_to: None, references_header: None, subject: "Test", from_address: "a@ex", from_name: "A", to_json: "[]", cc_json: "[]", date: "2024-01-01T10:00:00Z", date_epoch: sentinel_epoch, flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();

        let msgs = get_messages(&conn, "INBOX", 0, 10).unwrap();
        assert_eq!(msgs[0].date_epoch, sentinel_epoch);
    }

    #[test]
    fn test_threaded_messages_same_epoch_uid_tiebreaker() {
        // When two threads share the same thread_latest_epoch the outer ORDER BY
        // must fall back to uid DESC so results are stable across fetches.
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");

        let epoch: i64 = 1_700_000_000;
        upsert_message(&conn, "INBOX", 1, UpsertMessageParams { message_id: Some("<a@ex>"), in_reply_to: None, references_header: None, subject: "First", from_address: "a@ex", from_name: "A", to_json: "[]", cc_json: "[]", date: "", date_epoch: epoch, flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();
        upsert_message(&conn, "INBOX", 2, UpsertMessageParams { message_id: Some("<b@ex>"), in_reply_to: None, references_header: None, subject: "Second", from_address: "b@ex", from_name: "B", to_json: "[]", cc_json: "[]", date: "", date_epoch: epoch, flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();

        let threads = get_threaded_messages(&conn, "INBOX", 0, 10).unwrap();
        assert_eq!(threads.len(), 2);
        // Higher uid (more recently delivered) must sort first.
        assert_eq!(threads[0].msg.uid, 2);
        assert_eq!(threads[1].msg.uid, 1);
    }

    #[test]
    fn test_pagination_no_overlap() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");

        for uid in 1..=5 {
            insert_sample(&conn, "INBOX", uid, &format!("2024-01-{:02}T10:00:00Z", uid));
        }

        let page0 = get_messages(&conn, "INBOX", 0, 2).unwrap();
        let page1 = get_messages(&conn, "INBOX", 1, 2).unwrap();
        let page2 = get_messages(&conn, "INBOX", 2, 2).unwrap();

        assert_eq!(page0.len(), 2);
        assert_eq!(page1.len(), 2);
        assert_eq!(page2.len(), 1);

        // Verify no UIDs overlap between pages.
        let uids0: Vec<u32> = page0.iter().map(|m| m.uid).collect();
        let uids1: Vec<u32> = page1.iter().map(|m| m.uid).collect();
        let uids2: Vec<u32> = page2.iter().map(|m| m.uid).collect();

        for uid in &uids0 {
            assert!(!uids1.contains(uid));
            assert!(!uids2.contains(uid));
        }
        for uid in &uids1 {
            assert!(!uids2.contains(uid));
        }
    }

    #[test]
    fn test_count_messages() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");

        assert_eq!(count_messages(&conn, "INBOX").unwrap(), 0);

        insert_sample(&conn, "INBOX", 1, "2024-01-01T10:00:00Z");
        insert_sample(&conn, "INBOX", 2, "2024-01-02T10:00:00Z");

        assert_eq!(count_messages(&conn, "INBOX").unwrap(), 2);
    }

    #[test]
    fn test_update_message_flags() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");
        insert_sample(&conn, "INBOX", 1, "2024-01-01T10:00:00Z");

        update_message_flags(&conn, "INBOX", 1, "\\Seen,\\Flagged").unwrap();

        let msgs = get_messages(&conn, "INBOX", 0, 10).unwrap();
        assert_eq!(msgs[0].flags, "\\Seen,\\Flagged");
    }

    #[test]
    fn test_cache_and_get_body_initially_none() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");
        insert_sample(&conn, "INBOX", 1, "2024-01-01T10:00:00Z");

        // Body should not be cached yet.
        let body = get_cached_body(&conn, "INBOX", 1).unwrap();
        assert!(body.is_none());
    }

    #[test]
    fn test_cache_and_get_body_after_caching() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");
        insert_sample(&conn, "INBOX", 1, "2024-01-01T10:00:00Z");

        cache_message_body(&conn, "INBOX", 1, CacheMessageBodyParams { html: Some("<h1>Hello</h1>"), text: Some("Hello"), attachments_json: Some(r#"[{"id":"0","filename":"test.pdf","content_type":"application/pdf","size":1024,"content_id":null}]"#), raw_headers: Some("From: alice@example.com"), email_theme: Some(0) })
            .unwrap();

        let body = get_cached_body(&conn, "INBOX", 1).unwrap();
        assert!(body.is_some());
        let cached = body.unwrap();
        assert_eq!(cached.html.unwrap(), "<h1>Hello</h1>");
        assert_eq!(cached.text.unwrap(), "Hello");
        assert!(cached.attachments_json.unwrap().contains("test.pdf"));
        assert_eq!(cached.raw_headers.unwrap(), "From: alice@example.com");
        assert_eq!(cached.email_theme, Some(0));
    }

    #[test]
    fn test_update_email_theme() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");
        insert_sample(&conn, "INBOX", 1, "2024-01-01T10:00:00Z");

        update_email_theme(&conn, "INBOX", 1, 2).unwrap();

        let body = get_cached_body(&conn, "INBOX", 1).unwrap();
        assert!(body.is_none());

        cache_message_body(&conn, "INBOX", 1, CacheMessageBodyParams { html: Some("<h1>Test</h1>"), text: Some("Test"), attachments_json: None, raw_headers: None, email_theme: Some(0) }).unwrap();

        update_email_theme(&conn, "INBOX", 1, 1).unwrap();

        let cached = get_cached_body(&conn, "INBOX", 1).unwrap().unwrap();
        assert_eq!(cached.email_theme, Some(1));
    }

    #[test]
    fn test_delete_message() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");
        insert_sample(&conn, "INBOX", 1, "2024-01-01T10:00:00Z");
        insert_sample(&conn, "INBOX", 2, "2024-01-02T10:00:00Z");

        delete_message(&conn, "INBOX", 1).unwrap();

        let msgs = get_messages(&conn, "INBOX", 0, 10).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].uid, 2);
    }

    #[test]
    fn test_get_thread_messages_by_message_id() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");

        // Original message.
        upsert_message(&conn, "INBOX", 1, UpsertMessageParams { message_id: Some("<thread-1@example.com>"), in_reply_to: None, references_header: None, subject: "Hello", from_address: "alice@example.com", from_name: "Alice", to_json: "[]", cc_json: "[]", date: "2024-01-01T10:00:00Z", date_epoch: 0, flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();

        // Reply referencing original via in_reply_to.
        upsert_message(&conn, "INBOX", 2, UpsertMessageParams { message_id: Some("<reply-1@example.com>"), in_reply_to: Some("<thread-1@example.com>"), references_header: None, subject: "Re: Hello", from_address: "bob@example.com", from_name: "Bob", to_json: "[]", cc_json: "[]", date: "2024-01-02T10:00:00Z", date_epoch: 0, flags_csv: "", size: 200, has_attachments: false, snippet: "", reaction: None }).unwrap();

        let thread = get_thread_messages(&conn, "<thread-1@example.com>").unwrap();
        assert_eq!(thread.len(), 2);
        // ASC order: uid 1 first, uid 2 second.
        assert_eq!(thread[0].uid, 1);
        assert_eq!(thread[1].uid, 2);
    }

    #[test]
    fn test_get_thread_messages_by_references_header() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");

        // Original message.
        upsert_message(&conn, "INBOX", 1, UpsertMessageParams { message_id: Some("<orig@example.com>"), in_reply_to: None, references_header: None, subject: "Hello", from_address: "alice@example.com", from_name: "Alice", to_json: "[]", cc_json: "[]", date: "2024-01-01T10:00:00Z", date_epoch: 0, flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();

        // A message that references the original only via references_header.
        upsert_message(&conn, "INBOX", 3, UpsertMessageParams { message_id: Some("<deep-reply@example.com>"), in_reply_to: Some("<mid@example.com>"), references_header: Some("<orig@example.com> <mid@example.com>"), subject: "Re: Re: Hello", from_address: "carol@example.com", from_name: "Carol", to_json: "[]", cc_json: "[]", date: "2024-01-03T10:00:00Z", date_epoch: 0, flags_csv: "", size: 300, has_attachments: false, snippet: "", reaction: None }).unwrap();

        let thread = get_thread_messages(&conn, "<orig@example.com>").unwrap();
        assert_eq!(thread.len(), 2);
        assert_eq!(thread[0].uid, 1); // matched by message_id
        assert_eq!(thread[1].uid, 3); // matched by references_header LIKE
    }

    #[test]
    fn test_get_full_thread_walks_references_chain() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");

        // Message 1: original
        upsert_message(&conn, "INBOX", 1, UpsertMessageParams { message_id: Some("<a@ex>"), in_reply_to: None, references_header: None, subject: "Hello", from_address: "alice@ex", from_name: "Alice", to_json: "[]", cc_json: "[]", date: "2024-01-01T10:00:00Z", date_epoch: parse_date_epoch("2024-01-01T10:00:00Z"), flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();
        // Message 2: reply to 1
        upsert_message(&conn, "INBOX", 2, UpsertMessageParams { message_id: Some("<b@ex>"), in_reply_to: Some("<a@ex>"), references_header: Some("<a@ex>"), subject: "Re: Hello", from_address: "bob@ex", from_name: "Bob", to_json: "[]", cc_json: "[]", date: "2024-01-02T10:00:00Z", date_epoch: parse_date_epoch("2024-01-02T10:00:00Z"), flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();
        // Message 3: reply to 2, references both
        upsert_message(&conn, "INBOX", 3, UpsertMessageParams { message_id: Some("<c@ex>"), in_reply_to: Some("<b@ex>"), references_header: Some("<a@ex> <b@ex>"), subject: "Re: Re: Hello", from_address: "carol@ex", from_name: "Carol", to_json: "[]", cc_json: "[]", date: "2024-01-03T10:00:00Z", date_epoch: parse_date_epoch("2024-01-03T10:00:00Z"), flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();

        // Starting from message 2, should find the entire thread (1, 2, 3).
        let thread = get_full_thread(&conn, "<b@ex>", Some("<a@ex>")).unwrap();
        assert_eq!(thread.len(), 3);
        assert_eq!(thread[0].uid, 1);
        assert_eq!(thread[1].uid, 2);
        assert_eq!(thread[2].uid, 3);
    }

    #[test]
    fn test_get_full_thread_from_leaf_message() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");

        upsert_message(&conn, "INBOX", 1, UpsertMessageParams { message_id: Some("<root@ex>"), in_reply_to: None, references_header: None, subject: "Start", from_address: "alice@ex", from_name: "Alice", to_json: "[]", cc_json: "[]", date: "2024-01-01T10:00:00Z", date_epoch: 0, flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();
        upsert_message(&conn, "INBOX", 2, UpsertMessageParams { message_id: Some("<mid@ex>"), in_reply_to: Some("<root@ex>"), references_header: Some("<root@ex>"), subject: "Re: Start", from_address: "bob@ex", from_name: "Bob", to_json: "[]", cc_json: "[]", date: "2024-01-02T10:00:00Z", date_epoch: 0, flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();
        upsert_message(&conn, "INBOX", 3, UpsertMessageParams { message_id: Some("<leaf@ex>"), in_reply_to: Some("<mid@ex>"), references_header: Some("<root@ex> <mid@ex>"), subject: "Re: Re: Start", from_address: "carol@ex", from_name: "Carol", to_json: "[]", cc_json: "[]", date: "2024-01-03T10:00:00Z", date_epoch: 0, flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();

        // Starting from the leaf message, should still find entire thread.
        let thread = get_full_thread(&conn, "<leaf@ex>", Some("<root@ex> <mid@ex>")).unwrap();
        assert_eq!(thread.len(), 3);
    }

    #[test]
    fn test_get_full_thread_single_message() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");

        upsert_message(&conn, "INBOX", 1, UpsertMessageParams { message_id: Some("<solo@ex>"), in_reply_to: None, references_header: None, subject: "Solo", from_address: "alice@ex", from_name: "Alice", to_json: "[]", cc_json: "[]", date: "2024-01-01T10:00:00Z", date_epoch: 0, flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();

        let thread = get_full_thread(&conn, "<solo@ex>", None).unwrap();
        assert_eq!(thread.len(), 1);
        assert_eq!(thread[0].uid, 1);
    }

    #[test]
    fn test_get_messages_same_epoch_uid_tiebreaker() {
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");
        let epoch: i64 = 1_700_000_000;
        upsert_message(&conn, "INBOX", 1, UpsertMessageParams { message_id: Some("<a@ex>"), in_reply_to: None, references_header: None, subject: "First", from_address: "a@ex", from_name: "A", to_json: "[]", cc_json: "[]", date: "", date_epoch: epoch, flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();
        upsert_message(&conn, "INBOX", 2, UpsertMessageParams { message_id: Some("<b@ex>"), in_reply_to: None, references_header: None, subject: "Second", from_address: "b@ex", from_name: "B", to_json: "[]", cc_json: "[]", date: "", date_epoch: epoch, flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();
        let msgs = get_messages(&conn, "INBOX", 0, 10).unwrap();
        assert_eq!(msgs[0].uid, 2);
        assert_eq!(msgs[1].uid, 1);
    }

    #[test]
    fn test_get_full_thread_uses_stored_epoch_not_date_string() {
        // Simulates a message whose Date header was unparseable but INTERNALDATE
        // provided a valid epoch. The stored date_epoch must drive sort order.
        let conn = open_test_db();
        ensure_folder(&conn, "INBOX");
        let good_epoch = parse_date_epoch("2024-06-01T00:00:00Z");
        upsert_message(&conn, "INBOX", 1, UpsertMessageParams { message_id: Some("<root@ex>"), in_reply_to: None, references_header: None, subject: "Root", from_address: "a@ex", from_name: "A", to_json: "[]", cc_json: "[]", date: "INVALID DATE", date_epoch: good_epoch, flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();
        upsert_message(&conn, "INBOX", 2, UpsertMessageParams { message_id: Some("<reply@ex>"), in_reply_to: Some("<root@ex>"), references_header: Some("<root@ex>"), subject: "Reply", from_address: "b@ex", from_name: "B", to_json: "[]", cc_json: "[]", date: "INVALID DATE", date_epoch: good_epoch + 3600, flags_csv: "", size: 100, has_attachments: false, snippet: "", reaction: None }).unwrap();
        let thread = get_full_thread(&conn, "<reply@ex>", Some("<root@ex>")).unwrap();
        assert_eq!(thread.len(), 2);
        assert_eq!(thread[0].uid, 1); // root first (lower epoch)
        assert_eq!(thread[1].uid, 2); // reply second
    }
}
