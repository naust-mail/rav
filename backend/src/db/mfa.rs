use rusqlite::Connection;

#[cfg(test)]
use crate::auth::session::ServerEndpoint;

/// Returns `true` if a TOTP credential is enrolled for this user.
pub fn is_totp_enrolled(conn: &Connection) -> Result<bool, String> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM mfa_totp WHERE id = 1", [], |row| {
            row.get(0)
        })
        .map_err(|e| format!("DB error checking TOTP enrollment: {e}"))?;
    Ok(count > 0)
}

type TotpSecretResult = Result<Option<(Vec<u8>, Vec<u8>)>, String>;

/// Load the encrypted TOTP secret and nonce. Returns `None` if not enrolled.
pub fn get_totp_secret(conn: &Connection) -> TotpSecretResult {
    let result = conn.query_row(
        "SELECT encrypted_secret, nonce FROM mfa_totp WHERE id = 1",
        [],
        |row| {
            let secret: Vec<u8> = row.get(0)?;
            let nonce: Vec<u8> = row.get(1)?;
            Ok((secret, nonce))
        },
    );

    match result {
        Ok(pair) => Ok(Some(pair)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("DB error reading TOTP secret: {e}")),
    }
}

/// Store (or replace) the encrypted TOTP secret.
pub fn upsert_totp_secret(
    conn: &Connection,
    encrypted_secret: &[u8],
    nonce: &[u8],
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO mfa_totp (id, encrypted_secret, nonce)
         VALUES (1, ?1, ?2)
         ON CONFLICT(id) DO UPDATE SET
             encrypted_secret = excluded.encrypted_secret,
             nonce             = excluded.nonce,
             created_at        = unixepoch()",
        rusqlite::params![encrypted_secret, nonce],
    )
    .map_err(|e| format!("DB error storing TOTP secret: {e}"))?;
    Ok(())
}

/// Remove the TOTP credential and all associated replay/lockout state.
pub fn delete_totp(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "DELETE FROM mfa_totp;
         DELETE FROM mfa_totp_used_steps;
         UPDATE mfa_lockout SET failed_count = 0, locked_until = NULL, last_failure = NULL WHERE id = 1;",
    )
    .map_err(|e| format!("DB error deleting TOTP: {e}"))?;
    Ok(())
}

/// Returns `true` if the given time step has already been consumed.
pub fn is_step_used(conn: &Connection, step: u64) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM mfa_totp_used_steps WHERE step = ?1",
            rusqlite::params![step as i64],
            |row| row.get(0),
        )
        .map_err(|e| format!("DB error checking used step: {e}"))?;
    Ok(count > 0)
}

/// Mark a time step as consumed.
pub fn record_used_step(conn: &Connection, step: u64) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO mfa_totp_used_steps (step) VALUES (?1)",
        rusqlite::params![step as i64],
    )
    .map_err(|e| format!("DB error recording used step: {e}"))?;
    Ok(())
}

/// Delete used steps with `used_at` before `cutoff_epoch`.
pub fn prune_used_steps(conn: &Connection, cutoff_epoch: u64) -> Result<(), String> {
    conn.execute(
        "DELETE FROM mfa_totp_used_steps WHERE used_at < ?1",
        rusqlite::params![cutoff_epoch as i64],
    )
    .map_err(|e| format!("DB error pruning used steps: {e}"))?;
    Ok(())
}

