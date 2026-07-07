//! SMTP client abstraction and real implementation.

use async_trait::async_trait;
use tracing::warn;

use crate::error::ConnectError;

// Import and re-export types for backward compatibility
pub use crate::smtp::error::SmtpError;
pub use crate::smtp::types::{AttachmentData, PgpMode, PgpSendParams, SendableMessage, SmtpCredentials};

// ---------------------------------------------------------------------------
// Trait definition
// ---------------------------------------------------------------------------

/// Abstraction over SMTP operations.
///
/// Every method receives explicit connection parameters so that the trait
/// remains stateless — no persistent connections are held.
///
/// The `Send + Sync` bounds allow implementations to be shared across
/// Tokio tasks and stored in `Arc`.
#[async_trait]
pub trait SmtpClient: Send + Sync {
    /// Send an email message. Returns the generated Message-ID on success.
    async fn send_message(
        &self,
        creds: &SmtpCredentials,
        message: &SendableMessage,
    ) -> Result<String, SmtpError>;
}

// ---------------------------------------------------------------------------
// PGP/MIME wrapping helpers
// ---------------------------------------------------------------------------

/// Normalize line endings to CRLF.
fn to_crlf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n")
}

/// Extract envelope headers from formatted RFC 822 bytes, stripping MIME-specific
/// headers (Content-Type, MIME-Version) so they can be replaced by PGP variants.
fn filter_envelope_headers(raw: &str) -> String {
    let (headers_str, _) = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .map_or((raw, ""), |p| p);

    let mut result: Vec<&str> = Vec::new();
    let mut skip_continuation = false;

    for line in headers_str.split("\r\n") {
        if line.is_empty() {
            continue;
        }
        // Folded continuation lines start with whitespace.
        if line.starts_with(' ') || line.starts_with('\t') {
            if !skip_continuation {
                result.push(line);
            }
            continue;
        }
        let lower = line.to_ascii_lowercase();
        skip_continuation = lower.starts_with("content-type:")
            || lower.starts_with("mime-version:");
        if !skip_continuation {
            result.push(line);
        }
    }

    result.join("\r\n")
}

/// Wrap the inner email bytes in a PGP/MIME envelope per RFC 3156.
pub(crate) fn wrap_pgp_mime(inner_bytes: &[u8], pgp: &PgpSendParams) -> Result<Vec<u8>, String> {
    let raw = std::str::from_utf8(inner_bytes)
        .map_err(|e| format!("Message bytes are not valid UTF-8: {e}"))?;

    let envelope_headers = filter_envelope_headers(raw);
    let boundary = format!("----pgpboundary{}", uuid::Uuid::new_v4().simple());

    let wrapped = match pgp.mode {
        PgpMode::Sign => {
            let sig = pgp.signature.as_deref()
                .ok_or_else(|| "PGP sign mode requires a signature".to_string())?;

            // Part 1 is the canonical plain text body that the client signed.
            // The client signed toCanonical(content), so we reproduce that here.
            let body_text = extract_text_body(raw);
            let canonical = to_crlf(&body_text);

            format!(
                "{envelope_headers}\r\n\
                 MIME-Version: 1.0\r\n\
                 Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; \
                 micalg={micalg}; boundary=\"{boundary}\"\r\n\
                 \r\n\
                 --{boundary}\r\n\
                 Content-Type: text/plain; charset=utf-8\r\n\
                 \r\n\
                 {canonical}\r\n\
                 --{boundary}\r\n\
                 Content-Type: application/pgp-signature\r\n\
                 Content-Description: OpenPGP digital signature\r\n\
                 \r\n\
                 {sig}\r\n\
                 --{boundary}--\r\n",
                micalg = pgp.micalg,
            )
        }
        PgpMode::Encrypt => {
            let ct = pgp.ciphertext.as_deref()
                .ok_or_else(|| "PGP encrypt mode requires ciphertext".to_string())?;

            format!(
                "{envelope_headers}\r\n\
                 MIME-Version: 1.0\r\n\
                 Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; \
                 boundary=\"{boundary}\"\r\n\
                 \r\n\
                 --{boundary}\r\n\
                 Content-Type: application/pgp-encrypted\r\n\
                 Content-Description: PGP/MIME version identification\r\n\
                 \r\n\
                 Version: 1\r\n\
                 \r\n\
                 --{boundary}\r\n\
                 Content-Type: application/octet-stream; name=\"encrypted.asc\"\r\n\
                 Content-Description: OpenPGP encrypted message\r\n\
                 Content-Disposition: inline; filename=\"encrypted.asc\"\r\n\
                 \r\n\
                 {ct}\r\n\
                 --{boundary}--\r\n",
            )
        }
    };

    Ok(wrapped.into_bytes())
}

