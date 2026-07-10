use std::time::Duration;

use async_imap::error::Error as ImapError;
use tracing::warn;

use crate::error::ConnectError;

/// Result of validating credentials against an IMAP server.
#[derive(Debug, PartialEq)]
pub enum AuthResult {
    /// The credentials are valid; login succeeded (and we immediately logged out).
    Success,
    /// The server rejected the credentials (wrong email or password).
    InvalidCredentials,
    /// Could not reach or communicate with the IMAP server.
    ServerUnreachable(ConnectError),
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

/// Validate an email/password pair against a real IMAP server.
///
/// - `host`: TLS SNI hostname (must match the server certificate CN/SAN).
/// - `connect_host`: TCP address to connect to. Pass `"127.0.0.1"` to avoid
///   hairpin NAT when the server cannot reach its own public IP.
/// - `tls_connector`: pre-built connector from `MailTransport`. Already includes
///   any custom CA cert; no cert handling happens here.
pub async fn validate_imap_credentials(
    host: &str,
    connect_host: &str,
    port: u16,
    tls: bool,
    email: &str,
    password: &str,
    tls_connector: &async_native_tls::TlsConnector,
) -> AuthResult {
    let inner = async {
        if tls {
            validate_tls(host, connect_host, port, email, password, tls_connector).await
        } else {
            validate_plain(connect_host, port, email, password).await
        }
    };

    match tokio::time::timeout(CONNECT_TIMEOUT, inner).await {
        Ok(result) => result,
        Err(_) => {
            warn!(host, port, "IMAP connection attempt timed out");
            AuthResult::ServerUnreachable(ConnectError::Timeout)
        }
    }
}

/// TLS path: TCP connects to `connect_host`, TLS SNI uses `host`.
async fn validate_tls(
    host: &str,
    connect_host: &str,
    port: u16,
    email: &str,
    password: &str,
    tls_connector: &async_native_tls::TlsConnector,
) -> AuthResult {
    let addr = format!("{connect_host}:{port}");

    let tcp_stream = match tokio::net::TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, host, port, "IMAP TCP connect failed");
            return AuthResult::ServerUnreachable(ConnectError::from_io(&e));
        }
    };

    let tls_stream = match tls_connector.connect(host, tcp_stream).await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, host, port, "IMAP TLS handshake failed");
            return AuthResult::ServerUnreachable(ConnectError::TlsHandshake);
        }
    };

    let mut client = async_imap::Client::new(tls_stream);

    // Read the server greeting before attempting login.
    match client.read_response().await {
        Ok(Some(_)) => {}
        Ok(None) => {
            warn!(host, port, "IMAP server closed connection before greeting");
            return AuthResult::ServerUnreachable(ConnectError::Unreachable);
        }
        Err(e) => {
            warn!(error = %e, host, port, "IMAP greeting read failed");
            return AuthResult::ServerUnreachable(ConnectError::from_io(&e));
        }
    }

    attempt_login(client, email, password).await
}

/// Non-TLS path: connect via `tokio::net::TcpStream`, then wrap directly in
/// `async_imap::Client`.
async fn validate_plain(host: &str, port: u16, email: &str, password: &str) -> AuthResult {
    let addr = format!("{host}:{port}");

    let tcp_stream = match tokio::net::TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, host, port, "IMAP TCP connect failed");
            return AuthResult::ServerUnreachable(ConnectError::from_io(&e));
        }
    };

    let mut client = async_imap::Client::new(tcp_stream);

    // Read the server greeting before attempting login.
    match client.read_response().await {
        Ok(Some(_)) => {}
        Ok(None) => {
            warn!(host, port, "IMAP server closed connection before greeting");
            return AuthResult::ServerUnreachable(ConnectError::Unreachable);
        }
        Err(e) => {
            warn!(error = %e, host, port, "IMAP greeting read failed");
            return AuthResult::ServerUnreachable(ConnectError::from_io(&e));
        }
    }

    attempt_login(client, email, password).await
}

