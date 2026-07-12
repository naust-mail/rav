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
use crate::mail_transport::MailTransport;
use crate::smtp::client::{AttachmentData, SendableMessage, SmtpClient, SmtpCredentials};
use crate::smtp::types::{PgpMode, PgpSendParams};

/// IMAP/SMTP credentials for a single send, resolved once at enqueue time
/// (from the live session) and passed down to `perform_send`. The outbox
/// worker builds this same shape from the credentials it holds in memory
/// for the user's worker task.
pub(crate) struct SendCredentials {
    pub user_hash: String,
    pub email: String,
    pub password: String,
}

/// Fully-resolved send job, shared by the immediate-send handler and the
/// outbox worker's deferred send.
pub(crate) struct SendJob {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub text_body: String,
    pub html_body: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub draft_id: Option<String>,
    pub from_identity_id: Option<i64>,
    pub pgp: Option<PgpSendParams>,
}

/// Convert a client-supplied `PgpSendRequest` into the internal `PgpSendParams`,
/// validating the micalg against the allow-list. Shared by the immediate-send
/// handler and the outbox enqueue route, both of which take this shape from
/// the client and need the same validation before it's trusted.
pub(crate) fn resolve_pgp_params(pgp_req: &PgpSendRequest) -> Result<PgpSendParams, AppError> {
    let mode = match pgp_req.mode.as_str() {
        "sign" => PgpMode::Sign,
        "encrypt" => PgpMode::Encrypt,
        other => return Err(AppError::BadRequest(format!("Unknown PGP mode: {other}"))),
    };
    const ALLOWED_MICALG: &[&str] = &[
        "pgp-sha256", "pgp-sha384", "pgp-sha512", "pgp-sha224",
        "pgp-sha1", "pgp-ripemd160",
    ];
    let micalg = pgp_req.micalg.clone().unwrap_or_else(|| "pgp-sha256".to_string());
    if !ALLOWED_MICALG.contains(&micalg.as_str()) {
        return Err(AppError::BadRequest(format!("Invalid micalg: {micalg}")));
    }
    Ok(PgpSendParams {
        mode,
        signature: pgp_req.signature.clone(),
        ciphertext: pgp_req.ciphertext.clone(),
        micalg,
    })
}

