use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;

use super::connection::map_imap_error;
use super::parse::{
    decode_rfc2047, flag_to_string, has_attachments, imap_address_to_email, name_attribute_to_string,
};
use super::session_cache::SessionCache;
use crate::mail_transport::MailTransport;

pub use super::error::ImapError;
pub use super::types::{
    EmailAddress, FolderStatus, FolderStatusExtended, ImapAttachment, ImapCredentials, ImapFolder,
    ImapMessageBody, ImapMessageHeader, MailboxQuota,
};

pub(crate) use super::connection::connect;

/// IMAP fetch items used for bulk header sync. Both the polling path (client.rs) and the
/// IDLE push path (idle.rs) must use this constant so the set of fetched attributes stays
/// in sync.
pub(crate) const HEADER_FETCH_ITEMS: &str =
    "(UID ENVELOPE INTERNALDATE FLAGS BODYSTRUCTURE RFC822.SIZE BODY.PEEK[HEADER.FIELDS \
     (Message-ID In-Reply-To References Content-Class x-ms-exchange-generated-message-class)])";

#[cfg(test)]
#[path = "mock.rs"]
pub mod mock;

/// Abstraction over IMAP operations.
///
/// Every method receives explicit connection parameters so that the trait
/// remains stateless — no persistent connections are held.
///
/// The `Send + Sync` bounds allow implementations to be shared across
/// Tokio tasks and stored in `Arc`.
#[allow(dead_code)]
#[async_trait]
pub trait ImapClient: Send + Sync {
    /// List all folders (mailboxes) on the server.
    async fn list_folders(&self, creds: &ImapCredentials) -> Result<Vec<ImapFolder>, ImapError>;

    /// Perform a lightweight `SELECT` on a folder to get its status
    /// (UIDVALIDITY, EXISTS count, UIDNEXT) without fetching any messages.
    async fn folder_status(
        &self,
        creds: &ImapCredentials,
        folder: &str,
    ) -> Result<FolderStatus, ImapError>;

    /// Fetch message headers (envelopes) for a range of UIDs in a folder.
    async fn fetch_headers(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid_range: &str,
    ) -> Result<Vec<ImapMessageHeader>, ImapError>;

    /// Fetch the full body of a single message by UID.
    async fn fetch_body(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid: u32,
    ) -> Result<ImapMessageBody, ImapError>;

    /// Add flags to a message (IMAP +FLAGS).
    async fn add_flags(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid: u32,
        flags: &[&str],
    ) -> Result<(), ImapError>;

    /// Remove flags from a message (IMAP -FLAGS).
    async fn remove_flags(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid: u32,
        flags: &[&str],
    ) -> Result<(), ImapError>;

    /// Set (replace) the flags on a message.
    async fn set_flags(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid: u32,
        flags: &[&str],
    ) -> Result<(), ImapError>;

    /// Move a message from one folder to another.
    async fn move_message(
        &self,
        creds: &ImapCredentials,
        from_folder: &str,
        uid: u32,
        to_folder: &str,
    ) -> Result<(), ImapError>;

    /// Permanently remove a message that has the `\Deleted` flag.
    async fn expunge_message(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid: u32,
    ) -> Result<(), ImapError>;

    /// Append a raw RFC822 message to a folder.
    ///
    /// If `message_id` is provided, the folder is selected after the append and a UID SEARCH
    /// by `Message-ID` header is performed to retrieve the assigned UID. Returns `Some(uid)`
    /// on success, `None` if the search returned no results or `message_id` was not given.
    async fn append_message(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        message_bytes: &[u8],
        flags: &[&str],
        message_id: Option<&str>,
    ) -> Result<Option<u32>, ImapError>;

    /// Create a new folder (mailbox) and subscribe to it.
    async fn create_folder(
        &self,
        creds: &ImapCredentials,
        folder_name: &str,
    ) -> Result<(), ImapError>;

    /// Rename an existing folder.
    async fn rename_folder(
        &self,
        creds: &ImapCredentials,
        from: &str,
        to: &str,
    ) -> Result<(), ImapError>;

    /// Permanently delete a folder (mailbox).
    async fn delete_folder(
        &self,
        creds: &ImapCredentials,
        folder_name: &str,
    ) -> Result<(), ImapError>;

    /// Subscribe to or unsubscribe from a folder.
    async fn subscribe_folder(
        &self,
        creds: &ImapCredentials,
        folder_name: &str,
        subscribe: bool,
    ) -> Result<(), ImapError>;

    /// Fetch only UIDs and FLAGS for all messages in a folder.
    /// Used for periodic reconciliation to detect flag changes and deletions.
    async fn fetch_uids_and_flags(
        &self,
        creds: &ImapCredentials,
        folder: &str,
    ) -> Result<Vec<(u32, Vec<String>)>, ImapError>;

    /// Lightweight STATUS command to get folder metadata without SELECT.
    /// Returns UIDVALIDITY, EXISTS, UIDNEXT, UNSEEN, and HIGHESTMODSEQ.
    async fn folder_status_extended(
        &self,
        creds: &ImapCredentials,
        folder: &str,
    ) -> Result<FolderStatusExtended, ImapError>;