/// Extract the plain-text body from raw RFC 822 bytes for use as Part 1 of
/// a multipart/signed message. Falls back to empty string if not found.
fn extract_text_body(raw: &str) -> String {
    // Split on the first blank line to get the body.
    let body = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .map(|(_, b)| b)
        .unwrap_or("");

    // For multipart bodies, find the first text/plain part.
    // For simple single-part, the body IS the text.
    if body.contains("Content-Type: text/plain") {
        // Walk parts: find the text/plain part body.
        if let Some(start) = body.find("Content-Type: text/plain") {
            let after_header = &body[start..];
            if let Some(part_body_start) = after_header.find("\r\n\r\n").or_else(|| after_header.find("\n\n")) {
                let part_body = &after_header[part_body_start..].trim_start_matches("\r\n").trim_start_matches('\n');
                // Cut at the next boundary if present.
                let end = part_body.find("\r\n--").or_else(|| part_body.find("\n--")).unwrap_or(part_body.len());
                return part_body[..end].to_string();
            }
        }
    }

    body.to_string()
}

// ---------------------------------------------------------------------------
// Real implementation backed by lettre
// ---------------------------------------------------------------------------

/// Production SMTP client that uses `lettre`.
///
/// This is a stateless unit struct — every method creates a fresh connection,
/// performs the operation, and disconnects.
pub struct RealSmtpClient;