/// Shared login logic: attempt LOGIN, logout on success, classify errors.
async fn attempt_login<T>(client: async_imap::Client<T>, email: &str, password: &str) -> AuthResult
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    match client.login(email, password).await {
        Ok(mut session) => {
            // Successfully authenticated; clean up.
            let _ = session.logout().await;
            AuthResult::Success
        }
        Err((err, _client)) => classify_login_error(err),
    }
}

/// Classify an IMAP error into `InvalidCredentials` or `ServerUnreachable`.
///
/// A `No` response from the server means the credentials were rejected.
/// Everything else (I/O errors, BAD responses, parse errors, etc.) is
/// treated as a server/network problem.
fn classify_login_error(err: ImapError) -> AuthResult {
    match err {
        ImapError::No(_) => AuthResult::InvalidCredentials,
        other => AuthResult::ServerUnreachable(classify_imap_error(other)),
    }
}

fn classify_imap_error(err: ImapError) -> ConnectError {
    match err {
        ImapError::Io(e) => {
            warn!(error = %e, "IMAP I/O error during login");
            ConnectError::from_io(&e)
        }
        ImapError::ConnectionLost => {
            warn!("IMAP connection lost during login");
            ConnectError::Unreachable
        }
        other => {
            warn!(error = %other, "Unexpected IMAP error during login");
            ConnectError::BadServerResponse
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_connector() -> async_native_tls::TlsConnector {
        async_native_tls::TlsConnector::new()
    }

    /// Connecting to an unreachable server should return `ServerUnreachable`.
    ///
    /// We use 127.0.0.1:19999, a port that (with very high probability) has
    /// nothing listening on localhost.
    #[tokio::test]
    async fn unreachable_server_returns_server_unreachable() {
        let result = validate_imap_credentials(
            "127.0.0.1",
            "127.0.0.1",
            19999,
            false,
            "user@test.com",
            "pass",
            &default_connector(),
        )
        .await;

        assert!(
            matches!(result, AuthResult::ServerUnreachable(_)),
            "expected ServerUnreachable, got {result:?}",
        );
    }

    /// Same test but with TLS enabled: should also return `ServerUnreachable`.
    #[tokio::test]
    async fn unreachable_server_tls_returns_server_unreachable() {
        let result = validate_imap_credentials(
            "127.0.0.1",
            "127.0.0.1",
            19999,
            true,
            "user@test.com",
            "pass",
            &default_connector(),
        )
        .await;

        assert!(
            matches!(result, AuthResult::ServerUnreachable(_)),
            "expected ServerUnreachable, got {result:?}",
        );
    }

    /// Verify the error classifier maps `No` to `InvalidCredentials`.
    #[test]
    fn classify_no_as_invalid_credentials() {
        let err = ImapError::No("Authentication failed".into());
        assert_eq!(classify_login_error(err), AuthResult::InvalidCredentials);
    }

    /// Verify the error classifier maps I/O `ConnectionRefused` to the correct variant.
    #[test]
    fn classify_io_as_server_unreachable() {
        let err = ImapError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused",
        ));
        assert_eq!(
            classify_login_error(err),
            AuthResult::ServerUnreachable(ConnectError::ConnectionRefused),
        );
    }

    /// Verify the error classifier maps I/O `TimedOut` to `Timeout`.
    #[test]
    fn classify_io_timeout_as_timeout_variant() {
        let err = ImapError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timed out",
        ));
        assert_eq!(
            classify_login_error(err),
            AuthResult::ServerUnreachable(ConnectError::Timeout),
        );
    }

    /// Verify the error classifier maps `Bad` responses to `BadServerResponse`.
    #[test]
    fn classify_bad_as_server_unreachable() {
        let err = ImapError::Bad("server error".into());
        assert_eq!(
            classify_login_error(err),
            AuthResult::ServerUnreachable(ConnectError::BadServerResponse),
        );
    }

    /// Verify `ConnectionLost` maps to `Unreachable`.
    #[test]
    fn classify_connection_lost_as_unreachable() {
        let err = ImapError::ConnectionLost;
        assert_eq!(
            classify_login_error(err),
            AuthResult::ServerUnreachable(ConnectError::Unreachable),
        );
    }
}