    /// Fetch only messages whose flags changed since `since_modseq` using CONDSTORE.
    /// Returns (vec of (uid, flags), new_highest_modseq).
    async fn fetch_changed_flags(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        since_modseq: u64,
    ) -> Result<(Vec<(u32, Vec<String>)>, u64), ImapError>;

    /// Fetch mailbox quota via IMAP GETQUOTAROOT.
    /// Returns `None` if the server doesn't support quotas.
    async fn get_quota(
        &self,
        creds: &ImapCredentials,
    ) -> Result<Option<MailboxQuota>, ImapError>;

    /// Fetch the total size of all messages in a folder via UID FETCH 1:* (RFC822.SIZE).
    async fn fetch_folder_size(
        &self,
        creds: &ImapCredentials,
        folder: &str,
    ) -> Result<u64, ImapError>;

    /// Mark every message in a folder as read via UID STORE 1:* +FLAGS (\Seen).
    async fn mark_all_read(
        &self,
        creds: &ImapCredentials,
        folder: &str,
    ) -> Result<(), ImapError>;

    /// Fetch the raw RFC 822 bytes of a single message by UID.
    /// Used to submit the message to rspamd for spam/ham learning.
    async fn fetch_raw_bytes(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid: u32,
    ) -> Result<Vec<u8>, ImapError>;
}

// ---------------------------------------------------------------------------
// Real implementation backed by async-imap
// ---------------------------------------------------------------------------

/// Production IMAP client that uses `async-imap` and `mail-parser`.
///
/// Holds a session cache so that one authenticated connection per account is
/// reused across consecutive requests instead of creating a new TCP+TLS+LOGIN
/// sequence every time.
pub struct RealImapClient {
    cache: SessionCache,
    transport: Arc<MailTransport>,
}

impl RealImapClient {
    pub fn new(transport: Arc<MailTransport>) -> Self {
        RealImapClient {
            cache: SessionCache::new(),
            transport,
        }
    }
}


// ---- Trait implementation -------------------------------------------------

#[async_trait]
impl ImapClient for RealImapClient {
    async fn folder_status(
        &self,
        creds: &ImapCredentials,
        folder: &str,
    ) -> Result<FolderStatus, ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;

        let mailbox = session
            .select(folder)
            .await
            .map_err(|e| match &e {
                async_imap::error::Error::No(msg)
                    if msg.to_lowercase().contains("not found")
                        || msg.to_lowercase().contains("doesn't exist")
                        || msg.to_lowercase().contains("does not exist")
                        || msg.to_lowercase().contains("no such") =>
                {
                    ImapError::FolderNotFound(folder.to_string())
                }
                _ => map_imap_error(e),
            })?;

        let uid_validity = mailbox.uid_validity.unwrap_or(0);
        let exists = mailbox.exists;
        let uid_next = mailbox.uid_next.unwrap_or(0);

