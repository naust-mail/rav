use std::path::Path;
use std::sync::Arc;

use axum::extract::Path as AxumPath;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::db;
use crate::error::AppError;
use crate::imap::client::{ImapClient, ImapCredentials};
use crate::realtime::events::{EventBus, MailEvent};

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SaveDraftRequest {
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub bcc: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub text_body: String,
    pub html_body: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
}

#[derive(Debug, Serialize)]
struct SaveDraftResponse {
    status: String,
}

#[derive(Debug, Serialize)]
struct DeleteResponse {
    status: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Request body for `POST /api/drafts/reply-for`.
#[derive(Debug, Deserialize)]
pub struct ReplyForRequest {
    pub message_id: String,
}

/// `POST /api/drafts/reply-for` — Return the staging row for a reply draft
/// that targets the given Message-ID, if one exists.
pub async fn get_reply_draft_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Json(body): Json<ReplyForRequest>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    match db::drafts::find_by_reply_message_id(&conn, &body.message_id)
        .map_err(AppError::InternalError)?
    {
        None => Ok(axum::http::StatusCode::NOT_FOUND.into_response()),
        Some(staging) => {
            let draft_folder_name = db::folders::find_folder_by_attribute(&conn, "\\Drafts")
                .ok()
                .flatten()
                .unwrap_or_else(|| "Drafts".to_string());
            let cipher = crate::folder_cipher::FolderCipher::new(&session.folder_key);
            Ok(Json(serde_json::json!({
                "uuid": staging.uuid,
                "imap_uid": staging.imap_uid,
                "draft_folder": cipher.encrypt(&draft_folder_name),
            }))
            .into_response())
        }
    }
}

/// `POST /api/drafts/{uuid}` — Save draft body to IMAP, replacing the previous copy.
///
/// The UUID is client-generated and stable across saves. It is embedded as the
/// `Message-ID` in the RFC822 so the draft can be identified when reopened from IMAP.
pub async fn save_draft_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(event_bus): Extension<Arc<EventBus>>,
    AxumPath(uuid): AxumPath<String>,
    Json(req): Json<SaveDraftRequest>,
) -> Result<Response, AppError> {
    let Some(imap_host) = config.imap_host.as_deref() else {
        return Err(AppError::InternalError("IMAP not configured".to_string()));
    };

    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    let existing_uid = db::drafts::get_staging(&conn, &uuid)
        .ok()
        .flatten()
        .and_then(|s| s.imap_uid);

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

    // Expunge the previous IMAP copy before writing the new one.
    if let Some(old_uid) = existing_uid {
        if let Err(e) = imap_client.expunge_message(&imap_creds, &drafts_folder, old_uid).await {
            tracing::warn!(error = %e, uid = old_uid, "Failed to expunge old draft from IMAP");
        }
    }

    let message_id = format!("<{}@draft>", uuid);
    let new_uid = match build_draft_rfc822(&req, &session.email, &message_id) {
        Err(e) => {
            tracing::warn!(error = %e, "Failed to build draft RFC822");
            None
        }
        Ok(bytes) => {
            match imap_client
                .append_message(&imap_creds, &drafts_folder, &bytes, &["\\Draft", "\\Seen"], Some(&message_id))
                .await
            {
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to append draft to IMAP");
                    None
                }
                Ok(uid) => uid,
            }
        }
    };

    // Store the mapping so the next save can expunge the right UID.
    // reply_message_id is passed only on first save (COALESCE in the upsert preserves it).
    let reply_mid = req.in_reply_to.as_deref();
    if let Err(e) = db::drafts::upsert_staging(&conn, &uuid, new_uid, reply_mid) {
        tracing::warn!(error = %e, "Failed to upsert draft_staging");
    }

    // Update total_count to thread count so the folder badge matches the list.
    // If this is a new draft (no prior IMAP copy) and the append succeeded,
    // add 1 because the new message isn't in the local cache yet.
    let thread_count = db::messages::count_threads(&conn, &drafts_folder).unwrap_or(0);
    let adjusted = if existing_uid.is_none() && new_uid.is_some() {
        thread_count + 1
    } else {
        thread_count
    };
    let _ = db::folders::set_folder_total_count(&conn, &drafts_folder, adjusted);

    // Invalidate the Drafts folder cache and notify connected clients.
    let _ = db::folders::invalidate_folder_freshness(&conn, &drafts_folder);
    event_bus.publish(&session.user_hash, MailEvent::FolderUpdated { folder: Some(drafts_folder) }).await;

    Ok(Json(SaveDraftResponse { status: "saved".to_string() }).into_response())
}

