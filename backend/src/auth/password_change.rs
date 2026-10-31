//! Mailbox password changes through an admin-supplied helper program.
//!
//! IMAP has no command to change a password, and Rav does not know where the
//! mail server keeps its credentials (SQL, LDAP, passwd-file, ...). The admin
//! configures `PASSWORD_CHANGE_COMMAND`, a program that knows the store, and
//! Rav runs it after verifying the user's current password over IMAP.
//!
//! Helper contract:
//! - stdin: `{"username": "<IMAP login>", "new_password": "<plaintext>"}`
//! - exit 0: password changed
//! - exit 1: new password rejected (policy, invalid input)
//! - exit 2: no matching (active) user
//! - anything else, or running past the timeout: failure
//!
//! The password is never passed in argv or the environment. The helper runs
//! with a cleared environment except `PATH` and, when set, systemd's
//! `CREDENTIALS_DIRECTORY` (so it can read secrets from `LoadCredential=`).

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

/// Longest password accepted, in characters.
pub const MAX_PASSWORD_CHARS: usize = 1024;

const HELPER_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Attempts to start the helper while the OS reports it busy, and the pause
/// between them.
const SPAWN_ATTEMPTS: u32 = 5;
const SPAWN_RETRY_DELAY: Duration = Duration::from_millis(20);

/// Why a password change through the helper did not succeed.
#[derive(Debug, PartialEq)]
pub enum ChangeError {
    /// The helper rejected the new password (exit 1).
    Rejected,
    /// The helper found no matching active user (exit 2).
    UnknownUser,
    /// The helper could not be run, crashed, timed out or exited otherwise.
    Failed(String),
}

/// Check a requested new password against Rav's policy. Returns a message
/// suitable for showing to the user when the password is not acceptable.
pub fn validate_new_password(current: &str, new: &str, min_length: usize) -> Result<(), String> {
    let len = new.chars().count();
    if len < min_length {
        return Err(format!("The new password must be at least {min_length} characters long"));
    }
    if len > MAX_PASSWORD_CHARS {
        return Err(format!("The new password must be at most {MAX_PASSWORD_CHARS} characters long"));
    }
    if new.chars().any(char::is_control) {
        return Err("The new password must not contain control characters".to_string());
    }
    if new == current {
        return Err("The new password must differ from the current password".to_string());
    }
    Ok(())
}

