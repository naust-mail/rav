use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

/// A queued send. IMAP/SMTP credentials are never stored here — the
/// worker that drains this table holds them in memory, keyed by user.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct OutboxEntry {
    pub id: String,
    pub draft_id: Option<String>,
    pub to_addrs: Vec<String>,
    pub cc_addrs: Vec<String>,
    pub bcc_addrs: Vec<String>,
    pub subject: String,
    pub text_body: String,
    pub html_body: Option<String>,
    pub in_reply_to: Option<String>,
    pub references_hdr: Option<String>,
    pub from_identity_id: Option<i64>,
    /// JSON-encoded `PgpSendRequest`, if the message was queued with PGP params.
    pub pgp_json: Option<String>,
    pub state: String,
    pub fail_reason: Option<String>,
    pub attempt_count: i64,
    pub created_at: String,
    pub send_after: String,
}

/// Fields needed to enqueue a new send. `id` and `send_after` are supplied
/// by the caller (route handler) since they depend on a fresh UUID and the
/// user's `undo_send_delay` preference.
pub struct NewOutboxEntry<'a> {
    pub id: &'a str,
    pub draft_id: Option<&'a str>,
    pub to_addrs: &'a [String],
    pub cc_addrs: &'a [String],
    pub bcc_addrs: &'a [String],
    pub subject: &'a str,
    pub text_body: &'a str,
    pub html_body: Option<&'a str>,
    pub in_reply_to: Option<&'a str>,
    pub references_hdr: Option<&'a str>,
    pub from_identity_id: Option<i64>,
    pub pgp_json: Option<&'a str>,
    pub send_after: &'a str,
}

const SELECT_COLUMNS: &str = "id, draft_id, to_addrs, cc_addrs, bcc_addrs, subject, text_body, \
     html_body, in_reply_to, references_hdr, from_identity_id, pgp_json, state, fail_reason, \
     attempt_count, created_at, send_after";

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<OutboxEntry> {
    let to_json: String = row.get(2)?;
    let cc_json: String = row.get(3)?;
    let bcc_json: String = row.get(4)?;
    Ok(OutboxEntry {
        id: row.get(0)?,
        draft_id: row.get(1)?,
        to_addrs: serde_json::from_str(&to_json).unwrap_or_default(),
        cc_addrs: serde_json::from_str(&cc_json).unwrap_or_default(),
        bcc_addrs: serde_json::from_str(&bcc_json).unwrap_or_default(),
        subject: row.get(5)?,
        text_body: row.get(6)?,
        html_body: row.get(7)?,
        in_reply_to: row.get(8)?,
        references_hdr: row.get(9)?,
        from_identity_id: row.get(10)?,
        pgp_json: row.get(11)?,
        state: row.get(12)?,
        fail_reason: row.get(13)?,
        attempt_count: row.get(14)?,
        created_at: row.get(15)?,
        send_after: row.get(16)?,
    })
}

/// Insert a new outbox entry in the `scheduled` state.
pub fn enqueue(conn: &Connection, entry: &NewOutboxEntry) -> Result<OutboxEntry, String> {
    let to_json = serde_json::to_string(entry.to_addrs).map_err(|e| e.to_string())?;
    let cc_json = serde_json::to_string(entry.cc_addrs).map_err(|e| e.to_string())?;
    let bcc_json = serde_json::to_string(entry.bcc_addrs).map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO outbox (id, draft_id, to_addrs, cc_addrs, bcc_addrs, subject, text_body,
                              html_body, in_reply_to, references_hdr, from_identity_id, pgp_json,
                              send_after)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            entry.id,
            entry.draft_id,
            to_json,
            cc_json,
            bcc_json,
            entry.subject,
            entry.text_body,
            entry.html_body,
            entry.in_reply_to,
            entry.references_hdr,
            entry.from_identity_id,
            entry.pgp_json,
            entry.send_after,
        ],
    )
    .map_err(|e| format!("Failed to enqueue outbox entry: {e}"))?;

    get(conn, entry.id)?.ok_or_else(|| "Outbox entry vanished immediately after insert".to_string())
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<OutboxEntry>, String> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM outbox WHERE id = ?1"),
        params![id],
        row_to_entry,
    )
    .optional()
    .map_err(|e| format!("Failed to load outbox entry: {e}"))
}