        self.cache.release(creds, session);
        Ok(FolderStatus {
            uid_validity,
            exists,
            uid_next,
        })
    }

    async fn list_folders(&self, creds: &ImapCredentials) -> Result<Vec<ImapFolder>, ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;

        let folders = {
            let names_stream = session
                .list(Some(""), Some("*"))
                .await
                .map_err(map_imap_error)?;

            let mut names_stream = std::pin::pin!(names_stream);
            let mut names = Vec::new();
            while let Some(result) = names_stream.next().await {
                names.push(result.map_err(map_imap_error)?);
            }

            names
                .iter()
                .filter(|n| {
                    !n.attributes()
                        .iter()
                        .any(|a| matches!(a, async_imap::types::NameAttribute::NoSelect))
                })
                .map(|n| ImapFolder {
                    name: n.name().to_string(),
                    delimiter: n.delimiter().map(|d| d.to_string()),
                    attributes: n
                        .attributes()
                        .iter()
                        .map(name_attribute_to_string)
                        .collect(),
                })
                .collect()
        };

        self.cache.release(creds, session);
        Ok(folders)
    }

    async fn fetch_headers(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid_range: &str,
    ) -> Result<Vec<ImapMessageHeader>, ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;

        session
            .select(folder)
            .await
            .map_err(|e| match &e {
                async_imap::error::Error::No(msg)
                    if msg.to_lowercase().contains("not found")
                        || msg.to_lowercase().contains("doesn't exist")
                        || msg.to_lowercase().contains("does not exist")
                        || msg.to_lowercase().contains("no such") =>
                {
                    ImapError::FolderNotFound(folder.to_string())
                }
                _ => map_imap_error(e),
            })?;

        let headers = {
            // Fetch ENVELOPE, FLAGS, BODYSTRUCTURE, RFC822.SIZE, and threading headers.
            // We only fetch Message-ID, In-Reply-To, and References (a few bytes per message)
            // rather than full raw headers, to keep bulk syncs lightweight.
            let mut fetch_stream = session
                .uid_fetch(uid_range, HEADER_FETCH_ITEMS)
                .await
                .map_err(map_imap_error)?;

            let mut fetches = Vec::new();
            while let Some(result) = fetch_stream.next().await {
                fetches.push(result.map_err(map_imap_error)?);
            }

            let mut headers = Vec::with_capacity(fetches.len());
            for fetch in &fetches {
                let uid = match fetch.uid {
                    Some(u) => u,
                    None => continue,
                };

                // Parse threading headers from the small HEADER.FIELDS response.
                let raw_header_bytes = fetch.header();
                let parsed_threading = raw_header_bytes.and_then(|raw| {
                    mail_parser::MessageParser::default().parse(raw)
                });

                let (subject, from, to, cc, date) = if let Some(env) = fetch.envelope() {
                    let subject = env
                        .subject
                        .as_ref()
                        .and_then(|b| std::str::from_utf8(b).ok())
                        .map(decode_rfc2047);

                    let from: Vec<EmailAddress> = env
                        .from
                        .as_ref()
                        .map(|addrs| addrs.iter().map(imap_address_to_email).collect())
                        .unwrap_or_default();

                    let to: Vec<EmailAddress> = env
                        .to
                        .as_ref()
                        .map(|addrs| addrs.iter().map(imap_address_to_email).collect())
                        .unwrap_or_default();

                    let cc: Vec<EmailAddress> = env
                        .cc
                        .as_ref()
                        .map(|addrs| addrs.iter().map(imap_address_to_email).collect())
                        .unwrap_or_default();

                    let date = env
                        .date
                        .as_ref()
                        .and_then(|b| std::str::from_utf8(b).ok())
                        .map(|s| s.to_string());

                    (subject, from, to, cc, date)
                } else {
                    // No envelope — we can't fill subject/from/to/date from the
                    // small threading-only header fetch, so leave them empty.
                    // They'll be populated when the user opens the message body.
                    tracing::warn!(
                        uid = uid,
                        folder = %folder,
                        "ENVELOPE missing for message, headers will be empty until body is fetched"
                    );
                    (None, vec![], vec![], vec![], None)
                };

                let date_epoch = {
                    let from_header = date.as_deref()
                        .map(crate::db::messages::parse_date_epoch)
                        .unwrap_or(0);
                    if from_header > 0 {
                        from_header
                    } else {
                        fetch.internal_date().map(|d| d.timestamp()).unwrap_or(0)
                    }
                };

                // Extract threading headers from the small HEADER.FIELDS response.
                let message_id = parsed_threading.as_ref().and_then(|p| {
                    p.message_id().map(|s| format!("<{s}>"))
                });
                let in_reply_to = parsed_threading.as_ref().and_then(|p| {
                    let val = p.in_reply_to();
                    val.as_text().map(|s| format!("<{s}>"))
                });
                let references = parsed_threading.as_ref().and_then(|p| {
                    let val = p.references();
                    val.as_text_list()
                        .map(|list| list.iter().map(|s| format!("<{s}>")).collect::<Vec<_>>().join(" "))
                        .or_else(|| val.as_text().map(|s| format!("<{s}>")))
                });

                // Detect Outlook/Exchange reaction emails from headers.
                let reaction = raw_header_bytes.and_then(|raw| {
                    let header_str = std::str::from_utf8(raw).ok()?;
                    let lower = header_str.to_lowercase();
                    let is_reaction = lower.contains("content-class: activitynotification")
                        || lower.contains("urn:content-class:reaction");
                    if !is_reaction {
                        return None;
                    }
                    let subj = subject.as_deref().unwrap_or("").to_lowercase();
                    let emoji = match subj.trim() {
                        s if s.contains("like") => "\u{1f44d}",
                        s if s.contains("heart") || s.contains("love") => "\u{2764}\u{fe0f}",
                        s if s.contains("laugh") => "\u{1f604}",
                        s if s.contains("surprised") || s.contains("wow") => "\u{1f62e}",
                        s if s.contains("sad") => "\u{1f622}",
                        s if s.contains("angry") => "\u{1f620}",
                        _ => "\u{1f44d}",
                    };
                    Some(emoji.to_string())
                });

                let flags: Vec<String> = fetch.flags().map(|f| flag_to_string(&f)).collect();

                let has_attach = fetch
                    .bodystructure()
                    .map(|bs| has_attachments(bs))
                    .unwrap_or(false);

                let size = fetch.size.unwrap_or(0);

                headers.push(ImapMessageHeader {
                    uid,
                    subject,
                    from,
                    to,
                    date,
                    date_epoch,
                    flags,
                    has_attachments: has_attach,
                    size,
                    message_id,
                    in_reply_to,
                    references,
                    cc,
                    reaction,
                });
            }
            headers
        };

        self.cache.release(creds, session);
        Ok(headers)
    }

    async fn fetch_body(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid: u32,
    ) -> Result<ImapMessageBody, ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;

        session
            .select(folder)
            .await
            .map_err(|e| match &e {
                async_imap::error::Error::No(msg)
                    if msg.to_lowercase().contains("not found")
                        || msg.to_lowercase().contains("doesn't exist")
                        || msg.to_lowercase().contains("does not exist")
                        || msg.to_lowercase().contains("no such") =>
                {
                    ImapError::FolderNotFound(folder.to_string())
                }
                _ => map_imap_error(e),
            })?;

        let uid_str = uid.to_string();
        let body = {
            let mut fetch_stream = session
                .uid_fetch(&uid_str, "(UID BODY[])")
                .await
                .map_err(map_imap_error)?;

            let mut fetches = Vec::new();
            while let Some(result) = fetch_stream.next().await {
                fetches.push(result.map_err(map_imap_error)?);
            }

            let fetch = fetches.first().ok_or(ImapError::MessageNotFound {
                uid,
                folder: folder.to_string(),
            })?;

            let raw = fetch.body().ok_or_else(|| {
                ImapError::ProtocolError("BODY[] not returned by server".to_string())
            })?;

            use mail_parser::MimeHeaders;

            let parsed = mail_parser::MessageParser::default()
                .parse(raw)
                .ok_or_else(|| {
                    ImapError::ProtocolError("failed to parse RFC822 message".to_string())
                })?;

            let text_plain: Option<String> = parsed.body_text(0).map(|s| s.to_string());

            let has_html_part = parsed.parts.iter().any(|part| {
                part.content_type().is_some_and(|ct| ct.ctype() == "text" && ct.subtype() == Some("html"))
            });

            let text_html: Option<String> = if has_html_part {
                parsed.body_html(0).map(|s| s.to_string())
            } else {
                None
            };

            tracing::debug!(
                uid = uid,
                folder = %folder,
                total_parts = parsed.parts.len(),
                attachment_count = parsed.attachments().count(),
                has_text = text_plain.is_some(),
                has_html = text_html.is_some(),
                "fetch_body: parsed message structure"
            );

            let mut attachments = Vec::new();

            // Collect explicit attachments.
            for attachment in parsed.attachments() {
                let filename: Option<String> =
                    attachment.attachment_name().map(|s| s.to_string());
                let content_type: String = attachment.content_type().map_or_else(
                    || "application/octet-stream".to_string(),
                    |ct: &mail_parser::ContentType<'_>| {
                        if let Some(subtype) = ct.subtype() {
                            format!("{}/{}", ct.ctype(), subtype)
                        } else {
                            ct.ctype().to_string()
                        }
                    },
                );
                let content_id = attachment
                    .content_id()
                    .map(|s| s.trim_matches(|c| c == '<' || c == '>').to_string());
                let data = attachment.contents().to_vec();
                let size = data.len();
                attachments.push(ImapAttachment {
                    filename,
                    content_type,
                    size,
                    data,
                    content_id,
                });
            }

            // Also collect inline parts with Content-ID (e.g. embedded images
            // referenced via cid: URLs in the HTML body).
            for part in parsed.parts.iter() {
                if part.content_id().is_none() {
                    continue;
                }
                // Skip if this is a text/html or text/plain body part.
                let is_text = part
                    .content_type()
                    .is_some_and(|ct| ct.ctype() == "text");
                if is_text {
                    continue;
                }
                let cid = part
                    .content_id()
                    .unwrap()
                    .trim_matches(|c| c == '<' || c == '>')
                    .to_string();
                // Skip if we already captured this part via attachments().
                if attachments.iter().any(|a| a.content_id.as_deref() == Some(&cid)) {
                    continue;
                }
                let content_type: String = part.content_type().map_or_else(
                    || "application/octet-stream".to_string(),
                    |ct: &mail_parser::ContentType<'_>| {
                        if let Some(subtype) = ct.subtype() {
                            format!("{}/{}", ct.ctype(), subtype)
                        } else {
                            ct.ctype().to_string()
                        }
                    },
                );
                let data = part.contents().to_vec();
                let size = data.len();
                attachments.push(ImapAttachment {
                    filename: part.attachment_name().map(|s| s.to_string()),
                    content_type,
                    size,
                    data,
                    content_id: Some(cid),
                });
            }

            // Extract raw headers from the RFC 822 message.
            let raw_str = String::from_utf8_lossy(raw);
            let raw_headers = raw_str
                .split_once("\r\n\r\n")
                .or_else(|| raw_str.split_once("\n\n"))
                .map_or_else(|| raw_str.to_string(), |(h, _)| h.to_string());

            // Detect PGP/MIME structure from top-level content type.
            use crate::imap::types::PgpStatusKind;
            let pgp_status = detect_pgp_mime(&parsed);

            // For encrypted messages, clear body and attachments — the client decrypts.
            let (text_plain, text_html, attachments) = if pgp_status
                .as_ref()
                .is_some_and(|s| s.kind == PgpStatusKind::Encrypted)
            {
                (None, None, vec![])
            } else {
                (text_plain, text_html, attachments)
            };

            ImapMessageBody {
                uid,
                text_plain,
                text_html,
                attachments,
                raw_headers,
                pgp_status,
            }
        };

        self.cache.release(creds, session);
        Ok(body)
    }

    async fn add_flags(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid: u32,
        flags: &[&str],
    ) -> Result<(), ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;
        session.select(folder).await.map_err(map_imap_error)?;

        let uid_str = uid.to_string();
        let flags_str = format!("+FLAGS ({})", flags.join(" "));
        {
            let mut store_stream = session
                .uid_store(&uid_str, &flags_str)
                .await
                .map_err(map_imap_error)?;
            while let Some(result) = store_stream.next().await {
                result.map_err(map_imap_error)?;
            }
        }

        self.cache.release(creds, session);
        Ok(())
    }

    async fn remove_flags(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid: u32,
        flags: &[&str],
    ) -> Result<(), ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;
        session.select(folder).await.map_err(map_imap_error)?;

        let uid_str = uid.to_string();
        let flags_str = format!("-FLAGS ({})", flags.join(" "));
        {
            let mut store_stream = session
                .uid_store(&uid_str, &flags_str)
                .await
                .map_err(map_imap_error)?;
            while let Some(result) = store_stream.next().await {
                result.map_err(map_imap_error)?;
            }
        }

        self.cache.release(creds, session);
        Ok(())
    }

    async fn set_flags(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid: u32,
        flags: &[&str],
    ) -> Result<(), ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;

        session.select(folder).await.map_err(map_imap_error)?;

        let uid_str = uid.to_string();
        let flags_str = format!("FLAGS ({})", flags.join(" "));
        {
            let mut store_stream = session
                .uid_store(&uid_str, &flags_str)
                .await
                .map_err(map_imap_error)?;

            // Consume the stream to completion so the command finishes.
            while let Some(result) = store_stream.next().await {
                result.map_err(map_imap_error)?;
            }
        }

        self.cache.release(creds, session);
        Ok(())
    }

    async fn move_message(
        &self,
        creds: &ImapCredentials,
        from_folder: &str,
        uid: u32,
        to_folder: &str,
    ) -> Result<(), ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;

        session
            .select(from_folder)
            .await
            .map_err(map_imap_error)?;

        let uid_str = uid.to_string();

        // Try UID MOVE first; fall back to COPY + DELETE + EXPUNGE if the
        // server does not support the MOVE extension.
        match session.uid_mv(&uid_str, to_folder).await {
            Ok(()) => {}
            Err(async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_)) => {
                // Fallback: COPY, then flag \Deleted, then EXPUNGE.
                session
                    .uid_copy(&uid_str, to_folder)
                    .await
                    .map_err(map_imap_error)?;

                {
                    let mut store_stream = session
                        .uid_store(&uid_str, "+FLAGS (\\Deleted)")
                        .await
                        .map_err(map_imap_error)?;
                    while let Some(r) = store_stream.next().await {
                        r.map_err(map_imap_error)?;
                    }
                }

                {
                    let expunge_stream =
                        session.expunge().await.map_err(map_imap_error)?;
                    let mut expunge_stream = std::pin::pin!(expunge_stream);
                    while let Some(r) = expunge_stream.next().await {
                        r.map_err(map_imap_error)?;
                    }
                }
            }
            Err(e) => return Err(map_imap_error(e)),
        }

        self.cache.release(creds, session);
        Ok(())
    }

    async fn expunge_message(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid: u32,
    ) -> Result<(), ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;

        session.select(folder).await.map_err(map_imap_error)?;

        let uid_str = uid.to_string();

        // Mark the message as \Deleted.
        {
            let mut store_stream = session
                .uid_store(&uid_str, "+FLAGS (\\Deleted)")
                .await
                .map_err(map_imap_error)?;
            while let Some(r) = store_stream.next().await {
                r.map_err(map_imap_error)?;
            }
        }

        // Try UID EXPUNGE for precision; fall back to EXPUNGE.
        let uid_expunge_ok = {
            match session.uid_expunge(&uid_str).await {
                Ok(stream) => {
                    let mut stream = std::pin::pin!(stream);
                    while let Some(r) = stream.next().await {
                        r.map_err(map_imap_error)?;
                    }
                    true
                }
                Err(_) => false,
            }
        };
        if !uid_expunge_ok {
            let stream = session.expunge().await.map_err(map_imap_error)?;
            let mut stream = std::pin::pin!(stream);
            while let Some(r) = stream.next().await {
                r.map_err(map_imap_error)?;
            }
        }

        self.cache.release(creds, session);
        Ok(())
    }

    async fn append_message(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        message_bytes: &[u8],
        flags: &[&str],
        message_id: Option<&str>,
    ) -> Result<Option<u32>, ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;

        let flags_str: Vec<String> = flags.iter().map(|f| f.to_string()).collect();
        let flags_joined = if flags_str.is_empty() {
            None
        } else {
            Some(format!("({})", flags_str.join(" ")))
        };
        session
            .append(folder, flags_joined.as_deref(), None, message_bytes)
            .await
            .map_err(map_imap_error)?;

        // If the caller provided the Message-ID it embedded in the RFC822, SELECT the folder
        // and search by that header to retrieve the UID assigned by the server.
        let uid = if let Some(mid) = message_id {
            session.select(folder).await.map_err(map_imap_error)?;
            let uids = session
                .uid_search(format!("HEADER Message-ID {mid}"))
                .await
                .map_err(map_imap_error)?;
            uids.into_iter().max()
        } else {
            None
        };

        self.cache.release(creds, session);
        Ok(uid)
    }

    async fn create_folder(
        &self,
        creds: &ImapCredentials,
        folder_name: &str,
    ) -> Result<(), ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;
        session.create(folder_name).await.map_err(map_imap_error)?;
        session
            .subscribe(folder_name)
            .await
            .map_err(map_imap_error)?;
        self.cache.release(creds, session);
        Ok(())
    }

    async fn rename_folder(
        &self,
        creds: &ImapCredentials,
        from: &str,
        to: &str,
    ) -> Result<(), ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;
        session.rename(from, to).await.map_err(map_imap_error)?;
        self.cache.release(creds, session);
        Ok(())
    }

    async fn delete_folder(
        &self,
        creds: &ImapCredentials,
        folder_name: &str,
    ) -> Result<(), ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;
        session.delete(folder_name).await.map_err(map_imap_error)?;
        self.cache.release(creds, session);
        Ok(())
    }

    async fn subscribe_folder(
        &self,
        creds: &ImapCredentials,
        folder_name: &str,
        subscribe: bool,
    ) -> Result<(), ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;
        if subscribe {
            session
                .subscribe(folder_name)
                .await
                .map_err(map_imap_error)?;
        } else {
            session
                .unsubscribe(folder_name)
                .await
                .map_err(map_imap_error)?;
        }
        self.cache.release(creds, session);
        Ok(())
    }

    async fn fetch_uids_and_flags(
        &self,
        creds: &ImapCredentials,
        folder: &str,
    ) -> Result<Vec<(u32, Vec<String>)>, ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;

        session
            .select(folder)
            .await
            .map_err(|e| match &e {
                async_imap::error::Error::No(msg)
                    if msg.to_lowercase().contains("not found")
                        || msg.to_lowercase().contains("doesn't exist")
                        || msg.to_lowercase().contains("does not exist")
                        || msg.to_lowercase().contains("no such") =>
                {
                    ImapError::FolderNotFound(folder.to_string())
                }
                _ => map_imap_error(e),
            })?;

        let results = {
            let mut fetch_stream = session
                .uid_fetch("1:*", "(UID FLAGS)")
                .await
                .map_err(map_imap_error)?;

            let mut items = Vec::new();
            while let Some(result) = fetch_stream.next().await {
                let fetch = result.map_err(map_imap_error)?;
                if let Some(uid) = fetch.uid {
                    let flags: Vec<String> = fetch.flags().map(|f| flag_to_string(&f)).collect();
                    items.push((uid, flags));
                }
            }
            items
        };

        self.cache.release(creds, session);
        Ok(results)
    }

    async fn folder_status_extended(
        &self,
        creds: &ImapCredentials,
        folder: &str,
    ) -> Result<FolderStatusExtended, ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;

        let mailbox = session
            .status(folder, "(MESSAGES UIDNEXT UIDVALIDITY UNSEEN HIGHESTMODSEQ)")
            .await
            .map_err(map_imap_error)?;

        let result = FolderStatusExtended {
            uid_validity: mailbox.uid_validity.unwrap_or(0),
            exists: mailbox.exists,
            uid_next: mailbox.uid_next.unwrap_or(0),
            unseen: mailbox.unseen.unwrap_or(0),
            highest_modseq: mailbox.highest_modseq.unwrap_or(0),
        };

        self.cache.release(creds, session);
        Ok(result)
    }

    async fn fetch_changed_flags(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        since_modseq: u64,
    ) -> Result<(Vec<(u32, Vec<String>)>, u64), ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;

        let mailbox = session
            .select_condstore(folder)
            .await
            .map_err(map_imap_error)?;

        let new_modseq = mailbox.highest_modseq.unwrap_or(0);

        let items = {
            let mut fetch_stream = session
                .uid_fetch("1:*", format!("(UID FLAGS) (CHANGEDSINCE {})", since_modseq))
                .await
                .map_err(map_imap_error)?;

            let mut items = Vec::new();
            while let Some(result) = fetch_stream.next().await {
                let fetch = result.map_err(map_imap_error)?;
                if let Some(uid) = fetch.uid {
                    let flags: Vec<String> = fetch.flags().map(|f| flag_to_string(&f)).collect();
                    items.push((uid, flags));
                }
            }
            items
        };

        self.cache.release(creds, session);
        Ok((items, new_modseq))
    }

    async fn get_quota(
        &self,
        creds: &ImapCredentials,
    ) -> Result<Option<MailboxQuota>, ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;

        // Send GETQUOTAROOT INBOX — the server responds with QUOTAROOT + QUOTA lines.
        // If the server doesn't support QUOTA, it returns NO — we treat that as None.
        let req_id = match session.run_command("GETQUOTAROOT INBOX").await {
            Ok(id) => id,
            Err(_) => {
                self.cache.release(creds, session);
                return Ok(None);
            }
        };

        let mut quota_result: Option<MailboxQuota> = None;

        // Read responses until we get the tagged OK/NO/BAD (15s timeout).
        let read_result = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            async {
                loop {
                    let resp = match session.read_response().await {
                        Ok(Some(r)) => r,
                        Ok(None) => break,
                        Err(e) => {
                            tracing::warn!("GETQUOTAROOT read_response failed: {e}");
                            break;
                        }
                    };
                    match resp.parsed() {
                        async_imap::imap_proto::Response::Quota(q) => {
                            for resource in &q.resources {
                                if matches!(resource.name, async_imap::imap_proto::types::QuotaResourceName::Storage) {
                                    quota_result = Some(MailboxQuota {
                                        usage_bytes: resource.usage * 1024,
                                        limit_bytes: resource.limit * 1024,
                                    });
                                }
                            }
                        }
                        async_imap::imap_proto::Response::Done { tag, .. } if tag == &req_id => break,
                        _ => {}
                    }
                }
            },
        ).await;

        if read_result.is_err() {
            tracing::warn!("GETQUOTAROOT timed out after 15s");
        }

        self.cache.release(creds, session);
        Ok(quota_result)
    }

    async fn fetch_folder_size(
        &self,
        creds: &ImapCredentials,
        folder: &str,
    ) -> Result<u64, ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;

        let mailbox = session.select(folder).await.map_err(map_imap_error)?;
        if mailbox.exists == 0 {
            self.cache.release(creds, session);
            return Ok(0);
        }

        let mut total: u64 = 0;

        // 60s timeout for fetching all sizes in a folder.
        let fetch_result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            async {
                let mut fetch_stream = session
                    .uid_fetch("1:*", "RFC822.SIZE")
                    .await
                    .map_err(map_imap_error)?;

                while let Some(result) = fetch_stream.next().await {
                    let fetch = result.map_err(map_imap_error)?;
                    total += fetch.size.unwrap_or(0) as u64;
                }
                Ok::<(), ImapError>(())
            },
        ).await;

        match fetch_result {
            Ok(Err(e)) => {
                return Err(e);
            }
            Err(_) => {
                tracing::warn!(folder = %folder, "fetch_folder_size timed out after 60s");
            }
            _ => {}
        }

        self.cache.release(creds, session);
        Ok(total)
    }

    async fn mark_all_read(
        &self,
        creds: &ImapCredentials,
        folder: &str,
    ) -> Result<(), ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;
        let mailbox = session.select(folder).await.map_err(map_imap_error)?;
        if mailbox.exists == 0 {
            self.cache.release(creds, session);
            return Ok(());
        }
        {
            let mut store_stream = session
                .uid_store("1:*", "+FLAGS.SILENT (\\Seen)")
                .await
                .map_err(map_imap_error)?;
            while let Some(result) = store_stream.next().await {
                result.map_err(map_imap_error)?;
            }
        }
        self.cache.release(creds, session);
        Ok(())
    }

    async fn fetch_raw_bytes(
        &self,
        creds: &ImapCredentials,
        folder: &str,
        uid: u32,
    ) -> Result<Vec<u8>, ImapError> {
        let mut session = self.cache.acquire(creds, &self.transport.imap_connect_host, &self.transport.imap_connector).await?;
        session.select(folder).await.map_err(map_imap_error)?;

        let uid_str = uid.to_string();
        let mut raw = Vec::new();
        {
            let mut fetch_stream = session
                .uid_fetch(&uid_str, "BODY.PEEK[]")
                .await
                .map_err(map_imap_error)?;
            while let Some(result) = fetch_stream.next().await {
                let fetch = result.map_err(map_imap_error)?;
                if let Some(body) = fetch.body() {
                    raw = body.to_vec();
                }
            }
        }

        self.cache.release(creds, session);

        if raw.is_empty() {
            return Err(ImapError::MessageNotFound { uid, folder: folder.to_string() });
        }
        Ok(raw)
    }
}

