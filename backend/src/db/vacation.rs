use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// Vacation responder settings (singleton per user).
#[derive(Debug, Clone, Serialize)]
pub struct VacationResponder {
    pub enabled: bool,
    pub subject: String,
    pub body: String,
    /// ISO date string (YYYY-MM-DD), or null for no start constraint.
    pub start_date: Option<String>,
    /// ISO date string (YYYY-MM-DD), or null for no end constraint.
    pub end_date: Option<String>,
    /// Minimum hours between replies to the same sender.
    pub reply_interval_hours: i64,
}

#[derive(Debug, Deserialize)]
pub struct UpdateVacationResponder {
    pub enabled: Option<bool>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub start_date: Option<Option<String>>,
    pub end_date: Option<Option<String>>,
    pub reply_interval_hours: Option<i64>,
}

fn default_vacation() -> VacationResponder {
    VacationResponder {
        enabled: false,
        subject: "Out of office".to_string(),
        body: String::new(),
        start_date: None,
        end_date: None,
        reply_interval_hours: 24,
    }
}

pub fn get_vacation(conn: &Connection) -> Result<VacationResponder, String> {
    let result = conn.query_row(
        "SELECT enabled, subject, body, start_date, end_date, reply_interval_hours
         FROM vacation_responder WHERE id = 1",
        [],
        |row| {
            let enabled_int: i32 = row.get(0)?;
            Ok(VacationResponder {
                enabled: enabled_int != 0,
                subject: row.get(1)?,
                body: row.get(2)?,
                start_date: row.get(3)?,
                end_date: row.get(4)?,
                reply_interval_hours: row.get(5)?,
            })
        },
    );

    match result {
        Ok(v) => Ok(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(default_vacation()),
        Err(e) => Err(format!("Failed to get vacation responder: {e}")),
    }
}

pub fn update_vacation(
    conn: &Connection,
    data: &UpdateVacationResponder,
) -> Result<VacationResponder, String> {
    conn.execute(
        "INSERT OR IGNORE INTO vacation_responder (id) VALUES (1)",
        [],
    )
    .map_err(|e| format!("Failed to ensure vacation row: {e}"))?;

    let mut sets: Vec<String> = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1usize;

    macro_rules! push {
        ($col:expr, $val:expr) => {{
            sets.push(format!("{} = ?{idx}", $col));
            values.push(Box::new($val));
            idx += 1;
        }};
    }

    if let Some(enabled) = data.enabled {
        push!("enabled", enabled as i32);
    }
    if let Some(ref s) = data.subject {
        push!("subject", s.clone());
    }
    if let Some(ref b) = data.body {
        push!("body", b.clone());
    }
    if let Some(ref sd) = data.start_date {
        push!("start_date", sd.clone());
    }
    if let Some(ref ed) = data.end_date {
        push!("end_date", ed.clone());
    }
    if let Some(h) = data.reply_interval_hours {
        if h < 1 {
            return Err("reply_interval_hours must be >= 1".to_string());
        }
        push!("reply_interval_hours", h);
    }

    if !sets.is_empty() {
        sets.push("updated_at = datetime('now')".to_string());
        let sql = format!(
            "UPDATE vacation_responder SET {} WHERE id = ?{idx}",
            sets.join(", ")
        );
        values.push(Box::new(1i32));
        let refs: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        conn.execute(&sql, refs.as_slice())
            .map_err(|e| format!("Failed to update vacation responder: {e}"))?;
    }

    get_vacation(conn)
}

/// Returns true if a vacation reply should be sent to this sender right now.
/// Records the reply timestamp if returning true.
pub fn should_reply_and_record(
    conn: &Connection,
    sender_email: &str,
    interval_hours: i64,
) -> Result<bool, String> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT replied_at FROM vacation_replies WHERE sender_email = ?1",
            params![sender_email],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Failed to query vacation_replies: {e}"))?;

    if let Some(replied_at) = existing {
        // Check if enough time has passed.
        let elapsed: i64 = conn
            .query_row(
                "SELECT CAST((julianday('now') - julianday(?1)) * 24 AS INTEGER)",
                params![replied_at],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to compute elapsed time: {e}"))?;
        if elapsed < interval_hours {
            return Ok(false);
        }
        // Update timestamp.
        conn.execute(
            "UPDATE vacation_replies SET replied_at = datetime('now') WHERE sender_email = ?1",
            params![sender_email],
        )
        .map_err(|e| format!("Failed to update vacation_replies: {e}"))?;
    } else {
        conn.execute(
            "INSERT INTO vacation_replies (sender_email) VALUES (?1)",
            params![sender_email],
        )
        .map_err(|e| format!("Failed to insert vacation_reply: {e}"))?;
    }

    Ok(true)
}

/// Purge stale vacation reply records older than interval_hours, so the
/// table doesn't grow unbounded when vacation is toggled on/off repeatedly.
pub fn purge_old_replies(conn: &Connection, interval_hours: i64) -> Result<(), String> {
    conn.execute(
        "DELETE FROM vacation_replies
         WHERE CAST((julianday('now') - julianday(replied_at)) * 24 AS INTEGER) > ?1",
        params![interval_hours * 2],
    )
    .map_err(|e| format!("Failed to purge vacation_replies: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::open_test_db;

    #[test]
    fn test_get_default_vacation() {
        let conn = open_test_db();
        let v = get_vacation(&conn).unwrap();
        assert!(!v.enabled);
        assert_eq!(v.reply_interval_hours, 24);
    }

    #[test]
    fn test_update_vacation_enabled() {
        let conn = open_test_db();
        let v = update_vacation(
            &conn,
            &UpdateVacationResponder {
                enabled: Some(true),
                subject: Some("Away".to_string()),
                body: Some("I am away.".to_string()),
                start_date: None,
                end_date: None,
                reply_interval_hours: None,
            },
        )
        .unwrap();
        assert!(v.enabled);
        assert_eq!(v.subject, "Away");
        assert_eq!(v.body, "I am away.");
    }

    #[test]
    fn test_should_reply_and_record() {
        let conn = open_test_db();
        // First reply should succeed.
        assert!(should_reply_and_record(&conn, "sender@test.com", 24).unwrap());
        // Second reply within interval should be suppressed.
        assert!(!should_reply_and_record(&conn, "sender@test.com", 24).unwrap());
    }
}
