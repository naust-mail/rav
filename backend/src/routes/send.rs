use std::path::Path;
use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::db;
use crate::error::AppError;
use crate::imap::client::{ImapClient, ImapCredentials};
use crate::smtp::client::{AttachmentData, SendableMessage, SmtpClient, SmtpCredentials};
use crate::smtp::types::{PgpMode, PgpSendParams};

#[derive(Debug, Deserialize)]
pub struct PgpSendRequest {
    pub mode: String,
    pub signature: Option<String>,
    pub ciphertext: Option<String>,
    pub micalg: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SendRequest {
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    #[serde(default)]
    pub subject: String,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    /// If sending from a draft, include the draft ID to load attachments
    /// and clean up after send.
    pub draft_id: Option<String>,
    /// Optional identity ID to use as the From address.
    /// If not provided, falls back to the session email.
    pub from_identity_id: Option<i64>,
    /// PGP/MIME parameters computed by the client-side worker.
    pub pgp: Option<PgpSendRequest>,
}

#[derive(Debug, Serialize)]
pub struct SendResponse {
    pub status: String,
    pub message_id: String,
}

/// Handler for `POST /api/messages/send`.
///
/// Validates the request, sends the message via SMTP, and appends a copy
/// to the IMAP "Sent" folder (best-effort).
pub async fn send_message_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(transport): Extension<Arc<crate::mail_transport::MailTransport>>,
    Extension(smtp_client): Extension<Arc<dyn SmtpClient>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Json(req): Json<SendRequest>,
) -> Result<Response, AppError> {
    // Validate: at least one recipient.
    if req.to.is_empty() && req.cc.is_empty() && req.bcc.is_empty() {
        return Err(AppError::BadRequest(
            "At least one recipient is required".to_string(),
        ));
    }

    // Validate: subject or body must be non-empty.
    let has_subject = !req.subject.trim().is_empty();
    let has_text = req.text_body.as_deref().is_some_and(|t| !t.trim().is_empty());
    let has_html = req.html_body.as_deref().is_some_and(|h| !h.trim().is_empty());
    if !has_subject && !has_text && !has_html {
        return Err(AppError::BadRequest(
            "Subject or body is required".to_string(),
        ));
    }

    // Check that SMTP is configured.
    let smtp_host = config
        .smtp_host
        .as_deref()
        .ok_or_else(|| AppError::ServiceUnavailable("SMTP server not configured".to_string()))?;

    // Build SMTP credentials from config + session + transport.
    let smtp_creds = SmtpCredentials {
        host: smtp_host.to_string(),
        connect_host: transport.smtp_connect_host.clone(),
        port: config.smtp_port,
        tls: config.tls_enabled,
        email: session.email.clone(),
        password: session.password.clone(),
        tls_params: transport.smtp_tls_params.clone(),
    };

    // Resolve the From address: use identity if specified, else session email.
    let from_address = if let Some(identity_id) = req.from_identity_id {
        let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
            .map_err(|e| AppError::InternalError(format!("Failed to open database: {e}")))?;
        let identity = db::identities::get_identity(&conn, identity_id)
            .map_err(AppError::InternalError)?
            .ok_or_else(|| AppError::NotFound("Identity not found".to_string()))?;
        if identity.display_name.is_empty() {
            identity.email
        } else {
            format!("\"{}\" <{}>", identity.display_name, identity.email)
        }
    } else {
        session.email.clone()
    };

    // Load attachments from draft if draft_id is provided.
    let attachments = if let Some(ref draft_id) = req.draft_id {
        load_draft_attachments(&config.data_dir, &session.user_hash, draft_id)?
    } else {
        vec![]
    };

    // Convert PGP request params to internal type.
    let pgp_params: Option<PgpSendParams> = if let Some(ref pgp_req) = req.pgp {
        let mode = match pgp_req.mode.as_str() {
            "sign" => PgpMode::Sign,
            "encrypt" => PgpMode::Encrypt,
            other => {
                return Err(AppError::BadRequest(format!(
                    "Unknown PGP mode: {other}"
                )))
            }
        };
        const ALLOWED_MICALG: &[&str] = &[
            "pgp-sha256", "pgp-sha384", "pgp-sha512", "pgp-sha224",
            "pgp-sha1", "pgp-ripemd160",
        ];
        let micalg = pgp_req.micalg.clone().unwrap_or_else(|| "pgp-sha256".to_string());
        if !ALLOWED_MICALG.contains(&micalg.as_str()) {
            return Err(AppError::BadRequest(format!("Invalid micalg: {micalg}")));
        }
        Some(PgpSendParams {
            mode,
            signature: pgp_req.signature.clone(),
            ciphertext: pgp_req.ciphertext.clone(),
            micalg,
        })
    } else {
        None
    };

    // Build the sendable message.
    let message = SendableMessage {
        from: from_address,
        to: req.to,
        cc: req.cc,
        bcc: req.bcc,
        subject: req.subject,
        text_body: req.text_body.unwrap_or_default(),
        html_body: req.html_body,
        in_reply_to: req.in_reply_to,
        references: req.references,
        attachments,
        auto_submitted: false,
        pgp: pgp_params,
    };

    // Send via SMTP.
    let message_id = smtp_client
        .send_message(&smtp_creds, &message)
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("Failed to send email: {e}")))?;

    // Best-effort: append a copy to the Sent folder via IMAP.
    // Don't fail the send if this fails.
    if let Some(imap_host) = config.imap_host.as_deref() {
        let imap_creds = ImapCredentials {
            host: imap_host.to_string(),
            port: config.imap_port,
            tls: config.tls_enabled,
            email: session.email.clone(),
            password: session.password.clone(),
        };

        match build_rfc822_bytes(&message, &message_id) {
            Err(e) => {
                tracing::warn!(error = %e, "Failed to build RFC822 bytes for Sent copy");
            }
            Ok(rfc822_bytes) => {
                // Look up the actual Sent folder name from the cached folder list.
                // Hardcoding "Sent" fails when the server uses a different name.
                let sent_folder = db::pool::open_user_db(&config.data_dir, &session.user_hash)
                    .ok()
                    .and_then(|conn| {
                        db::folders::find_folder_by_attribute(&conn, "\\Sent").ok().flatten()
                    })
                    .unwrap_or_else(|| "Sent".to_string());

                match imap_client
                    .append_message(&imap_creds, &sent_folder, &rfc822_bytes, &["\\Seen"], None)
                    .await
                {
                    Err(e) => {
                        tracing::warn!(error = %e, folder = %sent_folder, "Failed to append sent message to IMAP Sent folder");
                    }
                    Ok(_) => {
                        // Invalidate the Sent folder cache so the next list_messages
                        // is forced to re-check IMAP rather than returning stale 0.
                        if let Ok(conn) = db::pool::open_user_db(&config.data_dir, &session.user_hash) {
                            let _ = db::folders::invalidate_folder_freshness(&conn, &sent_folder);
                        }
                    }
                }
            }
        }
    }

    // Clean up draft and attachment files after successful send.
    if let Some(ref draft_id) = req.draft_id {
        cleanup_draft(&config, &session, draft_id, &imap_client).await;
    }

    Ok(Json(SendResponse {
        status: "sent".to_string(),
        message_id,
    })
    .into_response())
}