/// List entries visible in the Outbox UI: everything still scheduled
/// (including ones mid-retry) or permanently failed. Sent entries are
/// deleted on success and never appear here.
pub fn list_visible(conn: &Connection) -> Result<Vec<OutboxEntry>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SELECT_COLUMNS} FROM outbox WHERE state IN ('scheduled', 'sending', 'failed') \
             ORDER BY created_at ASC"
        ))
        .map_err(|e| format!("Failed to prepare outbox list query: {e}"))?;
    let rows = stmt
        .query_map([], row_to_entry)
        .map_err(|e| format!("Failed to list outbox entries: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read outbox entries: {e}"))
}

/// Entries due for a send attempt right now.
pub fn list_due(conn: &Connection, now: &str) -> Result<Vec<OutboxEntry>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SELECT_COLUMNS} FROM outbox WHERE state = 'scheduled' AND send_after <= ?1 \
             ORDER BY send_after ASC"
        ))
        .map_err(|e| format!("Failed to prepare due-entries query: {e}"))?;
    let rows = stmt
        .query_map(params![now], row_to_entry)
        .map_err(|e| format!("Failed to list due outbox entries: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read due outbox entries: {e}"))
}

/// Earliest `send_after` among still-scheduled entries, so the worker knows
/// how long it can sleep before the next one comes due.
pub fn next_send_after(conn: &Connection) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT MIN(send_after) FROM outbox WHERE state = 'scheduled'",
        [],
        |row| row.get(0),
    )
    .map_err(|e| format!("Failed to query next send_after: {e}"))
}

pub fn mark_sending(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("UPDATE outbox SET state = 'sending' WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to mark outbox entry sending: {e}"))?;
    Ok(())
}

/// Transient failure: bump the attempt count and reschedule for `next_send_after`,
/// staying in `scheduled` so a restart naturally resumes it.
pub fn mark_retry(conn: &Connection, id: &str, next_send_after: &str, reason: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE outbox SET state = 'scheduled', send_after = ?2, attempt_count = attempt_count + 1,
                            fail_reason = ?3
         WHERE id = ?1",
        params![id, next_send_after, reason],
    )
    .map_err(|e| format!("Failed to reschedule outbox entry: {e}"))?;
    Ok(())
}

pub fn mark_failed(conn: &Connection, id: &str, reason: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE outbox SET state = 'failed', fail_reason = ?2 WHERE id = ?1",
        params![id, reason],
    )
    .map_err(|e| format!("Failed to mark outbox entry failed: {e}"))?;
    Ok(())
}

/// Delete an entry outright — used both for undo/cancel (state must be
/// `scheduled`, checked by the caller before calling this) and for cleanup
/// after a successful send.
pub fn delete(conn: &Connection, id: &str) -> Result<bool, String> {
    let changed = conn
        .execute("DELETE FROM outbox WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to delete outbox entry: {e}"))?;
    Ok(changed > 0)
}

