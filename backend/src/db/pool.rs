use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use r2d2::{CustomizeConnection, Pool};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

/// Runs once per newly-created physical SQLite connection (not per checkout):
/// enables WAL journal mode and foreign key enforcement. Mirrors the PRAGMAs
/// the old per-request `open_user_db` used to set on every call.
#[derive(Debug)]
struct PragmaCustomizer;

impl CustomizeConnection<Connection, rusqlite::Error> for PragmaCustomizer {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
    }
}

struct PoolEntry {
    pool: Pool<SqliteConnectionManager>,
    last_accessed: Instant,
}

/// Holds one r2d2 connection pool per user's SQLite database, created lazily
/// on first access. This replaces opening a fresh `Connection` (and re-running
/// its setup PRAGMAs) on every request.
///
/// Idle pools are dropped by the periodic eviction sweep (see
/// `spawn_eviction_sweep`) once unused for `idle_timeout`. If accepting a new
/// user would exceed `max_users` held pools, the least-recently-used pool is
/// evicted immediately to make room.
///
/// This is the only supported way to access a user's database - there is no
/// public function to open an unpooled `Connection` directly.
pub struct DbPoolManager {
    data_dir: String,
    max_connections_per_user: u32,
    idle_timeout: Duration,
    max_users: usize,
    pools: Mutex<HashMap<String, PoolEntry>>,
}

impl DbPoolManager {
    pub fn new(
        data_dir: String,
        max_connections_per_user: u32,
        idle_timeout: Duration,
        max_users: usize,
    ) -> Self {
        Self {
            data_dir,
            max_connections_per_user,
            idle_timeout,
            max_users,
            pools: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the pool for `user_hash`, creating it (and the user's data
    /// directory) on first access. Does NOT run migrations - call
    /// `auth::user_data::provision_user_data` first to ensure the schema is
    /// ready.
    fn get_pool(&self, user_hash: &str) -> Result<Pool<SqliteConnectionManager>, String> {
        let mut pools = self.pools.lock().expect("db pool map poisoned");

        if let Some(entry) = pools.get_mut(user_hash) {
            entry.last_accessed = Instant::now();
            return Ok(entry.pool.clone());
        }

        if pools.len() >= self.max_users {
            if let Some(lru_key) = pools
                .iter()
                .min_by_key(|(_, entry)| entry.last_accessed)
                .map(|(k, _)| k.clone())
            {
                pools.remove(&lru_key);
            }
        }

        let dir = Path::new(&self.data_dir).join(user_hash);
        fs::create_dir_all(&dir).map_err(|e| format!("Failed to create db dir: {e}"))?;
        let db_path = dir.join("db.sqlite");

        let manager = SqliteConnectionManager::file(&db_path);
        let pool = Pool::builder()
            .max_size(self.max_connections_per_user)
            .connection_customizer(Box::new(PragmaCustomizer))
            .build(manager)
            .map_err(|e| format!("Failed to build connection pool: {e}"))?;

        pools.insert(
            user_hash.to_string(),
            PoolEntry {
                pool: pool.clone(),
                last_accessed: Instant::now(),
            },
        );

        Ok(pool)
    }

    /// Drops any pool unused for longer than `idle_timeout`.
    fn evict_idle(&self) {
        let idle_timeout = self.idle_timeout;
        let mut pools = self.pools.lock().expect("db pool map poisoned");
        pools.retain(|_, entry| entry.last_accessed.elapsed() < idle_timeout);
    }

    /// Spawns a background task that periodically evicts idle pools. The
    /// sweep runs every minute regardless of the configured timeout, so
    /// timeouts as short as ~1 minute are still enforced promptly.
    pub fn spawn_eviction_sweep(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                self.evict_idle();
            }
        });
    }
}

/// Runs `f` against a pooled connection for `user_hash`, off the async
/// executor thread. This is the only supported way to touch a user's SQLite
/// database from request or worker code.
pub async fn with_user_db<F, T>(manager: &DbPoolManager, user_hash: &str, f: F) -> Result<T, String>
where
    F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    let pool = manager.get_pool(user_hash)?;
    tokio::task::spawn_blocking(move || {
        let conn = pool
            .get()
            .map_err(|e| format!("Failed to get pooled connection: {e}"))?;
        f(&conn)
    })
    .await
    .map_err(|e| format!("Blocking task panicked: {e}"))?
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
    (31, include_str!("../../migrations/V031__outbox.sql")),
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

    #[tokio::test]
    async fn pooled_connection_creates_file_with_pragmas_set() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().to_str().unwrap().to_string();

        let manager = DbPoolManager::new(data_dir, 4, Duration::from_secs(600), 500);

        with_user_db(&manager, "abc123", |conn| {
            let fk: i32 = conn
                .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                .unwrap();
            assert_eq!(fk, 1);
            let journal_mode: String = conn
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .unwrap();
            assert_eq!(journal_mode.to_lowercase(), "wal");
            Ok(())
        })
        .await
        .unwrap();

        let db_file = tmp.path().join("abc123").join("db.sqlite");
        assert!(db_file.exists());
    }

    #[tokio::test]
    async fn lru_eviction_drops_least_recently_used_pool_at_capacity() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().to_str().unwrap().to_string();

        // Cap at 2 users so a 3rd access forces an eviction.
        let manager = DbPoolManager::new(data_dir, 4, Duration::from_secs(600), 2);

        with_user_db(&manager, "user_a", |_| Ok(())).await.unwrap();
        with_user_db(&manager, "user_b", |_| Ok(())).await.unwrap();
        assert_eq!(manager.pools.lock().unwrap().len(), 2);

        with_user_db(&manager, "user_c", |_| Ok(())).await.unwrap();

        let pools = manager.pools.lock().unwrap();
        assert_eq!(pools.len(), 2);
        // user_a was least recently used and should have been evicted.
        assert!(!pools.contains_key("user_a"));
        assert!(pools.contains_key("user_b"));
        assert!(pools.contains_key("user_c"));
    }

    #[tokio::test]
    async fn idle_eviction_sweep_drops_pools_past_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().to_str().unwrap().to_string();

        // Zero-second timeout: any pool is immediately eligible for eviction.
        let manager = DbPoolManager::new(data_dir, 4, Duration::from_secs(0), 500);

        with_user_db(&manager, "abc123", |_| Ok(())).await.unwrap();
        assert_eq!(manager.pools.lock().unwrap().len(), 1);

        manager.evict_idle();

        assert_eq!(manager.pools.lock().unwrap().len(), 0);
    }
}