/// Run the helper at `command` to set `new_password` for `username`.
pub async fn run_helper(
    command: &str,
    username: &str,
    new_password: &str,
    timeout: Duration,
) -> Result<(), ChangeError> {
    let mut cmd = Command::new(command);
    cmd.env_clear()
        .env("PATH", HELPER_PATH)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = std::env::var_os("CREDENTIALS_DIRECTORY") {
        cmd.env("CREDENTIALS_DIRECTORY", dir);
    }

    // Linux refuses to execute a file that any process still has open for
    // writing ("Text file busy"). That happens briefly when the helper is
    // being replaced, or when another thread forks while the file is written,
    // so retry a few times before giving up.
    let mut attempt = 1;
    let mut child = loop {
        match cmd.spawn() {
            Ok(child) => break child,
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < SPAWN_ATTEMPTS => {
                attempt += 1;
                tokio::time::sleep(SPAWN_RETRY_DELAY).await;
            }
            Err(e) => return Err(ChangeError::Failed(format!("cannot start {command}: {e}"))),
        }
    };

    let input = serde_json::json!({ "username": username, "new_password": new_password }).to_string();

    let interaction = async {
        if let Some(mut stdin) = child.stdin.take() {
            // A helper that exits without reading its input closes the pipe;
            // its exit status below is what decides the outcome.
            let _ = stdin.write_all(input.as_bytes()).await;
        }
        let mut stderr = String::new();
        if let Some(mut err) = child.stderr.take() {
            let _ = err.read_to_string(&mut stderr).await;
        }
        let status = child.wait().await;
        (status, stderr)
    };

    let (status, stderr) = tokio::time::timeout(timeout, interaction)
        .await
        .map_err(|_| ChangeError::Failed(format!("{command} timed out after {}s", timeout.as_secs())))?;
    let status = status.map_err(|e| ChangeError::Failed(format!("waiting for {command}: {e}")))?;

    let detail = stderr.lines().next().unwrap_or("").trim().to_string();
    match status.code() {
        Some(0) => Ok(()),
        Some(1) => {
            tracing::info!(helper_message = %detail, "password change helper rejected the new password");
            Err(ChangeError::Rejected)
        }
        Some(2) => Err(ChangeError::UnknownUser),
        Some(code) => Err(ChangeError::Failed(format!("{command} exited with {code}: {detail}"))),
        None => Err(ChangeError::Failed(format!("{command} was killed by a signal"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_reasonable_password() {
        assert!(validate_new_password("old-secret", "correct horse battery", 8).is_ok());
    }

    #[test]
    fn validate_enforces_minimum_length_in_characters() {
        assert!(validate_new_password("old", "short", 8).is_err());
        // Eight non-ASCII characters are more than eight bytes but still eight characters.
        assert!(validate_new_password("old", "ääääääää", 8).is_ok());
        assert!(validate_new_password("old", "äääääää", 8).is_err());
    }

    #[test]
    fn validate_rejects_too_long_password() {
        let long = "a".repeat(MAX_PASSWORD_CHARS + 1);
        assert!(validate_new_password("old", &long, 8).is_err());
        assert!(validate_new_password("old", &long[..MAX_PASSWORD_CHARS], 8).is_ok());
    }

    #[test]
    fn validate_rejects_control_characters() {
        assert!(validate_new_password("old", "line1\nline2", 8).is_err());
        assert!(validate_new_password("old", "tab\tseparated", 8).is_err());
    }

    #[test]
    fn validate_rejects_unchanged_password() {
        assert!(validate_new_password("same-password", "same-password", 8).is_err());
    }

    #[cfg(unix)]
    mod helper {
        use super::super::*;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::TempDir;

        /// Write an executable shell script into `dir` and return its path.
        fn script(dir: &TempDir, body: &str) -> String {
            let path = dir.path().join("helper.sh");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path.to_string_lossy().into_owned()
        }

        const TIMEOUT: Duration = Duration::from_secs(10);

        #[tokio::test]
        async fn passes_json_on_stdin_and_succeeds_on_exit_0() {
            let dir = TempDir::new().unwrap();
            let out = dir.path().join("input.json");
            let helper = script(&dir, &format!("cat > '{}'", out.display()));

            run_helper(&helper, "user@example.com", "pä$s \"wörd\"", TIMEOUT).await.unwrap();

            let input: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
            assert_eq!(input["username"], "user@example.com");
            assert_eq!(input["new_password"], "pä$s \"wörd\"");
        }

        #[tokio::test]
        async fn password_is_not_in_argv_or_environment() {
            let dir = TempDir::new().unwrap();
            let out = dir.path().join("env.txt");
            let helper = script(&dir, &format!("cat >/dev/null; {{ echo \"$0 $*\"; env; }} > '{}'", out.display()));

            run_helper(&helper, "user@example.com", "s3cret-value", TIMEOUT).await.unwrap();

            let seen = std::fs::read_to_string(&out).unwrap();
            assert!(!seen.contains("s3cret-value"));
            // Only PATH (plus variables the shell sets itself) survives env_clear.
            assert!(!seen.contains("HOME="));
            assert!(seen.contains("PATH="));
        }

        #[tokio::test]
        async fn maps_exit_codes() {
            let dir = TempDir::new().unwrap();

            let helper = script(&dir, "cat >/dev/null; echo 'too weak' >&2; exit 1");
            assert_eq!(run_helper(&helper, "u", "p", TIMEOUT).await, Err(ChangeError::Rejected));

            let helper = script(&dir, "cat >/dev/null; exit 2");
            assert_eq!(run_helper(&helper, "u", "p", TIMEOUT).await, Err(ChangeError::UnknownUser));

            let helper = script(&dir, "cat >/dev/null; echo 'db down' >&2; exit 3");
            match run_helper(&helper, "u", "p", TIMEOUT).await {
                Err(ChangeError::Failed(msg)) => assert!(msg.contains("db down"), "{msg}"),
                other => panic!("expected Failed, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn helper_that_ignores_stdin_still_reports_status() {
            let dir = TempDir::new().unwrap();
            let helper = script(&dir, "exit 0");
            assert_eq!(run_helper(&helper, "u", "p", TIMEOUT).await, Ok(()));
        }

        #[tokio::test]
        async fn missing_helper_fails() {
            let result = run_helper("/nonexistent/rav-passwd", "u", "p", TIMEOUT).await;
            assert!(matches!(result, Err(ChangeError::Failed(_))));
        }

        /// Open the helper for writing, as a concurrent writer or a forked
        /// child holding an inherited descriptor would, so exec gets ETXTBSY.
        fn hold_open_for_writing(path: &str) -> std::fs::File {
            std::fs::OpenOptions::new().append(true).open(path).unwrap()
        }

        #[tokio::test]
        async fn retries_while_helper_is_busy() {
            let dir = TempDir::new().unwrap();
            let helper = script(&dir, "cat >/dev/null; exit 0");
            let writer = hold_open_for_writing(&helper);

            let run = tokio::spawn(async move { run_helper(&helper, "u", "p", TIMEOUT).await });
            // Long enough for the first attempt to fail, well within the retries.
            tokio::time::sleep(SPAWN_RETRY_DELAY).await;
            drop(writer);

            assert_eq!(run.await.unwrap(), Ok(()));
        }

        #[tokio::test]
        async fn gives_up_when_helper_stays_busy() {
            let dir = TempDir::new().unwrap();
            let helper = script(&dir, "exit 0");
            let _writer = hold_open_for_writing(&helper);

            match run_helper(&helper, "u", "p", TIMEOUT).await {
                Err(ChangeError::Failed(msg)) => assert!(msg.contains("cannot start"), "{msg}"),
                other => panic!("expected Failed, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn slow_helper_times_out() {
            let dir = TempDir::new().unwrap();
            let helper = script(&dir, "sleep 5");
            let result = run_helper(&helper, "u", "p", Duration::from_millis(200)).await;
            assert!(matches!(result, Err(ChangeError::Failed(msg)) if msg.contains("timed out")));
        }
    }
}
