use tracing::warn;

use crate::error::ConnectError;

use super::error::ImapError;
use super::types::ImapCredentials;

// ---- Connection helper ----------------------------------------------------

/// Establish an authenticated IMAP session.
///
/// - `connect_host`: TCP address to connect to (may differ from `creds.host`
///   to avoid hairpin NAT). The TLS SNI hostname is always `creds.host`.
/// - `tls_connector`: pre-built connector from `MailTransport`, already
///   includes any custom CA cert.
///
/// Returns a `Session` over a TLS stream (when `creds.tls` is true) or a
/// plain TCP stream.  Because the two stream types are different concrete
/// types we use an enum wrapper that implements the traits `async-imap`
/// requires (`tokio::io::AsyncRead + AsyncWrite + Unpin + Debug`).
pub(crate) async fn connect(
    creds: &ImapCredentials,
    connect_host: &str,
    tls_connector: &async_native_tls::TlsConnector,
) -> Result<async_imap::Session<ImapStream>, ImapError> {
    let connect_future = tokio::net::TcpStream::connect((connect_host, creds.port));
    // 10 second timeout for the initial TCP connection
    let tcp = tokio::time::timeout(std::time::Duration::from_secs(10), connect_future)
        .await
        .map_err(|_| {
            warn!(host = connect_host, port = creds.port, "IMAP TCP connect timed out");
            ImapError::ConnectionFailed(ConnectError::Timeout)
        })?
        .map_err(|e| {
            warn!(error = %e, host = connect_host, port = creds.port, "IMAP TCP connect failed");
            ImapError::ConnectionFailed(ConnectError::from_io(&e))
        })?;

    if creds.tls {
        let tls_stream = tls_connector
            .connect(&creds.host, tcp)
            .await
            .map_err(|e| {
                warn!(error = %e, host = creds.host, port = creds.port, "IMAP TLS handshake failed");
                ImapError::ConnectionFailed(ConnectError::TlsHandshake)
            })?;
        let client = async_imap::Client::new(ImapStream::Tls(tls_stream));
        let session = client
            .login(&creds.email, &creds.password)
            .await
            .map_err(|(e, _)| classify_login_error(e))?;
        Ok(session)
    } else {
        let client = async_imap::Client::new(ImapStream::Plain(tcp));
        let session = client
            .login(&creds.email, &creds.password)
            .await
            .map_err(|(e, _)| classify_login_error(e))?;
        Ok(session)
    }
}

/// Classify an `async_imap::error::Error` that occurred during LOGIN.
pub(crate) fn classify_login_error(err: async_imap::error::Error) -> ImapError {
    match err {
        async_imap::error::Error::No(_) => ImapError::AuthenticationFailed,
        async_imap::error::Error::Io(e) => {
            warn!(error = %e, "IMAP I/O error during session login");
            ImapError::ConnectionFailed(ConnectError::from_io(&e))
        }
        async_imap::error::Error::ConnectionLost => {
            warn!("IMAP connection lost during session login");
            ImapError::ConnectionFailed(ConnectError::Unreachable)
        }
        other => {
            warn!(error = %other, "Unexpected IMAP error during session login");
            ImapError::ConnectionFailed(ConnectError::BadServerResponse)
        }
    }
}

/// Map a generic `async_imap` error to our `ImapError`.
pub(crate) fn map_imap_error(err: async_imap::error::Error) -> ImapError {
    match err {
        async_imap::error::Error::No(msg) => ImapError::ProtocolError(format!("NO: {msg}")),
        async_imap::error::Error::Io(e) => {
            warn!(error = %e, "IMAP I/O error");
            ImapError::ConnectionFailed(ConnectError::from_io(&e))
        }
        async_imap::error::Error::ConnectionLost => {
            warn!("IMAP connection lost");
            ImapError::ConnectionFailed(ConnectError::Unreachable)
        }
        other => ImapError::ProtocolError(other.to_string()),
    }
}

// ---- Stream enum ----------------------------------------------------------

/// A wrapper enum so that `Session` can be generic over a single type
/// regardless of whether TLS is used.
#[derive(Debug)]
pub(crate) enum ImapStream {
    Tls(async_native_tls::TlsStream<tokio::net::TcpStream>),
    Plain(tokio::net::TcpStream),
}

impl tokio::io::AsyncRead for ImapStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            ImapStream::Tls(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            ImapStream::Plain(s) => std::pin::Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl tokio::io::AsyncWrite for ImapStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            ImapStream::Tls(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            ImapStream::Plain(s) => std::pin::Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            ImapStream::Tls(s) => std::pin::Pin::new(s).poll_flush(cx),
            ImapStream::Plain(s) => std::pin::Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            ImapStream::Tls(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            ImapStream::Plain(s) => std::pin::Pin::new(s).poll_shutdown(cx),
        }
    }
}
