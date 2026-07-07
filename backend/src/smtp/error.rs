//! SMTP error types.

use std::fmt;

use crate::error::ConnectError;

/// Errors that can occur during SMTP operations.
#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
pub enum SmtpError {
    /// Could not connect to the SMTP server.
    ConnectionFailed(ConnectError),
    /// The server rejected our credentials.
    AuthenticationFailed,
    /// The message could not be sent.
    SendFailed(String),
}

impl fmt::Display for SmtpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmtpError::ConnectionFailed(e) => write!(f, "{e}"),
            SmtpError::AuthenticationFailed => write!(f, "Authentication failed"),
            SmtpError::SendFailed(msg) => write!(f, "Send failed: {msg}"),
        }
    }
}

impl std::error::Error for SmtpError {}
