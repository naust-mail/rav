use rusqlite::{Connection, params};
use serde::Serialize;

/// A sticker assigned to a specific calendar date.
#[derive(Debug, Clone, Serialize)]
pub struct CalendarSticker {
    /// ISO date: YYYY-MM-DD
    pub date: String,
    pub sticker_id: String,
    pub updated_at: String,
}

fn row_to_sticker(row: &rusqlite::Row<'_>) -> rusqlite::Result<CalendarSticker> {
    Ok(CalendarSticker {
        date: row.get(0)?,
        sticker_id: row.get(1)?,
        updated_at: row.get(2)?,
    })
}

/// List all sticker assignments in an inclusive date range (YYYY-MM-DD strings).
pub fn list_stickers(
    conn: &Connection,
    from: &str,
    to: &str,
) -> Result<Vec<CalendarSticker>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT date, sticker_id, updated_at FROM calendar_stickers
             WHERE date >= ?1 AND date <= ?2 ORDER BY date ASC",
        )
        .map_err(|e| format!("Failed to prepare list_stickers: {e}"))?;
    let rows = stmt
        .query_map(params![from, to], row_to_sticker)
        .map_err(|e| format!("Failed to query stickers: {e}"))?;
    rows.map(|r| r.map_err(|e| format!("Failed to read sticker row: {e}")))
        .collect()
}

/// Upsert a sticker assignment for a date.
pub fn put_sticker(
    conn: &Connection,
    date: &str,
    sticker_id: &str,
) -> Result<CalendarSticker, String> {
    conn.execute(
        "INSERT INTO calendar_stickers (date, sticker_id, updated_at)
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
         ON CONFLICT(date) DO UPDATE SET sticker_id = excluded.sticker_id,
                                         updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
        params![date, sticker_id],
    )
    .map_err(|e| format!("Failed to upsert sticker: {e}"))?;

    conn.query_row(
        "SELECT date, sticker_id, updated_at FROM calendar_stickers WHERE date = ?1",
        params![date],
        row_to_sticker,
    )
    .map_err(|e| format!("Failed to read sticker after upsert: {e}"))
}

/// Remove a sticker assignment. Returns true if a row was deleted.
pub fn delete_sticker(conn: &Connection, date: &str) -> Result<bool, String> {
    let deleted = conn
        .execute(
            "DELETE FROM calendar_stickers WHERE date = ?1",
            params![date],
        )
        .map_err(|e| format!("Failed to delete sticker: {e}"))?;
    Ok(deleted > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::open_test_db;

    #[test]
    fn test_put_and_list() {
        let conn = open_test_db();
        put_sticker(&conn, "2025-07-01", "happy").unwrap();
        put_sticker(&conn, "2025-07-04", "fireworks").unwrap();

        let all = list_stickers(&conn, "2025-07-01", "2025-07-31").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].sticker_id, "happy");
        assert_eq!(all[1].sticker_id, "fireworks");
    }

    #[test]
    fn test_upsert_replaces() {
        let conn = open_test_db();
        put_sticker(&conn, "2025-07-01", "happy").unwrap();
        put_sticker(&conn, "2025-07-01", "sleepy").unwrap();

        let all = list_stickers(&conn, "2025-07-01", "2025-07-01").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].sticker_id, "sleepy");
    }

    #[test]
    fn test_delete() {
        let conn = open_test_db();
        put_sticker(&conn, "2025-07-01", "happy").unwrap();
        assert!(delete_sticker(&conn, "2025-07-01").unwrap());
        assert!(!delete_sticker(&conn, "2025-07-01").unwrap());
        assert!(list_stickers(&conn, "2025-07-01", "2025-07-01").unwrap().is_empty());
    }

    #[test]
    fn test_range_filter() {
        let conn = open_test_db();
        put_sticker(&conn, "2025-06-30", "before").unwrap();
        put_sticker(&conn, "2025-07-15", "inside").unwrap();
        put_sticker(&conn, "2025-08-01", "after").unwrap();

        let july = list_stickers(&conn, "2025-07-01", "2025-07-31").unwrap();
        assert_eq!(july.len(), 1);
        assert_eq!(july[0].sticker_id, "inside");
    }
}