/// Build the message, send it over SMTP, best-effort append a copy to the
/// Sent folder, and (if the job came from a draft) clean up the draft's
/// IMAP copy, staging row, and attachment files. Used both by the
/// immediate-send handler and the outbox worker's deferred send — the two
/// only differ in where the credentials and job data come from.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn perform_send(
    config: &AppConfig,
    transport: &MailTransport,
    smtp_client: &Arc<dyn SmtpClient>,
    imap_client: &Arc<dyn ImapClient>,
    db_pool_manager: &db::pool::DbPoolManager,
    creds: &SendCredentials,
    job: SendJob,
) -> Result<String, AppError> {
    let smtp_host = config
        .smtp_host
        .as_deref()
        .ok_or_else(|| AppError::ServiceUnavailable("SMTP server not configured".to_string()))?;

    let smtp_creds = SmtpCredentials {
        host: smtp_host.to_string(),
        connect_host: transport.smtp_connect_host.clone(),
        port: config.smtp_port,
        tls: config.tls_enabled,
        email: creds.email.clone(),
        password: creds.password.clone(),
        tls_params: transport.smtp_tls_params.clone(),
    };

    // Resolve the From address: use identity if specified, else session email.
    let from_address = if let Some(identity_id) = job.from_identity_id {
        let identity = db::pool::with_user_db(db_pool_manager, &creds.user_hash, move |conn| {
            db::identities::get_identity(conn, identity_id)
        })
        .await
        .map_err(AppError::InternalError)?
        .ok_or_else(|| AppError::NotFound("Identity not found".to_string()))?;
        if identity.display_name.is_empty() {
            identity.email
        } else {
            format!("\"{}\" <{}>", identity.display_name, identity.email)
        }
    } else {
        creds.email.clone()
    };

    // Load attachments from draft if draft_id is provided.
    let attachments = if let Some(ref draft_id) = job.draft_id {
        load_draft_attachments(db_pool_manager, &creds.user_hash, draft_id).await?
    } else {
        vec![]
    };

    let message = SendableMessage {
        from: from_address,
        to: job.to,
        cc: job.cc,
        bcc: job.bcc,
        subject: job.subject,
        text_body: job.text_body,
        html_body: job.html_body,
        in_reply_to: job.in_reply_to,
        references: job.references,
        attachments,
        auto_submitted: false,
        pgp: job.pgp,
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
            email: creds.email.clone(),
            password: creds.password.clone(),
        };

        match build_rfc822_bytes(&message, &message_id) {
            Err(e) => {
                tracing::warn!(error = %e, "Failed to build RFC822 bytes for Sent copy");
            }
            Ok(rfc822_bytes) => {
                // Look up the actual Sent folder name from the cached folder list.
                // Hardcoding "Sent" fails when the server uses a different name.
                let sent_folder = db::pool::with_user_db(db_pool_manager, &creds.user_hash, |conn| {
                    Ok(db::folders::find_folder_by_attribute(conn, "\\Sent").ok().flatten())
                })
                .await
                .ok()
                .flatten()
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
                        let _ = db::pool::with_user_db(db_pool_manager, &creds.user_hash, {
                            let sent_folder = sent_folder.clone();
                            move |conn| db::folders::invalidate_folder_freshness(conn, &sent_folder)
                        })
                        .await;
                    }
                }
            }
        }
    }

    // Clean up draft and attachment files after successful send.
    if let Some(ref draft_id) = job.draft_id {
        cleanup_draft(config, db_pool_manager, &creds.user_hash, &creds.email, &creds.password, draft_id, imap_client).await;
    }

    Ok(message_id)
}

/// Whether a failed `perform_send` is worth retrying. Setup-time failures
/// (bad config, missing identity, unreadable attachment) won't fix
/// themselves on a blind retry; SMTP auth failures mean the stored
/// password is wrong and retrying just repeats the same rejection.
/// Connection/transient send failures are the only ones worth another attempt.
pub(crate) fn is_retryable(err: &AppError) -> bool {
    match err {
        AppError::ServiceUnavailable(msg) => !msg.contains("Authentication failed"),
        AppError::InternalError(_) | AppError::NotFound(_) | AppError::BadRequest(_) | AppError::Unauthorized(_) => false,
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct PgpSendRequest {
    pub mode: String,
    pub signature: Option<String>,
    pub ciphertext: Option<String>,
    pub micalg: Option<String>,
}

/// Shared request body for both `POST /api/messages/send` (immediate) and
/// `POST /api/outbox` (queued). Both send flows take the exact same shape.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct SendResponse {
    pub status: String,
    pub message_id: String,
}

/// Handler for `POST /api/messages/send`.
///
/// Sends immediately, bypassing the outbox/undo-delay queue. Kept as a
/// direct API primitive; the webmail frontend itself goes through
/// `POST /api/outbox` so sends are undoable and retryable.
pub async fn send_message_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(transport): Extension<Arc<MailTransport>>,
    Extension(smtp_client): Extension<Arc<dyn SmtpClient>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(req): Json<SendRequest>,
) -> Result<Response, AppError> {
    validate_send_request(&req)?;

    let pgp = req.pgp.as_ref().map(resolve_pgp_params).transpose()?;

    let creds = SendCredentials {
        user_hash: session.user_hash.clone(),
        email: session.email.clone(),
        password: session.password.clone(),
    };
    let job = SendJob {
        to: req.to,
        cc: req.cc,
        bcc: req.bcc,
        subject: req.subject,
        text_body: req.text_body.unwrap_or_default(),
        html_body: req.html_body,
        in_reply_to: req.in_reply_to,
        references: req.references,
        draft_id: req.draft_id,
        from_identity_id: req.from_identity_id,
        pgp,
    };

    let message_id = perform_send(&config, &transport, &smtp_client, &imap_client, &db_pool_manager, &creds, job).await?;

    Ok(Json(SendResponse {
        status: "sent".to_string(),
        message_id,
    })
    .into_response())
}

