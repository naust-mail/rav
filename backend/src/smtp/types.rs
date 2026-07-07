//! SMTP data types.

use serde::Deserialize;

/// Parameters needed to establish an SMTP connection.
/// Passed explicitly to every trait method so the trait stays stateless.
#[derive(Clone)]
pub struct SmtpCredentials {
    /// TLS SNI hostname, also used in Message-ID generation.
    pub host: String,
    /// TCP connect address. May differ from host to avoid hairpin NAT.
    /// When equal to host, behaves identically to the original single-host design.
    pub connect_host: String,
    pub port: u16,
    pub tls: bool,
    pub email: String,
    pub password: String,
    /// Pre-built TLS parameters from MailTransport. Includes any custom CA cert.
    /// None falls back to lettre's default relay builders (system roots).
    pub tls_params: Option<lettre::transport::smtp::client::TlsParameters>,
}

// Manual Debug impl: TlsParameters doesn't implement Debug, and we omit password.
impl std::fmt::Debug for SmtpCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpCredentials")
            .field("host", &self.host)
            .field("connect_host", &self.connect_host)
            .field("port", &self.port)
            .field("tls", &self.tls)
            .field("email", &self.email)
            .field("tls_params", &self.tls_params.as_ref().map(|_| "TlsParameters(..)"))
            .finish()
    }
}

/// Whether to sign or sign+encrypt an outbound PGP/MIME message.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PgpMode {
    Sign,
    Encrypt,
}

/// Parameters for PGP/MIME message wrapping.
/// Set by the client after performing crypto in the browser worker.
#[derive(Debug, Clone, Deserialize)]
pub struct PgpSendParams {
    pub mode: PgpMode,
    /// Armored detached signature (for Sign mode).
    pub signature: Option<String>,
    /// Armored PGP MESSAGE ciphertext (for Encrypt mode).
    pub ciphertext: Option<String>,
    /// micalg value for multipart/signed (e.g. "pgp-sha256").
    pub micalg: String,
}

/// A message ready to be sent via SMTP.
#[derive(Debug, Clone)]
pub struct SendableMessage {
    /// Sender email address.
    pub from: String,
    /// Primary recipients.
    pub to: Vec<String>,
    /// CC recipients.
    pub cc: Vec<String>,
    /// BCC recipients.
    pub bcc: Vec<String>,
    /// Subject line.
    pub subject: String,
    /// Plain-text body.
    pub text_body: String,
    /// Optional HTML body.
    pub html_body: Option<String>,
    /// In-Reply-To header value for threading.
    pub in_reply_to: Option<String>,
    /// References header value for threading.
    pub references: Option<String>,
    /// File attachments.
    pub attachments: Vec<AttachmentData>,
    /// When true, adds `Auto-Submitted: auto-replied` per RFC 3834.
    /// Set on automated replies (vacation responder) to prevent mail loops.
    pub auto_submitted: bool,
    /// If set, wrap outbound message in PGP/MIME.
    pub pgp: Option<PgpSendParams>,
}

/// A single file attachment to include in an outgoing message.
#[derive(Debug, Clone)]
pub struct AttachmentData {
    /// Filename as it should appear to the recipient.
    pub filename: String,
    /// MIME content type (e.g. "application/pdf").
    pub content_type: String,
    /// Raw file content.
    pub data: Vec<u8>,
    /// Optional Content-ID for inline images (referenced via `cid:` in HTML).
    /// When set, the attachment is treated as an inline image rather than a
    /// regular file attachment.
    pub content_id: Option<String>,
}