#[async_trait]
impl SmtpClient for RealSmtpClient {
    async fn send_message(
        &self,
        creds: &SmtpCredentials,
        message: &SendableMessage,
    ) -> Result<String, SmtpError> {
        use lettre::message::{
            header::ContentType, Attachment, Mailbox, MessageBuilder, MultiPart, SinglePart,
        };
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

        // Generate a unique Message-ID.
        let message_id = format!(
            "<{}.{}@{}>",
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            creds.host
        );

        // Build the email message.
        let from_mailbox: Mailbox = message
            .from
            .parse()
            .map_err(|e: lettre::address::AddressError| SmtpError::SendFailed(e.to_string()))?;

        let mut builder: MessageBuilder = lettre::Message::builder()
            .from(from_mailbox)
            .subject(&message.subject)
            .message_id(Some(message_id.clone()));

        // Add To recipients.
        for addr in &message.to {
            let mailbox: Mailbox = addr
                .parse()
                .map_err(|e: lettre::address::AddressError| SmtpError::SendFailed(e.to_string()))?;
            builder = builder.to(mailbox);
        }

        // Add CC recipients.
        for addr in &message.cc {
            let mailbox: Mailbox = addr
                .parse()
                .map_err(|e: lettre::address::AddressError| SmtpError::SendFailed(e.to_string()))?;
            builder = builder.cc(mailbox);
        }

        // Add BCC recipients.
        for addr in &message.bcc {
            let mailbox: Mailbox = addr
                .parse()
                .map_err(|e: lettre::address::AddressError| SmtpError::SendFailed(e.to_string()))?;
            builder = builder.bcc(mailbox);
        }

        // Add In-Reply-To header.
        if let Some(ref irt) = message.in_reply_to {
            builder = builder.in_reply_to(irt.clone());
        }

        // Add References header.
        if let Some(ref refs) = message.references {
            builder = builder.references(refs.clone());
        }

        // RFC 3834: automated replies must identify themselves to prevent loops.
        if message.auto_submitted {
            use lettre::message::header::{HeaderName, HeaderValue};
            let name = HeaderName::new_from_ascii_str("Auto-Submitted");
            builder = builder.raw_header(HeaderValue::new(name, "auto-replied".to_string()));
        }

        // Separate inline images (those with content_id referenced in HTML)
        // from regular file attachments.
        let html_body = message.html_body.as_deref().unwrap_or("");
        let (inline_atts, file_atts): (Vec<_>, Vec<_>) =
            message.attachments.iter().partition(|att| {
                att.content_id
                    .as_ref()
                    .is_some_and(|cid| html_body.contains(&format!("cid:{cid}")))
            });

        // Build the body part(s).
        let body_part = if let Some(ref html) = message.html_body {
            if inline_atts.is_empty() {
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .content_type(ContentType::TEXT_PLAIN)
                            .body(message.text_body.clone()),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .content_type(ContentType::TEXT_HTML)
                            .body(html.clone()),
                    )
            } else {
                let mut related = MultiPart::related().singlepart(
                    SinglePart::builder()
                        .content_type(ContentType::TEXT_HTML)
                        .body(html.clone()),
                );
                for att in &inline_atts {
                    let ct: ContentType = att
                        .content_type
                        .parse()
                        .unwrap_or(ContentType::TEXT_PLAIN);
                    let cid = att.content_id.as_deref().unwrap_or("unknown");
                    let inline_part =
                        Attachment::new_inline(cid.to_string()).body(att.data.clone(), ct);
                    related = related.singlepart(inline_part);
                }

                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .content_type(ContentType::TEXT_PLAIN)
                            .body(message.text_body.clone()),
                    )
                    .multipart(related)
            }
        } else {
            MultiPart::alternative().singlepart(
                SinglePart::builder()
                    .content_type(ContentType::TEXT_PLAIN)
                    .body(message.text_body.clone()),
            )
        };

        // Build the final email — wrap in mixed multipart if there are file attachments.
        let email = if file_atts.is_empty() {
            builder
                .multipart(body_part)
                .map_err(|e| SmtpError::SendFailed(e.to_string()))?
        } else {
            let mut mixed = MultiPart::mixed().multipart(body_part);
            for att in &file_atts {
                let ct: ContentType = att
                    .content_type
                    .parse()
                    .unwrap_or(ContentType::TEXT_PLAIN);
                let attachment =
                    Attachment::new(att.filename.clone()).body(att.data.clone(), ct);
                mixed = mixed.singlepart(attachment);
            }
            builder
                .multipart(mixed)
                .map_err(|e| SmtpError::SendFailed(e.to_string()))?
        };

        // Build the SMTP transport.
        let smtp_creds =
            Credentials::new(creds.email.clone(), creds.password.clone());

        let transport: AsyncSmtpTransport<Tokio1Executor> = if creds.tls {
            use lettre::transport::smtp::client::Tls;

            // When custom TLS params are available (including any extra trusted cert),
            // use builder_dangerous to connect to connect_host for TCP while the
            // params carry the correct SNI hostname and cert trust settings.
            // Without custom params, fall back to lettre's relay helpers which use
            // the host for both TCP and SNI with system CA roots.
            if let Some(params) = creds.tls_params.clone() {
                let tls_mode = if creds.port == 587 {
                    Tls::Required(params)   // STARTTLS
                } else {
                    Tls::Wrapper(params)    // implicit TLS (port 465)
                };
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&creds.connect_host)
                    .port(creds.port)
                    .credentials(smtp_creds)
                    .tls(tls_mode)
                    .build()
            } else if creds.port == 587 {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&creds.host)
                    .map_err(|e| {
                        warn!(error = %e, host = creds.host, "SMTP STARTTLS relay setup failed");
                        SmtpError::ConnectionFailed(ConnectError::TlsHandshake)
                    })?
                    .port(creds.port)
                    .credentials(smtp_creds)
                    .build()
            } else {
                AsyncSmtpTransport::<Tokio1Executor>::relay(&creds.host)
                    .map_err(|e| {
                        warn!(error = %e, host = creds.host, "SMTP TLS relay setup failed");
                        SmtpError::ConnectionFailed(ConnectError::TlsHandshake)
                    })?
                    .port(creds.port)
                    .credentials(smtp_creds)
                    .build()
            }
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&creds.connect_host)
                .port(creds.port)
                .credentials(smtp_creds)
                .build()
        };

        // Send the message, wrapping in PGP/MIME if requested.
        let send_result = if let Some(ref pgp) = message.pgp {
            let inner_bytes = email.formatted();
            let envelope = email.envelope().clone();
            let pgp_bytes = wrap_pgp_mime(&inner_bytes, pgp)
                .map_err(SmtpError::SendFailed)?;
            transport.send_raw(&envelope, &pgp_bytes).await
        } else {
            transport.send(email).await
        };

        send_result.map_err(|e| {
            if e.is_tls() {
                warn!(error = %e, host = creds.host, "SMTP TLS error");
                SmtpError::ConnectionFailed(ConnectError::TlsHandshake)
            } else if e.is_timeout() {
                warn!(host = creds.host, "SMTP connection timed out");
                SmtpError::ConnectionFailed(ConnectError::Timeout)
            } else if e.is_transport_shutdown() {
                warn!(error = %e, host = creds.host, "SMTP transport shutdown unexpectedly");
                SmtpError::ConnectionFailed(ConnectError::Unreachable)
            } else {
                let msg = e.to_string();
                if msg.to_lowercase().contains("authentication")
                    || msg.to_lowercase().contains("credentials")
                    || msg.to_lowercase().contains("auth")
                {
                    warn!(error = %e, host = creds.host, "SMTP authentication failed");
                    SmtpError::AuthenticationFailed
                } else if msg.to_lowercase().contains("connect")
                    || msg.to_lowercase().contains("dns")
                    || msg.to_lowercase().contains("resolve")
                {
                    warn!(error = %e, host = creds.host, "SMTP connection failed");
                    SmtpError::ConnectionFailed(ConnectError::Unreachable)
                } else {
                    warn!(error = %e, host = creds.host, "SMTP send failed");
                    SmtpError::SendFailed(msg)
                }
            }
        })?;

        Ok(message_id)
    }
}

