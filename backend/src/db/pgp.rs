use rusqlite::{params, Connection};
use serde::Serialize;

/// A stored PGP key record, mirroring the `pgp_keys` table.
#[derive(Debug, Clone, Serialize)]
pub struct PgpKeyRecord {
    /// UUID string, generated client-side.
    pub id: String,
    /// Associated identity, if any.
    pub identity_id: Option<i64>,
    /// Hex-encoded uppercase OpenPGP fingerprint.
    pub fingerprint: String,
    /// Armored public key.
    pub public_key: String,
    /// Passphrase-protected armored private key (opaque blob from client).
    pub private_key_enc: String,
    /// Unix timestamp of key creation.
    pub created_at: i64,
}

/// Public summary of a PGP key (no private key material).
#[derive(Debug, Clone, Serialize)]
pub struct PgpKeySummary {
    pub id: String,
    pub identity_id: Option<i64>,
    pub fingerprint: String,
    pub public_key: String,
    pub created_at: i64,
}


const SUMMARY_COLS: &str = "id, identity_id, fingerprint, public_key, created_at";
const FULL_COLS: &str = "id, identity_id, fingerprint, public_key, private_key_enc, created_at";

fn row_to_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<PgpKeySummary> {
    Ok(PgpKeySummary {
        id: row.get(0)?,
        identity_id: row.get(1)?,
        fingerprint: row.get(2)?,
        public_key: row.get(3)?,
        created_at: row.get(4)?,
    })
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<PgpKeyRecord> {
    Ok(PgpKeyRecord {
        id: row.get(0)?,
        identity_id: row.get(1)?,
        fingerprint: row.get(2)?,
        public_key: row.get(3)?,
        private_key_enc: row.get(4)?,
        created_at: row.get(5)?,
    })
}

/// List all stored keys ordered by creation time descending.
pub fn list_keys(conn: &Connection) -> Result<Vec<PgpKeySummary>, String> {
    let sql = format!("SELECT {SUMMARY_COLS} FROM pgp_keys ORDER BY created_at DESC");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare list_keys: {e}"))?;
    let rows = stmt
        .query_map([], row_to_summary)
        .map_err(|e| format!("Failed to query pgp_keys: {e}"))?;

    let mut keys = Vec::new();
    for row in rows {
        keys.push(row.map_err(|e| format!("Failed to read pgp_key row: {e}"))?);
    }
    Ok(keys)
}

/// Get a single key by ID, including the encrypted private key.
pub fn get_key(conn: &Connection, id: &str) -> Result<Option<PgpKeyRecord>, String> {
    let sql = format!("SELECT {FULL_COLS} FROM pgp_keys WHERE id = ?1");
    match conn.query_row(&sql, params![id], row_to_record) {
        Ok(record) => Ok(Some(record)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to get pgp_key: {e}")),
    }
}


/// Insert or replace a PGP key record.
pub fn upsert_key(
    conn: &Connection,
    id: &str,
    identity_id: Option<i64>,
    fingerprint: &str,
    public_key: &str,
    private_key_enc: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO pgp_keys (id, identity_id, fingerprint, public_key, private_key_enc)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
           identity_id     = excluded.identity_id,
           fingerprint     = excluded.fingerprint,
           public_key      = excluded.public_key,
           private_key_enc = excluded.private_key_enc",
        params![id, identity_id, fingerprint, public_key, private_key_enc],
    )
    .map_err(|e| format!("Failed to upsert pgp_key: {e}"))?;
    Ok(())
}

/// Delete a PGP key by ID. Returns true if a row was deleted.
pub fn delete_key(conn: &Connection, id: &str) -> Result<bool, String> {
    let deleted = conn
        .execute("DELETE FROM pgp_keys WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to delete pgp_key: {e}"))?;
    Ok(deleted > 0)
}