/// User-triggered retry of a permanently failed entry: reset it back to
/// `scheduled` with a fresh attempt count so the backoff schedule restarts.
pub fn requeue(conn: &Connection, id: &str, send_after: &str) -> Result<bool, String> {
    let changed = conn
        .execute(
            "UPDATE outbox SET state = 'scheduled', send_after = ?2, attempt_count = 0, fail_reason = NULL
             WHERE id = ?1 AND state = 'failed'",
            params![id, send_after],
        )
        .map_err(|e| format!("Failed to requeue outbox entry: {e}"))?;
    Ok(changed > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::open_test_db;

    fn sample_entry<'a>(id: &'a str, to: &'a [String]) -> NewOutboxEntry<'a> {
        NewOutboxEntry {
            id,
            draft_id: None,
            to_addrs: to,
            cc_addrs: &[],
            bcc_addrs: &[],
            subject: "Hello",
            text_body: "Body",
            html_body: None,
            in_reply_to: None,
            references_hdr: None,
            from_identity_id: None,
            pgp_json: None,
            send_after: "2026-01-01T00:00:05Z",
        }
    }

    #[test]
    fn enqueue_and_get_round_trip() {
        let conn = open_test_db();
        let to = vec!["bob@example.com".to_string()];
        let entry = enqueue(&conn, &sample_entry("id-1", &to)).unwrap();

        assert_eq!(entry.id, "id-1");
        assert_eq!(entry.to_addrs, to);
        assert_eq!(entry.state, "scheduled");
        assert_eq!(entry.attempt_count, 0);

        let fetched = get(&conn, "id-1").unwrap().unwrap();
        assert_eq!(fetched.subject, "Hello");
    }

    #[test]
    fn list_due_respects_send_after() {
        let conn = open_test_db();
        let to = vec!["bob@example.com".to_string()];
        enqueue(&conn, &sample_entry("id-1", &to)).unwrap();

        assert!(list_due(&conn, "2026-01-01T00:00:00Z").unwrap().is_empty());
        assert_eq!(list_due(&conn, "2026-01-01T00:00:10Z").unwrap().len(), 1);
    }

    #[test]
    fn mark_retry_reschedules_and_bumps_attempt_count() {
        let conn = open_test_db();
        let to = vec!["bob@example.com".to_string()];
        enqueue(&conn, &sample_entry("id-1", &to)).unwrap();

        mark_retry(&conn, "id-1", "2026-01-01T01:00:00Z", "smtp timeout").unwrap();

        let entry = get(&conn, "id-1").unwrap().unwrap();
        assert_eq!(entry.state, "scheduled");
        assert_eq!(entry.attempt_count, 1);
        assert_eq!(entry.send_after, "2026-01-01T01:00:00Z");
        assert_eq!(entry.fail_reason.as_deref(), Some("smtp timeout"));
    }

    #[test]
    fn mark_failed_sets_state_and_reason() {
        let conn = open_test_db();
        let to = vec!["bob@example.com".to_string()];
        enqueue(&conn, &sample_entry("id-1", &to)).unwrap();

        mark_failed(&conn, "id-1", "invalid recipient").unwrap();

        let entry = get(&conn, "id-1").unwrap().unwrap();
        assert_eq!(entry.state, "failed");
        assert_eq!(entry.fail_reason.as_deref(), Some("invalid recipient"));
    }

    #[test]
    fn delete_removes_entry() {
        let conn = open_test_db();
        let to = vec!["bob@example.com".to_string()];
        enqueue(&conn, &sample_entry("id-1", &to)).unwrap();

        assert!(delete(&conn, "id-1").unwrap());
        assert!(get(&conn, "id-1").unwrap().is_none());
        assert!(!delete(&conn, "id-1").unwrap());
    }

    #[test]
    fn requeue_only_affects_failed_entries() {
        let conn = open_test_db();
        let to = vec!["bob@example.com".to_string()];
        enqueue(&conn, &sample_entry("id-1", &to)).unwrap();

        // Not failed yet — requeue should be a no-op.
        assert!(!requeue(&conn, "id-1", "2026-01-01T02:00:00Z").unwrap());

        mark_failed(&conn, "id-1", "boom").unwrap();
        mark_retry(&conn, "id-1", "2026-01-01T01:00:00Z", "boom").unwrap(); // bump attempt_count via retry path first
        mark_failed(&conn, "id-1", "boom again").unwrap();

        assert!(requeue(&conn, "id-1", "2026-01-01T02:00:00Z").unwrap());
        let entry = get(&conn, "id-1").unwrap().unwrap();
        assert_eq!(entry.state, "scheduled");
        assert_eq!(entry.attempt_count, 0);
        assert!(entry.fail_reason.is_none());
        assert_eq!(entry.send_after, "2026-01-01T02:00:00Z");
    }

    #[test]
    fn list_visible_excludes_nothing_but_sent() {
        let conn = open_test_db();
        let to = vec!["bob@example.com".to_string()];
        enqueue(&conn, &sample_entry("id-1", &to)).unwrap();
        enqueue(&conn, &sample_entry("id-2", &to)).unwrap();
        mark_failed(&conn, "id-2", "boom").unwrap();

        let visible = list_visible(&conn).unwrap();
        assert_eq!(visible.len(), 2);
    }
}