/// Load attachment data from disk for a given draft.
fn load_draft_attachments(
    data_dir: &str,
    user_hash: &str,
    draft_id: &str,
) -> Result<Vec<AttachmentData>, AppError> {
    let conn = db::pool::open_user_db(data_dir, user_hash)
        .map_err(|e| AppError::InternalError(format!("Failed to open database: {e}")))?;

    let db_attachments = db::drafts::get_draft_attachments(&conn, draft_id)
        .map_err(AppError::InternalError)?;

    let mut attachments = Vec::new();
    for att in db_attachments {
        let data = std::fs::read(&att.file_path).map_err(|e| {
            AppError::InternalError(format!(
                "Failed to read attachment file '{}': {e}",
                att.filename
            ))
        })?;
        attachments.push(AttachmentData {
            filename: att.filename,
            content_type: att.content_type,
            data,
            content_id: Some(att.id),
        });
    }
    Ok(attachments)
}

/// Clean up draft record and attachment files from disk after successful send.
async fn cleanup_draft(
    config: &AppConfig,
    session: &SessionState,
    draft_id: &str,
    imap_client: &Arc<dyn ImapClient>,
) {
    let conn = match db::pool::open_user_db(&config.data_dir, &session.user_hash) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to open DB during draft cleanup");
            return;
        }
    };

    // Expunge the IMAP Drafts copy before deleting the staging record.
    if let Some(imap_host) = config.imap_host.as_deref()
        && let Some(uid) = db::drafts::get_staging(&conn, draft_id)
            .ok()
            .flatten()
            .and_then(|s| s.imap_uid)
        {
            let imap_creds = ImapCredentials {
                host: imap_host.to_string(),
                port: config.imap_port,
                tls: config.tls_enabled,
                email: session.email.clone(),
                password: session.password.clone(),
            };
            let drafts_folder = db::folders::find_folder_by_attribute(&conn, "\\Drafts")
                .ok()
                .flatten()
                .unwrap_or_else(|| "Drafts".to_string());
            if let Err(e) = imap_client.expunge_message(&imap_creds, &drafts_folder, uid).await {
                tracing::warn!(error = %e, uid = uid, "Failed to expunge draft from IMAP after send");
            }
        }

    if let Err(e) = db::drafts::delete_staging(&conn, draft_id) {
        tracing::warn!(error = %e, "Failed to delete draft staging after send");
    }

    let att_dir = Path::new(&config.data_dir)
        .join(&session.user_hash)
        .join("attachments")
        .join(draft_id);
    if att_dir.exists()
        && let Err(e) = tokio::fs::remove_dir_all(&att_dir).await
    {
        tracing::warn!(error = %e, path = %att_dir.display(), "Failed to clean up attachment directory");
    }
}