/// Validate recipients and subject/body presence. Shared by the immediate-send
/// handler and the outbox enqueue route, since both take the same request shape.
pub(crate) fn validate_send_request(req: &SendRequest) -> Result<(), AppError> {
    if req.to.is_empty() && req.cc.is_empty() && req.bcc.is_empty() {
        return Err(AppError::BadRequest(
            "At least one recipient is required".to_string(),
        ));
    }

    let has_subject = !req.subject.trim().is_empty();
    let has_text = req.text_body.as_deref().is_some_and(|t| !t.trim().is_empty());
    let has_html = req.html_body.as_deref().is_some_and(|h| !h.trim().is_empty());
    if !has_subject && !has_text && !has_html {
        return Err(AppError::BadRequest(
            "Subject or body is required".to_string(),
        ));
    }

    Ok(())
}

/// Load attachment data from disk for a given draft.
async fn load_draft_attachments(
    db_pool_manager: &db::pool::DbPoolManager,
    user_hash: &str,
    draft_id: &str,
) -> Result<Vec<AttachmentData>, AppError> {
    let db_attachments = db::pool::with_user_db(db_pool_manager, user_hash, {
        let draft_id = draft_id.to_string();
        move |conn| db::drafts::get_draft_attachments(conn, &draft_id)
    })
    .await
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

struct DraftCleanupPreState {
    imap_uid: Option<u32>,
    drafts_folder: String,
}

/// Clean up draft record and attachment files from disk after successful send.
#[allow(clippy::too_many_arguments)]
async fn cleanup_draft(
    config: &AppConfig,
    db_pool_manager: &db::pool::DbPoolManager,
    user_hash: &str,
    email: &str,
    password: &str,
    draft_id: &str,
    imap_client: &Arc<dyn ImapClient>,
) {
    let pre = db::pool::with_user_db(db_pool_manager, user_hash, {
        let draft_id = draft_id.to_string();
        move |conn| {
            let imap_uid = db::drafts::get_staging(conn, &draft_id)
                .ok()
                .flatten()
                .and_then(|s| s.imap_uid);
            let drafts_folder = db::folders::find_folder_by_attribute(conn, "\\Drafts")
                .ok()
                .flatten()
                .unwrap_or_else(|| "Drafts".to_string());
            Ok(DraftCleanupPreState { imap_uid, drafts_folder })
        }
    })
    .await;

    let Ok(DraftCleanupPreState { imap_uid, drafts_folder }) = pre else {
        tracing::warn!("Failed to open DB during draft cleanup");
        return;
    };

    // Expunge the IMAP Drafts copy before deleting the staging record.
    if let Some(imap_host) = config.imap_host.as_deref()
        && let Some(uid) = imap_uid
        {
            let imap_creds = ImapCredentials {
                host: imap_host.to_string(),
                port: config.imap_port,
                tls: config.tls_enabled,
                email: email.to_string(),
                password: password.to_string(),
            };
            if let Err(e) = imap_client.expunge_message(&imap_creds, &drafts_folder, uid).await {
                tracing::warn!(error = %e, uid = uid, "Failed to expunge draft from IMAP after send");
            }
        }

    let staging_result = db::pool::with_user_db(db_pool_manager, user_hash, {
        let draft_id = draft_id.to_string();
        move |conn| db::drafts::delete_staging(conn, &draft_id)
    })
    .await;
    if let Err(e) = staging_result {
        tracing::warn!(error = %e, "Failed to delete draft staging after send");
    }

    let att_dir = Path::new(&config.data_dir)
        .join(user_hash)
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