// ---------------------------------------------------------------------------
// PGP/MIME detection helpers
// ---------------------------------------------------------------------------

/// Inspect the top-level MIME content type of a parsed message and return
/// PGP status if the message uses RFC 3156 PGP/MIME structure.
fn detect_pgp_mime(
    parsed: &mail_parser::Message<'_>,
) -> Option<crate::imap::types::PgpMessageStatus> {
    use mail_parser::MimeHeaders;
    use crate::imap::types::{PgpMessageStatus, PgpStatusKind};

    let ct = parsed.content_type()?;
    if ct.ctype() != "multipart" {
        return None;
    }

    match ct.subtype() {
        Some("encrypted")
            if ct
                .attribute("protocol")
                .is_some_and(|p| p == "application/pgp-encrypted") =>
        {
            let ciphertext = find_part_content(parsed, "application", "octet-stream");
            Some(PgpMessageStatus {
                kind: PgpStatusKind::Encrypted,
                ciphertext,
                signature: None,
                micalg: None,
                signed_content: None,
            })
        }
        Some("signed")
            if ct
                .attribute("protocol")
                .is_some_and(|p| p == "application/pgp-signature") =>
        {
            let signature = find_part_content(parsed, "application", "pgp-signature");
            let micalg = ct.attribute("micalg").map(|s| s.to_string());
            // Extract the raw first body part (including its MIME headers) so
            // the client can verify over the exact bytes that were signed per RFC 3156.
            // Part 0 is the outer multipart; part 1 is the first child (the signed body).
            // Base64-encoded because the part may contain arbitrary bytes (binary attachments).
            let signed_content = parsed.parts.get(1).and_then(|part| {
                use base64::Engine;
                let raw = parsed.raw_message.as_ref();
                let start = part.offset_header as usize;
                let end = part.offset_end as usize;
                // Use get() rather than index to avoid a panic on a malformed message
                // where mail_parser emits offsets outside the raw buffer.
                raw.get(start..end)
                    .map(|slice| base64::engine::general_purpose::STANDARD.encode(slice))
            });
            Some(PgpMessageStatus {
                kind: PgpStatusKind::Signed,
                ciphertext: None,
                signature,
                micalg,
                signed_content,
            })
        }
        _ => None,
    }
}