/// `DELETE /api/drafts/{uuid}` — Discard a draft: expunge from IMAP and delete staging.
pub async fn delete_draft_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(event_bus): Extension<Arc<EventBus>>,
    AxumPath(uuid): AxumPath<String>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    let imap_uid = db::drafts::get_staging(&conn, &uuid)
        .ok()
        .flatten()
        .and_then(|s| s.imap_uid);

    db::drafts::delete_staging(&conn, &uuid)
        .map_err(AppError::InternalError)?;

    // Expunge from IMAP.
    if let (Some(uid), Some(imap_host)) = (imap_uid, config.imap_host.as_deref()) {
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
            tracing::warn!(error = %e, uid = uid, "Failed to expunge draft from IMAP on delete");
        }
        let _ = db::folders::invalidate_folder_freshness(&conn, &drafts_folder);
        event_bus.publish(&session.user_hash, MailEvent::FolderUpdated { folder: Some(drafts_folder) }).await;
    }

    // Clean up attachment files from disk.
    let att_dir = Path::new(&config.data_dir)
        .join(&session.user_hash)
        .join("attachments")
        .join(&uuid);
    if att_dir.exists()
        && let Err(e) = tokio::fs::remove_dir_all(&att_dir).await
    {
        tracing::warn!(error = %e, path = %att_dir.display(), "Failed to clean up attachment directory");
    }

    Ok(Json(DeleteResponse { status: "deleted".to_string() }).into_response())
}

// ---------------------------------------------------------------------------
// RFC822 builder
// ---------------------------------------------------------------------------

/// Build a minimal RFC822 message from draft fields for IMAP APPEND.
/// `message_id` is set as the `Message-ID` header so the UID can be retrieved
/// after APPEND via UID SEARCH, and so the UUID can be recovered when the
/// draft is reopened from the IMAP Drafts folder.
pub fn build_draft_rfc822(req: &SaveDraftRequest, from_email: &str, message_id: &str) -> Result<Vec<u8>, String> {
    use lettre::message::{header::ContentType, Mailbox, MultiPart, SinglePart};

    let from_mailbox: Mailbox = from_email
        .parse()
        .map_err(|e: lettre::address::AddressError| e.to_string())?;

    let mut builder = lettre::Message::builder()
        .message_id(Some(message_id.to_string()))
        .from(from_mailbox)
        .subject(&req.subject);

    for addr in req.to.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(mailbox) = addr.parse::<Mailbox>() {
            builder = builder.to(mailbox);
        }
    }
    for addr in req.cc.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(mailbox) = addr.parse::<Mailbox>() {
            builder = builder.cc(mailbox);
        }
    }
    for addr in req.bcc.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(mailbox) = addr.parse::<Mailbox>() {
            builder = builder.bcc(mailbox);
        }
    }
    if let Some(ref irt) = req.in_reply_to {
        builder = builder.in_reply_to(irt.clone());
    }
    if let Some(ref refs) = req.references {
        builder = builder.references(refs.clone());
    }

    let email = if let Some(ref html) = req.html_body {
        builder
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .content_type(ContentType::TEXT_PLAIN)
                            .body(req.text_body.clone()),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .content_type(ContentType::TEXT_HTML)
                            .body(html.clone()),
                    ),
            )
            .map_err(|e| e.to_string())?
    } else {
        builder
            .body(req.text_body.clone())
            .map_err(|e| e.to_string())?
    };

    Ok(email.formatted())
}
