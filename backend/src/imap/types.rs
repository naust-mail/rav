use serde::{Deserialize, Serialize};

/// Represents an IMAP folder (mailbox).
#[derive(Debug, Clone, Serialize)]
pub struct ImapFolder {
    /// Folder name as returned by the IMAP server (e.g. "INBOX", "Sent").
    pub name: String,
    /// Delimiter used by the server (e.g. "/" or ".").
    pub delimiter: Option<String>,
    /// IMAP attributes for this folder (e.g. `\Noselect`, `\HasChildren`).
    pub attributes: Vec<String>,
}

/// A parsed email address with optional display name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailAddress {
    /// Display name, if present (e.g. "Alice Smith").
    pub name: Option<String>,
    /// The actual email address (e.g. "alice@example.com").
    pub address: String,
}

/// A lightweight summary of an email message (envelope data).
#[derive(Debug, Clone, Serialize)]
pub struct ImapMessageHeader {
    /// IMAP UID of the message within its folder.
    pub uid: u32,
    /// Subject line.
    pub subject: Option<String>,
    /// Sender(s) of the message.
    pub from: Vec<EmailAddress>,
    /// Recipient(s) of the message.
    pub to: Vec<EmailAddress>,
    /// Date header value (raw string from the server).
    pub date: Option<String>,
    /// Resolved sort epoch: parsed from the Date header, falling back to INTERNALDATE.
    /// Zero only if both are absent/unparseable.
    pub date_epoch: i64,
    /// IMAP flags currently set on this message (e.g. `\Seen`, `\Flagged`).
    pub flags: Vec<String>,
    /// Whether this message has attachments (derived from BODYSTRUCTURE).
    pub has_attachments: bool,
    /// RFC 2822 size of the message in bytes.
    pub size: u32,
    /// Message-ID header value for threading.
    pub message_id: Option<String>,
    /// In-Reply-To header value for threading.
    pub in_reply_to: Option<String>,
    /// References header value for threading.
    pub references: Option<String>,
    /// CC addresses.
    pub cc: Vec<EmailAddress>,
    /// Reaction emoji if this is an Outlook/Exchange reaction notification.
    pub reaction: Option<String>,
}

/// Whether a message is PGP encrypted, signed, or both.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PgpStatusKind {
    Encrypted,
    Signed,
    #[allow(dead_code)]
    SignedAndEncrypted,
}

/// PGP status detected from the top-level MIME structure of a received message.
#[derive(Debug, Clone, Serialize)]
pub struct PgpMessageStatus {
    pub kind: PgpStatusKind,
    /// Armored PGP ciphertext (for Encrypted kind). The client decrypts this.
    pub ciphertext: Option<String>,
    /// Armored detached signature (for Signed kind). The client verifies this.
    pub signature: Option<String>,
    /// micalg value from multipart/signed (e.g. "pgp-sha256").
    pub micalg: Option<String>,
    /// Base64-encoded raw bytes of the signed MIME body part (for Signed kind).
    /// RFC 3156 signatures are computed over the complete first body part
    /// including its MIME headers; `message.text` alone is not sufficient.
    pub signed_content: Option<String>,
}

/// The full body of an email message, including attachments.
#[derive(Debug, Clone, Serialize)]
pub struct ImapMessageBody {
    /// IMAP UID of the message within its folder.
    pub uid: u32,
    /// Plain-text body part, if available.
    pub text_plain: Option<String>,
    /// HTML body part, if available.
    pub text_html: Option<String>,
    /// List of attachments found in the message.
    pub attachments: Vec<ImapAttachment>,
    /// Raw RFC 822 headers as a single string.
    pub raw_headers: String,
    /// PGP status if the message uses PGP/MIME structure.
    pub pgp_status: Option<PgpMessageStatus>,
}

/// Metadata about a single attachment in an email message.
#[derive(Debug, Clone, Serialize)]
pub struct ImapAttachment {
    /// Filename of the attachment, if provided by the sender.
    pub filename: Option<String>,
    /// MIME content type (e.g. "application/pdf").
    pub content_type: String,
    /// Size in bytes.
    pub size: usize,
    /// Raw attachment content.
    pub data: Vec<u8>,
    /// Content-ID for inline images (e.g. "image001@01D1234"), without angle brackets.
    pub content_id: Option<String>,
}

/// Parameters needed to establish an IMAP connection.
/// Passed explicitly to every trait method so the trait stays stateless.
#[derive(Debug, Clone)]
pub struct ImapCredentials {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub email: String,
    pub password: String,
}

/// Lightweight result of an IMAP `SELECT` command.
#[derive(Debug, Clone, Serialize)]
pub struct FolderStatus {
    /// UIDVALIDITY - changes when the mailbox is rebuilt or UIDs are reassigned.
    pub uid_validity: u32,
    /// The total number of messages currently in the folder.
    pub exists: u32,
    /// The highest UID that will be assigned to the next appended message.
    pub uid_next: u32,
}

/// Mailbox quota information from IMAP GETQUOTAROOT.
#[derive(Debug, Clone, Serialize)]
pub struct MailboxQuota {
    /// Storage used in bytes (STORAGE resource reports in KB, we convert).
    pub usage_bytes: u64,
    /// Storage limit in bytes.
    pub limit_bytes: u64,
}

/// Extended folder status from an IMAP `STATUS` command with CONDSTORE fields.
/// Used for cheap pre-checks before full sync.
#[derive(Debug, Clone, Serialize)]
pub struct FolderStatusExtended {
    pub uid_validity: u32,
    pub exists: u32,
    pub uid_next: u32,
    pub unseen: u32,
    pub highest_modseq: u64,
}