/// Returns `true` if `locked_until` is in the future.
pub fn check_lockout(conn: &Connection, now: i64) -> Result<bool, String> {
    let locked_until: Option<i64> = conn
        .query_row(
            "SELECT locked_until FROM mfa_lockout WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("DB error reading lockout: {e}"))?;

    Ok(locked_until.map(|t| t > now).unwrap_or(false))
}

/// Increment the failure counter. If it reaches `max_failures`, set
/// `locked_until` to `now + lockout_seconds`.
pub fn increment_lockout(
    conn: &Connection,
    max_failures: i64,
    lockout_seconds: i64,
) -> Result<(), String> {
    let new_count: i64 = conn
        .query_row(
            "UPDATE mfa_lockout SET
                 failed_count = failed_count + 1,
                 last_failure = unixepoch()
             WHERE id = 1
             RETURNING failed_count",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("DB error incrementing lockout: {e}"))?;

    if new_count >= max_failures {
        conn.execute(
            "UPDATE mfa_lockout SET locked_until = unixepoch() + ?1 WHERE id = 1",
            rusqlite::params![lockout_seconds],
        )
        .map_err(|e| format!("DB error setting lockout: {e}"))?;
    }

    Ok(())
}

/// Reset the failure counter and clear any lockout after a successful auth.
pub fn reset_lockout(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "UPDATE mfa_lockout SET failed_count = 0, locked_until = NULL, last_failure = NULL WHERE id = 1",
        [],
    )
    .map_err(|e| format!("DB error resetting lockout: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Passkey CRUD
// ---------------------------------------------------------------------------

/// Stored metadata for a single enrolled passkey (full row for auth ceremony).
pub struct PasskeyRow {
    pub credential_id: String,
    pub passkey_json: String,
    pub prf_salt: Vec<u8>,
    pub encrypted_imap: Vec<u8>,
    pub imap_nonce: Vec<u8>,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_tls: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: bool,
}

/// Minimal info returned by the list endpoint.
pub struct PasskeyInfo {
    pub credential_id: String,
    pub name: String,
    pub created_at: i64,
}

/// Parameters for [`insert_passkey`].
pub struct NewPasskey<'a> {
    pub credential_id: &'a str,
    pub passkey_json: &'a str,
    pub prf_salt: &'a [u8],
    pub encrypted_imap: &'a [u8],
    pub imap_nonce: &'a [u8],
    pub name: &'a str,
}

/// Insert a new passkey credential.
pub fn insert_passkey(
    conn: &Connection,
    p: NewPasskey,
    imap: crate::auth::session::ServerEndpoint,
    smtp: crate::auth::session::ServerEndpoint,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO mfa_passkeys
             (credential_id, passkey_json, prf_salt, encrypted_imap, imap_nonce,
              name, imap_host, imap_port, imap_tls, smtp_host, smtp_port, smtp_tls)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            p.credential_id,
            p.passkey_json,
            p.prf_salt,
            p.encrypted_imap,
            p.imap_nonce,
            p.name,
            imap.host,
            imap.port as i64,
            imap.tls as i64,
            smtp.host,
            smtp.port as i64,
            smtp.tls as i64,
        ],
    )
    .map_err(|e| format!("DB error inserting passkey: {e}"))?;
    Ok(())
}

/// Update the stored passkey JSON (sign count) after a successful authentication.
pub fn update_passkey_json(
    conn: &Connection,
    credential_id: &str,
    passkey_json: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE mfa_passkeys SET passkey_json = ?1 WHERE credential_id = ?2",
        rusqlite::params![passkey_json, credential_id],
    )
    .map_err(|e| format!("DB error updating passkey: {e}"))?;
    Ok(())
}

/// Load a single passkey row by credential ID.
pub fn get_passkey(conn: &Connection, credential_id: &str) -> Result<Option<PasskeyRow>, String> {
    let result = conn.query_row(
        "SELECT credential_id, passkey_json, prf_salt, encrypted_imap, imap_nonce,
                imap_host, imap_port, imap_tls, smtp_host, smtp_port, smtp_tls
         FROM mfa_passkeys WHERE credential_id = ?1",
        rusqlite::params![credential_id],
        |row| {
            Ok(PasskeyRow {
                credential_id: row.get(0)?,
                passkey_json: row.get(1)?,
                prf_salt: row.get(2)?,
                encrypted_imap: row.get(3)?,
                imap_nonce: row.get(4)?,
                imap_host: row.get(5)?,
                imap_port: row.get::<_, i64>(6)? as u16,
                imap_tls: row.get::<_, i64>(7)? != 0,
                smtp_host: row.get(8)?,
                smtp_port: row.get::<_, i64>(9)? as u16,
                smtp_tls: row.get::<_, i64>(10)? != 0,
            })
        },
    );
    match result {
        Ok(row) => Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("DB error fetching passkey: {e}")),
    }
}

