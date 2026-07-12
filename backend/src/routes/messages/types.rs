use serde::{Deserialize, Serialize};

use crate::folder_cipher::FolderId;

pub use crate::imap::types::PgpMessageStatus;

pub use crate::email_theme::EmailTheme;

/// Default page size for paginated list queries.
pub fn default_per_page() -> u32 {
    50
}

/// Query parameters for `GET /api/folders/:folder/messages`.
#[derive(Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub(crate) struct ListMessagesQuery {
    #[serde(default)]
    pub(crate) page: u32,
    #[serde(default = "default_per_page")]
    pub(crate) per_page: u32,
}

/// Request body for `PATCH /api/messages/:folder/:uid/flags`.
#[derive(Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub(crate) struct UpdateFlagsRequest {
    pub(crate) flags: Vec<String>,
    pub(crate) add: bool,
}

/// Request body for `POST /api/messages/move`.
#[derive(Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub(crate) struct MoveMessageRequest {
    pub(crate) from_folder: FolderId,
    pub(crate) to_folder: FolderId,
    pub(crate) uid: u32,
}

/// Request body for `PATCH /api/messages/:folder/flags/bulk`.
#[derive(Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub(crate) struct BulkUpdateFlagsRequest {
    pub(crate) uids: Vec<u32>,
    pub(crate) flags: Vec<String>,
    pub(crate) add: bool,
}

/// Request body for `POST /api/messages/move/bulk`.
#[derive(Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub(crate) struct BulkMoveMessagesRequest {
    pub(crate) from_folder: FolderId,
    pub(crate) to_folder: FolderId,
    pub(crate) uids: Vec<u32>,
}

/// Request body for `POST /api/messages/:folder/delete/bulk`.
#[derive(Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub(crate) struct BulkDeleteMessagesRequest {
    pub(crate) uids: Vec<u32>,
}

/// Response for `PATCH /api/messages/:folder/flags/bulk`,
/// `POST /api/messages/move/bulk`, and `POST /api/messages/:folder/delete/bulk`.
///
/// A 200 here means the request was processed, not that every UID matched a
/// message. IMAP UID commands silently skip UIDs that don't exist in the
/// mailbox rather than erroring, and our local cache mirrors that: any
/// requested UID with no matching row (already deleted elsewhere, stale
/// client state, a bad ID) is reported back here instead of being ignored.
#[derive(Serialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub(crate) struct BulkMessageOpResponse {
    /// UIDs from the request that did not correspond to a cached message in
    /// the folder. Empty when every UID was found.
    pub(crate) failed_uids: Vec<u32>,
}

/// Response envelope for `GET /api/folders/:folder/messages`.
#[derive(Serialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub(crate) struct ListMessagesResponse {
    pub(crate) messages: Vec<MessageSummary>,
    pub(crate) total_count: u32,
    pub(crate) page: u32,
    pub(crate) per_page: u32,
    pub(crate) syncing: bool,
}

/// A message summary in the list response. Represents the latest message in a
/// thread, with per-thread aggregate counts for UI grouping.
#[derive(Serialize, Clone)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct MessageSummary {
    pub uid: u32,
    /// Opaque, single-use token for this exact response. Pass back verbatim
    /// in the next request touching this message (flags, delete, tags,
    /// attachments). Never compare or cache it - a fresh value is minted on
    /// every response, even for two messages in the same folder.
    pub folder_id: FolderId,
    /// Plaintext, stable folder name. Use for display, comparisons, and
    /// cache keys - never send this back to the server as a path/body value.
    pub folder_name: String,
    pub subject: String,
    pub from_address: String,
    pub from_name: String,
    pub to_addresses: String,
    pub date: String,
    pub flags: String,
    pub size: u32,
    pub has_attachments: bool,
    pub snippet: String,
    pub reaction: Option<String>,
    pub tags: Vec<crate::db::tags::MessageTag>,
    /// Total number of messages in this thread within the folder.
    pub thread_count: u32,
    /// Number of unread messages in this thread.
    pub unread_count: u32,
}

/// An email address entry for the detail response.
#[derive(Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub(crate) struct AddressEntry {
    pub(crate) name: Option<String>,
    pub(crate) address: String,
}

/// Parse a JSON-encoded address list string (e.g. from the SQLite cache) into
/// a `Vec<AddressEntry>`. Returns an empty vec on parse failure.
pub(crate) fn parse_address_list(json_str: &str) -> Vec<AddressEntry> {
    serde_json::from_str(json_str).unwrap_or_default()
}

/// Split a comma-separated flags string into a `Vec<String>`.
pub(crate) fn parse_flags(flags_csv: &str) -> Vec<String> {
    if flags_csv.is_empty() {
        return vec![];
    }
    flags_csv
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Response for `GET /api/messages/:folder/:uid`.
#[derive(Serialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub(crate) struct MessageDetailResponse {
    pub(crate) uid: u32,
    /// Opaque, single-use token - see `MessageSummary::folder_id`.
    pub(crate) folder_id: FolderId,
    /// Plaintext, stable folder name - see `MessageSummary::folder_name`.
    pub(crate) folder_name: String,
    pub(crate) subject: String,
    pub(crate) from_address: String,
    pub(crate) from_name: String,
    pub(crate) to_addresses: Vec<AddressEntry>,
    pub(crate) cc_addresses: Vec<AddressEntry>,
    pub(crate) date: String,
    pub(crate) flags: Vec<String>,
    pub(crate) has_attachments: bool,
    pub(crate) html: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) raw_headers: String,
    pub(crate) attachments: Vec<AttachmentMeta>,
    pub(crate) thread: Vec<ThreadMessage>,
    pub(crate) email_theme: Option<EmailTheme>,
    pub(crate) pgp_status: Option<PgpMessageStatus>,
}

/// A message summary within a thread.
#[derive(Serialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub(crate) struct ThreadMessage {
    pub(crate) uid: u32,
    /// Opaque, single-use token - see `MessageSummary::folder_id`.
    pub(crate) folder_id: FolderId,
    /// Plaintext, stable folder name - see `MessageSummary::folder_name`.
    pub(crate) folder_name: String,
    pub(crate) message_id: Option<String>,
    pub(crate) in_reply_to: Option<String>,
    pub(crate) subject: String,
    pub(crate) from_address: String,
    pub(crate) from_name: String,
    pub(crate) to_addresses: String,
    pub(crate) cc_addresses: String,
    pub(crate) date: String,
    pub(crate) flags: String,
    pub(crate) size: u32,
    pub(crate) has_attachments: bool,
    pub(crate) snippet: String,
}

/// Attachment metadata (without the binary data).
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub(crate) struct AttachmentMeta {
    pub(crate) id: String,
    pub(crate) filename: Option<String>,
    pub(crate) content_type: String,
    pub(crate) size: usize,
    pub(crate) content_id: Option<String>,
}