// ---------------------------------------------------------------------------
// Mock implementation (test-only)
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "mock.rs"]
pub mod mock;

#[cfg(test)]
mod wrap_tests {
    use super::*;
    use crate::smtp::types::{PgpMode, PgpSendParams};

    fn inner_message(body: &str) -> Vec<u8> {
        format!(
            "From: alice@example.com\r\n\
             To: bob@example.com\r\n\
             Subject: Test\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             MIME-Version: 1.0\r\n\
             \r\n\
             {body}"
        )
        .into_bytes()
    }

    #[test]
    fn sign_produces_multipart_signed_structure() {
        let params = PgpSendParams {
            mode: PgpMode::Sign,
            signature: Some("-----BEGIN PGP SIGNATURE-----\nSIGDATA\n-----END PGP SIGNATURE-----".into()),
            ciphertext: None,
            micalg: "pgp-sha256".into(),
        };
        let output = wrap_pgp_mime(&inner_message("Hello"), &params).unwrap();
        let text = std::str::from_utf8(&output).unwrap();

        assert!(text.contains("multipart/signed"), "must be multipart/signed");
        assert!(text.contains("protocol=\"application/pgp-signature\""), "must carry pgp-signature protocol");
        assert!(text.contains("micalg=pgp-sha256"), "must carry micalg");
        assert!(text.contains("Content-Type: application/pgp-signature"), "must have signature part");
        assert!(text.contains("SIGDATA"), "must include signature body");
        // Envelope headers kept, MIME headers replaced.
        assert!(text.contains("From: alice@example.com"));
        assert!(!text.contains("MIME-Version: 1.0\r\nContent-Type: text/plain"), "inner MIME-Version must be stripped");
    }

    #[test]
    fn encrypt_produces_multipart_encrypted_structure() {
        let params = PgpSendParams {
            mode: PgpMode::Encrypt,
            signature: None,
            ciphertext: Some("-----BEGIN PGP MESSAGE-----\nCIPHER\n-----END PGP MESSAGE-----".into()),
            micalg: "pgp-sha256".into(),
        };
        let output = wrap_pgp_mime(&inner_message("Secret"), &params).unwrap();
        let text = std::str::from_utf8(&output).unwrap();

        assert!(text.contains("multipart/encrypted"), "must be multipart/encrypted");
        assert!(text.contains("protocol=\"application/pgp-encrypted\""), "must carry pgp-encrypted protocol");
        assert!(text.contains("Content-Type: application/pgp-encrypted\r\n"), "must have version part");
        assert!(text.contains("Version: 1"), "must include RFC 3156 version header");
        assert!(text.contains("Content-Type: application/octet-stream"), "must have ciphertext part");
        assert!(text.contains("CIPHER"), "must include ciphertext body");
    }

    #[test]
    fn sign_without_signature_returns_err() {
        let params = PgpSendParams {
            mode: PgpMode::Sign,
            signature: None,
            ciphertext: None,
            micalg: "pgp-sha256".into(),
        };
        assert!(wrap_pgp_mime(&inner_message("Hi"), &params).is_err());
    }

    #[test]
    fn encrypt_without_ciphertext_returns_err() {
        let params = PgpSendParams {
            mode: PgpMode::Encrypt,
            signature: None,
            ciphertext: None,
            micalg: "pgp-sha256".into(),
        };
        assert!(wrap_pgp_mime(&inner_message("Hi"), &params).is_err());
    }

    #[test]
    fn non_utf8_inner_returns_err() {
        let params = PgpSendParams {
            mode: PgpMode::Sign,
            signature: Some("SIG".into()),
            ciphertext: None,
            micalg: "pgp-sha256".into(),
        };
        // 0xFF is not valid UTF-8.
        assert!(wrap_pgp_mime(&[0xFF, 0xFE], &params).is_err());
    }
}