/// Build RFC822 bytes from a SendableMessage for IMAP APPEND.
/// If the message has PGP params set, produces a PGP/MIME-wrapped RFC 822 message.
fn build_rfc822_bytes(message: &SendableMessage, message_id: &str) -> Result<Vec<u8>, String> {
    use lettre::message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart};

    let from_mailbox: Mailbox = message
        .from
        .parse()
        .map_err(|e: lettre::address::AddressError| e.to_string())?;

    let mut builder = lettre::Message::builder()
        .from(from_mailbox)
        .subject(&message.subject)
        .message_id(Some(message_id.to_string()));

    for addr in &message.to {
        let mailbox: Mailbox = addr.parse().map_err(|e: lettre::address::AddressError| e.to_string())?;
        builder = builder.to(mailbox);
    }
    for addr in &message.cc {
        let mailbox: Mailbox = addr.parse().map_err(|e: lettre::address::AddressError| e.to_string())?;
        builder = builder.cc(mailbox);
    }
    if let Some(ref irt) = message.in_reply_to {
        builder = builder.in_reply_to(irt.clone());
    }
    if let Some(ref refs) = message.references {
        builder = builder.references(refs.clone());
    }

    let html_body = message.html_body.as_deref().unwrap_or("");
    let (inline_atts, file_atts): (Vec<_>, Vec<_>) =
        message.attachments.iter().partition(|att| {
            att.content_id
                .as_ref()
                .is_some_and(|cid| html_body.contains(&format!("cid:{cid}")))
        });

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
                let ct: ContentType = att.content_type.parse().unwrap_or(ContentType::TEXT_PLAIN);
                let cid = att.content_id.as_deref().unwrap_or("unknown");
                let inline_part = Attachment::new_inline(cid.to_string()).body(att.data.clone(), ct);
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

    let email = if file_atts.is_empty() {
        builder.multipart(body_part).map_err(|e| e.to_string())?
    } else {
        let mut mixed = MultiPart::mixed().multipart(body_part);
        for att in &file_atts {
            let ct: ContentType = att.content_type.parse().unwrap_or(ContentType::TEXT_PLAIN);
            let attachment = Attachment::new(att.filename.clone()).body(att.data.clone(), ct);
            mixed = mixed.singlepart(attachment);
        }
        builder.multipart(mixed).map_err(|e| e.to_string())?
    };

    // If PGP params are set, wrap the formatted message in PGP/MIME.
    if let Some(ref pgp) = message.pgp {
        use crate::smtp::client::wrap_pgp_mime;
        let inner = email.formatted();
        return wrap_pgp_mime(&inner, pgp);
    }

    Ok(email.formatted())
}
