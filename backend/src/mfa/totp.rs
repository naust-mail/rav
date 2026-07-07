use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use totp_rs::{Algorithm, Secret, TOTP};

use crate::db::mfa as db;

const TOTP_DIGITS: usize = 6;
const TOTP_STEP: u64 = 30;
const TOTP_SKEW: u8 = 1;
const MAX_FAILURES: i64 = 5;
const LOCKOUT_SECONDS: i64 = 900; // 15 minutes

/// Generates a new random TOTP secret and returns its base32 representation.
pub fn generate_secret() -> Result<String, String> {
    let secret = Secret::generate_secret();
    secret
        .to_encoded()
        .to_string()
        .parse::<String>()
        .map_err(|e| format!("Secret encoding failed: {e}"))
}

/// Returns the `otpauth://` URI for QR code display.
pub fn get_url(secret_b32: &str, email: &str, issuer: &str) -> Result<String, String> {
    let secret_bytes = Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .map_err(|e| format!("Invalid base32 secret: {e}"))?;

    let totp = TOTP::new(
        Algorithm::SHA1,
        TOTP_DIGITS,
        TOTP_SKEW,
        TOTP_STEP,
        secret_bytes,
        Some(issuer.to_string()),
        email.to_string(),
    )
    .map_err(|e| format!("TOTP init failed: {e}"))?;

    Ok(totp.get_url())
}

/// Verifies `code` against `secret_b32` with a ±1 step window.
/// On success, marks the matched step as used (replay prevention) and resets
/// the lockout counter. On failure, increments the lockout counter.
///
/// Returns `Ok(true)` when valid, `Ok(false)` when invalid or replayed.
/// Returns `Err` only for unexpected DB or crypto failures.
pub fn verify_and_record(
    conn: &Connection,
    secret_b32: &str,
    code: &str,
) -> Result<bool, String> {
    let secret_bytes = Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .map_err(|e| format!("Invalid base32 secret: {e}"))?;

    let totp = TOTP::new(
        Algorithm::SHA1,
        TOTP_DIGITS,
        TOTP_SKEW,
        TOTP_STEP,
        secret_bytes,
        None,
        String::new(),
    )
    .map_err(|e| format!("TOTP init failed: {e}"))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Clock error: {e}"))?
        .as_secs();

    let current_step = now / TOTP_STEP;

    // Prune used steps older than 5 minutes before checking.
    db::prune_used_steps(conn, now.saturating_sub(300))?;

    for offset in [-1i64, 0, 1] {
        let check_step = current_step.saturating_add_signed(offset);
        let check_time = check_step * TOTP_STEP;
        let expected = totp.generate(check_time);

        if constant_eq(code.trim(), &expected) {
            if db::is_step_used(conn, check_step)? {
                // Valid code but already consumed - replay attempt.
                return Ok(false);
            }
            db::record_used_step(conn, check_step)?;
            db::reset_lockout(conn)?;
            return Ok(true);
        }
    }

    db::increment_lockout(conn, MAX_FAILURES, LOCKOUT_SECONDS)?;
    Ok(false)
}

/// Returns `true` if the account is currently locked out.
pub fn is_locked_out(conn: &Connection) -> Result<bool, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Clock error: {e}"))?
        .as_secs() as i64;

    db::check_lockout(conn, now)
}

/// Constant-time string comparison to prevent timing oracles.
fn constant_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::open_test_db;

    #[test]
    fn totp_replay_second_use_rejected() {
        let conn = open_test_db();
        let secret = generate_secret().unwrap();

        let secret_bytes = Secret::Encoded(secret.clone()).to_bytes().unwrap();
        let totp = TOTP::new(Algorithm::SHA1, TOTP_DIGITS, TOTP_SKEW, TOTP_STEP, secret_bytes, None, String::new()).unwrap();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let code = totp.generate(now / TOTP_STEP * TOTP_STEP);

        assert!(verify_and_record(&conn, &secret, &code).unwrap(), "first use must succeed");
        assert!(!verify_and_record(&conn, &secret, &code).unwrap(), "replayed code must be rejected");
    }
}