/// Load all passkeys for the user (full rows, for the authentication ceremony).
pub fn list_passkeys_full(conn: &Connection) -> Result<Vec<PasskeyRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT credential_id, passkey_json, prf_salt, encrypted_imap, imap_nonce,
                    imap_host, imap_port, imap_tls, smtp_host, smtp_port, smtp_tls
             FROM mfa_passkeys ORDER BY created_at ASC",
        )
        .map_err(|e| format!("DB error preparing passkey list: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(PasskeyRow {
                credential_id: row.get(0)?,
                passkey_json: row.get(1)?,
                prf_salt: row.get(2)?,
                encrypted_imap: row.get(3)?,
                imap_nonce: row.get(4)?,
                imap_host: row.get(5)?,
                imap_port: row.get::<_, i64>(6)? as u16,
                imap_tls: row.get::<_, i64>(7)? != 0,
                smtp_host: row.get(8)?,
                smtp_port: row.get::<_, i64>(9)? as u16,
                smtp_tls: row.get::<_, i64>(10)? != 0,
            })
        })
        .map_err(|e| format!("DB error listing passkeys: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("DB error collecting passkeys: {e}"))?;

    Ok(rows)
}

/// Load summary info for the settings UI.
pub fn list_passkeys_info(conn: &Connection) -> Result<Vec<PasskeyInfo>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT credential_id, name, created_at FROM mfa_passkeys ORDER BY created_at ASC",
        )
        .map_err(|e| format!("DB error preparing passkey info list: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(PasskeyInfo {
                credential_id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
            })
        })
        .map_err(|e| format!("DB error listing passkey info: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("DB error collecting passkey info: {e}"))?;

    Ok(rows)
}

/// Returns `true` if at least one passkey is enrolled.
pub fn has_passkeys(conn: &Connection) -> Result<bool, String> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM mfa_passkeys", [], |row| row.get(0))
        .map_err(|e| format!("DB error counting passkeys: {e}"))?;
    Ok(count > 0)
}

/// Delete a passkey by credential ID. Returns `true` if a row was deleted.
pub fn delete_passkey(conn: &Connection, credential_id: &str) -> Result<bool, String> {
    let n = conn
        .execute(
            "DELETE FROM mfa_passkeys WHERE credential_id = ?1",
            rusqlite::params![credential_id],
        )
        .map_err(|e| format!("DB error deleting passkey: {e}"))?;
    Ok(n > 0)
}

// ---------------------------------------------------------------------------
// MFA settings
// ---------------------------------------------------------------------------

pub struct MfaSettings {
    pub passkey_only: bool,
}