/// Assign or unassign an identity to a key. Returns true if the key exists.
pub fn assign_identity(
    conn: &Connection,
    id: &str,
    identity_id: Option<i64>,
) -> Result<bool, String> {
    let updated = conn
        .execute(
            "UPDATE pgp_keys SET identity_id = ?1 WHERE id = ?2",
            params![identity_id, id],
        )
        .map_err(|e| format!("Failed to assign identity: {e}"))?;
    Ok(updated > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::open_test_db;

    fn sample_key(suffix: &str) -> (String, String, String, String) {
        (
            format!("key-id-{suffix}"),
            format!("FINGERPRINT{suffix}"),
            format!("-----BEGIN PGP PUBLIC KEY BLOCK-----\npub{suffix}\n-----END PGP PUBLIC KEY BLOCK-----"),
            format!("-----BEGIN PGP PRIVATE KEY BLOCK-----\nenc{suffix}\n-----END PGP PRIVATE KEY BLOCK-----"),
        )
    }

    #[test]
    fn test_upsert_and_retrieve() {
        let conn = open_test_db();
        let (id, fp, pubkey, privkey) = sample_key("1");

        upsert_key(&conn, &id, None, &fp, &pubkey, &privkey).unwrap();

        let record = get_key(&conn, &id).unwrap().unwrap();
        assert_eq!(record.id, id);
        assert_eq!(record.fingerprint, fp);
        assert_eq!(record.public_key, pubkey);
        assert_eq!(record.private_key_enc, privkey);
        assert_eq!(record.identity_id, None);
    }

    #[test]
    fn test_fingerprint_uniqueness_constraint() {
        let conn = open_test_db();
        let (_, fp, pubkey, privkey) = sample_key("2");

        upsert_key(&conn, "id-a", None, &fp, &pubkey, &privkey).unwrap();
        // Inserting a second key with the same fingerprint but different ID must fail.
        let result = conn.execute(
            "INSERT INTO pgp_keys (id, fingerprint, public_key, private_key_enc) VALUES ('id-b', ?1, ?2, ?3)",
            params![fp, pubkey, privkey],
        );
        assert!(result.is_err(), "Duplicate fingerprint should fail");
    }

    #[test]
    fn test_delete_returns_false_when_not_found() {
        let conn = open_test_db();
        assert!(!delete_key(&conn, "nonexistent").unwrap());
    }

    #[test]
    fn test_delete_key() {
        let conn = open_test_db();
        let (id, fp, pubkey, privkey) = sample_key("3");
        upsert_key(&conn, &id, None, &fp, &pubkey, &privkey).unwrap();
        assert!(delete_key(&conn, &id).unwrap());
        assert!(get_key(&conn, &id).unwrap().is_none());
    }

    #[test]
    fn test_assign_and_unassign_identity() {
        let conn = open_test_db();
        // FK constraints are enabled - insert a real identity first.
        let identity_id: i64 = conn
            .query_row(
                "INSERT INTO identities (email, display_name, signature_html, is_default) VALUES ('test@example.com', '', '', 0) RETURNING id",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let (id, fp, pubkey, privkey) = sample_key("4");
        upsert_key(&conn, &id, None, &fp, &pubkey, &privkey).unwrap();

        assert!(assign_identity(&conn, &id, Some(identity_id)).unwrap());
        let record = get_key(&conn, &id).unwrap().unwrap();
        assert_eq!(record.identity_id, Some(identity_id));

        assert!(assign_identity(&conn, &id, None).unwrap());
        let record = get_key(&conn, &id).unwrap().unwrap();
        assert_eq!(record.identity_id, None);
    }

    #[test]
    fn test_assign_identity_returns_false_when_not_found() {
        let conn = open_test_db();
        assert!(!assign_identity(&conn, "nonexistent", Some(1)).unwrap());
    }

    #[test]
    fn test_list_returns_summaries_without_private_key() {
        let conn = open_test_db();
        for i in 0..3u32 {
            let (id, fp, pubkey, privkey) = sample_key(&i.to_string());
            upsert_key(&conn, &id, None, &fp, &pubkey, &privkey).unwrap();
        }

        let summaries = list_keys(&conn).unwrap();
        assert_eq!(summaries.len(), 3);
        // PgpKeySummary has no private_key_enc field - verified by type system.
    }


}
