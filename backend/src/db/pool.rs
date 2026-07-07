use std::fs;
use std::path::Path;

use rusqlite::Connection;

/// Opens the per-user SQLite database at `{data_dir}/{user_hash}/db.sqlite`.
///
/// Creates the directory tree if it doesn't exist, enables WAL journal mode
/// and foreign key enforcement. Does NOT run migrations - call
/// `auth::user_data::provision_user_data` first to ensure the schema is ready.
pub fn open_user_db(data_dir: &str, user_hash: &str) -> Result<Connection, String> {
    let dir = Path::new(data_dir).join(user_hash);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create db dir: {e}"))?;

    let db_path = dir.join("db.sqlite");
    let conn =
        Connection::open(&db_path).map_err(|e| format!("Failed to open SQLite: {e}"))?;

    // Enable WAL mode for better concurrent read performance.
    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(|e| format!("Failed to set WAL mode: {e}"))?;

    // Enable foreign key constraint enforcement.
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("Failed to enable foreign keys: {e}"))?;

    Ok(conn)
}

#[cfg(test)]
const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("../../migrations/V001__initial_schema.sql")),
    (2, include_str!("../../migrations/V002__folders_and_messages.sql")),
    (3, include_str!("../../migrations/V003__add_date_epoch.sql")),
    (4, include_str!("../../migrations/V004__folder_messages_updated_at.sql")),
    (5, include_str!("../../migrations/V005__drafts_and_attachments.sql")),
    (6, include_str!("../../migrations/V006__cache_attachments_and_headers.sql")),
    (7, include_str!("../../migrations/V007__contacts.sql")),
    (8, include_str!("../../migrations/V008__identities.sql")),
    (9, include_str!("../../migrations/V009__notification_preferences.sql")),
    (10, include_str!("../../migrations/V010__contact_groups.sql")),
    (11, include_str!("../../migrations/V011__message_reaction.sql")),
    (12, include_str!("../../migrations/V012__tags.sql")),
    (13, include_str!("../../migrations/V013__display_preferences.sql")),
    (14, include_str!("../../migrations/V014__compose_format.sql")),
    (15, include_str!("../../migrations/V015__deep_index.sql")),
    (16, include_str!("../../migrations/V016__calendar.sql")),
    (17, include_str!("../../migrations/V017__add_email_theme.sql")),
    (18, include_str!("../../migrations/V018__animation_mode.sql")),
    (19, include_str!("../../migrations/V019__known_addresses.sql")),
    (20, include_str!("../../migrations/V020__mobile_nav_prefs.sql")),
    (21, include_str!("../../migrations/V021__thread_id.sql")),
    (22, include_str!("../../migrations/V022__undo_send_delay.sql")),
    (23, include_str!("../../migrations/V023__vacation_responder.sql")),
    (24, include_str!("../../migrations/V024__filter_rules.sql")),
    (25, include_str!("../../migrations/V025__mfa.sql")),
    (26, include_str!("../../migrations/V026__mfa_passkey.sql")),
    (27, include_str!("../../migrations/V027__pgp_keys.sql")),
    (28, include_str!("../../migrations/V028__calendar_stickers.sql")),
    (29, include_str!("../../migrations/V029__filter_rules_v2.sql")),
    (30, include_str!("../../migrations/V030__draft_staging.sql")),
];

#[cfg(test)]
fn run_migrations(conn: &Connection) -> Result<(), String> {
    let current: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read user_version: {e}"))?;

    for &(version, sql) in MIGRATIONS {
        if version > current {
            conn.execute_batch(sql)
                .map_err(|e| format!("Migration V{version:03} failed: {e}"))?;
            conn.execute_batch(&format!("PRAGMA user_version = {version};"))
                .map_err(|e| format!("Failed to set user_version to {version}: {e}"))?;
        }
    }

    Ok(())
}

/// Opens an in-memory SQLite database with all migration scripts applied.
/// Used exclusively by tests so every test starts with a clean, fully-migrated
/// schema without touching the filesystem.
#[cfg(test)]
pub fn open_test_db() -> Connection {
    let conn = Connection::open_in_memory().expect("Failed to open in-memory SQLite");

    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .expect("Failed to enable foreign keys");

    run_migrations(&conn).expect("Test migrations failed");

    conn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_test_db_has_tables() {
        let conn = open_test_db();

        // Verify the three expected tables exist.
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"user_meta".to_string()));
        assert!(tables.contains(&"folders".to_string()));
        assert!(tables.contains(&"messages".to_string()));
        assert!(tables.contains(&"draft_staging".to_string()));
        assert!(tables.contains(&"draft_attachments".to_string()));
    }

    #[test]
    fn test_open_user_db_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().to_str().unwrap();

        let conn = open_user_db(data_dir, "abc123").unwrap();

        // The file should exist on disk.
        let db_file = tmp.path().join("abc123").join("db.sqlite");
        assert!(db_file.exists());

        // Foreign keys should be enabled.
        let fk: i32 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);

        drop(conn);
    }
}