pub fn get_mfa_settings(conn: &Connection) -> Result<MfaSettings, String> {
    let passkey_only: i64 = conn
        .query_row(
            "SELECT passkey_only FROM mfa_settings WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("DB error reading MFA settings: {e}"))?;
    Ok(MfaSettings {
        passkey_only: passkey_only != 0,
    })
}

pub fn set_passkey_only(conn: &Connection, enabled: bool) -> Result<(), String> {
    conn.execute(
        "UPDATE mfa_settings SET passkey_only = ?1 WHERE id = 1",
        rusqlite::params![enabled as i64],
    )
    .map_err(|e| format!("DB error setting passkey_only: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::open_test_db;

    fn insert_test_passkey(conn: &Connection, credential_id: &str, name: &str) {
        insert_passkey(
            conn,
            NewPasskey {
                credential_id,
                passkey_json: r#"{"test":"passkey"}"#,
                prf_salt: &[0u8; 32],
                encrypted_imap: &[0u8; 16],
                imap_nonce: &[0u8; 12],
                name,
            },
            ServerEndpoint { host: "imap.example.com".to_string(), port: 993, tls: true },
            ServerEndpoint { host: "smtp.example.com".to_string(), port: 587, tls: true },
        )
        .expect("insert_test_passkey should succeed");
    }

    #[test]
    fn passkey_insert_and_get() {
        let conn = open_test_db();
        insert_test_passkey(&conn, "cred-abc", "My Key");

        let row = get_passkey(&conn, "cred-abc").unwrap().expect("should find row");
        assert_eq!(row.credential_id, "cred-abc");
        assert_eq!(row.passkey_json, r#"{"test":"passkey"}"#);
        assert_eq!(row.prf_salt, vec![0u8; 32]);
        assert_eq!(row.imap_host, "imap.example.com");
        assert_eq!(row.imap_port, 993);
        assert!(row.imap_tls);
        assert_eq!(row.smtp_host, "smtp.example.com");
        assert_eq!(row.smtp_port, 587);
        assert!(row.smtp_tls);
    }

    #[test]
    fn passkey_get_nonexistent_returns_none() {
        let conn = open_test_db();
        let row = get_passkey(&conn, "nonexistent").unwrap();
        assert!(row.is_none());
    }

    #[test]
    fn passkey_list_info_returns_ordered_by_created_at() {
        let conn = open_test_db();
        insert_test_passkey(&conn, "cred-1", "First Key");
        insert_test_passkey(&conn, "cred-2", "Second Key");

        let infos = list_passkeys_info(&conn).unwrap();
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].credential_id, "cred-1");
        assert_eq!(infos[0].name, "First Key");
        assert_eq!(infos[1].credential_id, "cred-2");
        assert_eq!(infos[1].name, "Second Key");
    }

    #[test]
    fn passkey_delete_removes_row() {
        let conn = open_test_db();
        insert_test_passkey(&conn, "cred-del", "Delete Me");

        let deleted = delete_passkey(&conn, "cred-del").unwrap();
        assert!(deleted);

        let row = get_passkey(&conn, "cred-del").unwrap();
        assert!(row.is_none());
    }

    #[test]
    fn passkey_delete_nonexistent_returns_false() {
        let conn = open_test_db();
        let deleted = delete_passkey(&conn, "no-such-cred").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn passkey_update_json_persists() {
        let conn = open_test_db();
        insert_test_passkey(&conn, "cred-upd", "Updatable");

        update_passkey_json(&conn, "cred-upd", r#"{"updated":true}"#).unwrap();

        let row = get_passkey(&conn, "cred-upd").unwrap().expect("row should exist");
        assert_eq!(row.passkey_json, r#"{"updated":true}"#);
    }

    #[test]
    fn has_passkeys_false_when_empty() {
        let conn = open_test_db();
        assert!(!has_passkeys(&conn).unwrap());
    }

    #[test]
    fn has_passkeys_true_after_insert() {
        let conn = open_test_db();
        insert_test_passkey(&conn, "cred-exists", "Exists");
        assert!(has_passkeys(&conn).unwrap());
    }

    #[test]
    fn mfa_settings_default_passkey_only_false() {
        let conn = open_test_db();
        let settings = get_mfa_settings(&conn).unwrap();
        assert!(!settings.passkey_only);
    }

    #[test]
    fn set_passkey_only_and_read_back() {
        let conn = open_test_db();

        set_passkey_only(&conn, true).unwrap();
        assert!(get_mfa_settings(&conn).unwrap().passkey_only);

        set_passkey_only(&conn, false).unwrap();
        assert!(!get_mfa_settings(&conn).unwrap().passkey_only);
    }
}