/// Find a MIME part matching the given type/subtype and return its contents.
fn find_part_content(
    parsed: &mail_parser::Message<'_>,
    ctype: &str,
    subtype: &str,
) -> Option<String> {
    use mail_parser::MimeHeaders;
    for part in parsed.parts.iter() {
        if let Some(ct) = part.content_type()
            && ct.ctype() == ctype && ct.subtype() == Some(subtype)
        {
            return Some(String::from_utf8_lossy(part.contents()).into_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> mail_parser::Message<'_> {
        mail_parser::MessageParser::default()
            .parse(raw.as_bytes())
            .expect("test message failed to parse")
    }

    #[test]
    fn detect_plain_message_returns_none() {
        let msg = parse(
            "From: a@b.com\r\nSubject: Hi\r\nContent-Type: text/plain\r\n\r\nHello",
        );
        assert!(detect_pgp_mime(&msg).is_none());
    }

    #[test]
    fn detect_multipart_signed_extracts_signature_and_micalg() {
        let boundary = "abc123";
        let sig = "-----BEGIN PGP SIGNATURE-----\nhashdata\n-----END PGP SIGNATURE-----";
        let raw = format!(
            "From: a@b.com\r\n\
             Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; \
             micalg=pgp-sha256; boundary=\"{boundary}\"\r\n\
             \r\n\
             --{boundary}\r\n\
             Content-Type: text/plain\r\n\
             \r\n\
             Hello world\r\n\
             --{boundary}\r\n\
             Content-Type: application/pgp-signature\r\n\
             \r\n\
             {sig}\r\n\
             --{boundary}--\r\n",
        );
        let msg = parse(&raw);
        let status = detect_pgp_mime(&msg).expect("should detect signed");
        assert_eq!(status.kind, crate::imap::types::PgpStatusKind::Signed);
        assert_eq!(status.micalg.as_deref(), Some("pgp-sha256"));
        assert!(status.signature.as_deref().unwrap().contains("PGP SIGNATURE"));
        assert!(status.ciphertext.is_none());
        // signed_content is base64-encoded raw bytes of the first body part.
        let sc_b64 = status.signed_content.expect("signed_content should be present");
        use base64::Engine;
        let sc_bytes = base64::engine::general_purpose::STANDARD.decode(&sc_b64).unwrap();
        let sc = String::from_utf8(sc_bytes).unwrap();
        assert!(sc.contains("Content-Type: text/plain"));
        assert!(sc.contains("Hello world"));
    }

    #[test]
    fn detect_multipart_encrypted_extracts_ciphertext() {
        let boundary = "pgpbnd";
        let ct = "-----BEGIN PGP MESSAGE-----\nciphertext\n-----END PGP MESSAGE-----";
        let raw = format!(
            "From: a@b.com\r\n\
             Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; \
             boundary=\"{boundary}\"\r\n\
             \r\n\
             --{boundary}\r\n\
             Content-Type: application/pgp-encrypted\r\n\
             \r\n\
             Version: 1\r\n\
             --{boundary}\r\n\
             Content-Type: application/octet-stream\r\n\
             \r\n\
             {ct}\r\n\
             --{boundary}--\r\n",
        );
        let msg = parse(&raw);
        let status = detect_pgp_mime(&msg).expect("should detect encrypted");
        assert_eq!(status.kind, crate::imap::types::PgpStatusKind::Encrypted);
        assert!(status.ciphertext.as_deref().unwrap().contains("PGP MESSAGE"));
        assert!(status.signature.is_none());
        assert!(status.micalg.is_none());
    }

    #[test]
    fn detect_multipart_signed_wrong_protocol_returns_none() {
        let boundary = "bnd";
        let raw = format!(
            "From: a@b.com\r\n\
             Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"; \
             boundary=\"{boundary}\"\r\n\
             \r\n\
             --{boundary}\r\nContent-Type: text/plain\r\n\r\nHi\r\n\
             --{boundary}--\r\n",
        );
        let msg = parse(&raw);
        assert!(detect_pgp_mime(&msg).is_none());
    }
}
