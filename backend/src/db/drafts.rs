use rusqlite::{Connection, params};

/// Maps a client-generated draft UUID to local state that IMAP doesn't store.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DraftStaging {
    /// Client-generated UUID, stable across saves.
    pub uuid: String,
    /// IMAP UID of the current draft copy in the Drafts folder.
    pub imap_uid: Option<u32>,
    /// Message-ID of the message being replied to. Stable across folder moves;
    /// resolved to a UID+folder via the local messages cache on reopen.
    pub reply_message_id: Option<String>,
}

/// An attachment file staged for a draft.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DraftAttachment {
    pub id: String,
    pub draft_uuid: String,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    pub file_path: String,
    pub created_at: String,
}

/// Insert or update the staging record for a draft UUID.
/// `reply_message_id` is only set on first save; subsequent saves leave it unchanged.
pub fn upsert_staging(
    conn: &Connection,
    uuid: &str,
    imap_uid: Option<u32>,
    reply_message_id: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO draft_staging (uuid, imap_uid, reply_message_id) VALUES (?1, ?2, ?3)
         ON CONFLICT(uuid) DO UPDATE SET
             imap_uid = excluded.imap_uid,
             reply_message_id = COALESCE(draft_staging.reply_message_id, excluded.reply_message_id)",
        params![uuid, imap_uid, reply_message_id],
    )
    .map_err(|e| format!("Failed to upsert draft_staging: {e}"))?;
    Ok(())
}

/// Retrieve a staging record by UUID.
pub fn get_staging(conn: &Connection, uuid: &str) -> Result<Option<DraftStaging>, String> {
    let result = conn.query_row(
        "SELECT uuid, imap_uid, reply_message_id FROM draft_staging WHERE uuid = ?1",
        params![uuid],
        |row| Ok(DraftStaging {
            uuid: row.get(0)?,
            imap_uid: row.get(1)?,
            reply_message_id: row.get(2)?,
        }),
    );
    match result {
        Ok(s) => Ok(Some(s)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to get draft_staging: {e}")),
    }
}

/// Find a staging record by the Message-ID of the message being replied to.
pub fn find_by_reply_message_id(conn: &Connection, reply_message_id: &str) -> Result<Option<DraftStaging>, String> {
    let result = conn.query_row(
        "SELECT uuid, imap_uid, reply_message_id FROM draft_staging WHERE reply_message_id = ?1",
        params![reply_message_id],
        |row| Ok(DraftStaging {
            uuid: row.get(0)?,
            imap_uid: row.get(1)?,
            reply_message_id: row.get(2)?,
        }),
    );
    match result {
        Ok(s) => Ok(Some(s)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to find draft_staging by reply_message_id: {e}")),
    }
}

/// Delete a staging record. Cascades to draft_attachments.
pub fn delete_staging(conn: &Connection, uuid: &str) -> Result<bool, String> {
    let n = conn
        .execute("DELETE FROM draft_staging WHERE uuid = ?1", params![uuid])
        .map_err(|e| format!("Failed to delete draft_staging: {e}"))?;
    Ok(n > 0)
}

/// Add an attachment record for a draft.
pub fn add_draft_attachment(
    conn: &Connection,
    id: &str,
    draft_uuid: &str,
    filename: &str,
    content_type: &str,
    size: i64,
    file_path: &str,
) -> Result<(), String> {
    // Ensure staging row exists so the FK doesn't fail.
    upsert_staging(conn, draft_uuid, None, None)?;
    conn.execute(
        "INSERT INTO draft_attachments (id, draft_uuid, filename, content_type, size, file_path)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, draft_uuid, filename, content_type, size, file_path],
    )
    .map_err(|e| format!("Failed to add draft attachment: {e}"))?;
    Ok(())
}

/// Get all attachments for a draft UUID.
pub fn get_draft_attachments(conn: &Connection, draft_uuid: &str) -> Result<Vec<DraftAttachment>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, draft_uuid, filename, content_type, size, file_path, created_at
             FROM draft_attachments WHERE draft_uuid = ?1 ORDER BY created_at ASC",
        )
        .map_err(|e| format!("Failed to prepare get_draft_attachments: {e}"))?;

    let rows = stmt
        .query_map(params![draft_uuid], |row| {
            Ok(DraftAttachment {
                id: row.get(0)?,
                draft_uuid: row.get(1)?,
                filename: row.get(2)?,
                content_type: row.get(3)?,
                size: row.get(4)?,
                file_path: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(|e| format!("Failed to query draft attachments: {e}"))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("Failed to read attachment row: {e}"))?);
    }
    Ok(out)
}

/// Delete a single attachment by ID.
pub fn delete_draft_attachment(conn: &Connection, id: &str) -> Result<bool, String> {
    let n = conn
        .execute("DELETE FROM draft_attachments WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to delete draft attachment: {e}"))?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::open_test_db;

    #[test]
    fn test_upsert_and_get_staging() {
        let conn = open_test_db();
        upsert_staging(&conn, "uuid-1", None, None).unwrap();
        let s = get_staging(&conn, "uuid-1").unwrap().unwrap();
        assert_eq!(s.uuid, "uuid-1");
        assert!(s.imap_uid.is_none());

        upsert_staging(&conn, "uuid-1", Some(42), None).unwrap();
        let s = get_staging(&conn, "uuid-1").unwrap().unwrap();
        assert_eq!(s.imap_uid, Some(42));
    }

    #[test]
    fn test_delete_staging() {
        let conn = open_test_db();
        upsert_staging(&conn, "uuid-1", Some(1), None).unwrap();
        assert!(delete_staging(&conn, "uuid-1").unwrap());
        assert!(get_staging(&conn, "uuid-1").unwrap().is_none());
        assert!(!delete_staging(&conn, "uuid-1").unwrap());
    }

    #[test]
    fn test_attachments_cascade_on_staging_delete() {
        let conn = open_test_db();
        add_draft_attachment(&conn, "att-1", "uuid-1", "file.pdf", "application/pdf", 100, "/path").unwrap();
        assert_eq!(get_draft_attachments(&conn, "uuid-1").unwrap().len(), 1);
        delete_staging(&conn, "uuid-1").unwrap();
        assert!(get_draft_attachments(&conn, "uuid-1").unwrap().is_empty());
    }

    #[test]
    fn test_add_attachment_auto_creates_staging() {
        let conn = open_test_db();
        assert!(get_staging(&conn, "uuid-2").unwrap().is_none());
        add_draft_attachment(&conn, "att-1", "uuid-2", "f.txt", "text/plain", 10, "/p").unwrap();
        assert!(get_staging(&conn, "uuid-2").unwrap().is_some());
    }
}
