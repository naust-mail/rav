use std::path::Path;
use std::sync::Arc;

use axum::extract::Path as AxumPath;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::db;
use crate::error::AppError;
use crate::imap::client::{ImapClient, ImapCredentials};
use crate::realtime::events::{EventBus, MailEvent};

/// Serializes concurrent save/delete requests for the same draft UUID.
///
/// Without this, two overlapping saves (autosave timer racing a manual
/// save, or a retried request) both read the same stale `imap_uid` from
/// `draft_staging`, both independently expunge+append against IMAP, and
/// whichever response writes `draft_staging` last "wins" — the other
/// request's freshly-appended IMAP message is orphaned forever (nothing
/// ever points at it to clean it up). Locking per `(user_hash, uuid)`
/// makes the whole expunge-append-upsert sequence atomic with respect to
/// itself.
#[derive(Default)]
pub struct DraftLocks {
    locks: DashMap<String, Arc<AsyncMutex<()>>>,
}

impl DraftLocks {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn with_lock<F, Fut, T>(&self, user_hash: &str, uuid: &str, f: F) -> T
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        let key = format!("{user_hash}:{uuid}");
        let mutex = self
            .locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();

        let result = {
            let _guard = mutex.lock().await;
            f().await
        };

        // Best-effort cleanup so the map doesn't grow forever: only remove
        // the entry if nobody else has claimed a reference to it in the
        // meantime (e.g. a request that's queued up behind us).
        drop(mutex);
        self.locks.remove_if(&key, |_, v| Arc::strong_count(v) <= 1);

        result
    }
}

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
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(body): Json<ReplyForRequest>,
) -> Result<Response, AppError> {
    let found = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        let staging = db::drafts::find_by_reply_message_id(conn, &body.message_id)?;
        let Some(staging) = staging else {
            return Ok(None);
        };
        let draft_folder_name = db::folders::find_folder_by_attribute(conn, "\\Drafts")
            .ok()
            .flatten()
            .unwrap_or_else(|| "Drafts".to_string());
        Ok(Some((staging, draft_folder_name)))
    })
    .await
    .map_err(AppError::InternalError)?;

    match found {
        None => Ok(axum::http::StatusCode::NOT_FOUND.into_response()),
        Some((staging, draft_folder_name)) => {
            let cipher = crate::folder_cipher::FolderCipher::new(&session.folder_key);
            Ok(Json(serde_json::json!({
                "uuid": staging.uuid,
                "imap_uid": staging.imap_uid,
                "draft_folder_id": cipher.encrypt(&draft_folder_name),
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
    Extension(draft_locks): Extension<Arc<DraftLocks>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    AxumPath(uuid): AxumPath<String>,
    Json(req): Json<SaveDraftRequest>,
) -> Result<Response, AppError> {
    let user_hash = session.user_hash.clone();
    draft_locks
        .with_lock(&user_hash, &uuid.clone(), || {
            save_draft_locked(session, config, imap_client, event_bus, db_pool_manager, uuid, req)
        })
        .await
}

struct DraftPreState {
    existing_uid: Option<u32>,
    drafts_folder: String,
}

#[allow(clippy::too_many_arguments)]
async fn save_draft_locked(
    session: SessionState,
    config: Arc<AppConfig>,
    imap_client: Arc<dyn ImapClient>,
    event_bus: Arc<EventBus>,
    db_pool_manager: Arc<db::pool::DbPoolManager>,
    uuid: String,
    req: SaveDraftRequest,
) -> Result<Response, AppError> {
    let Some(imap_host) = config.imap_host.as_deref() else {
        return Err(AppError::InternalError("IMAP not configured".to_string()));
    };

    let DraftPreState { existing_uid, drafts_folder } = db::pool::with_user_db(&db_pool_manager, &session.user_hash, {
        let uuid = uuid.clone();
        move |conn| {
            let existing_uid = db::drafts::get_staging(conn, &uuid)
                .ok()
                .flatten()
                .and_then(|s| s.imap_uid);
            let drafts_folder = db::folders::find_folder_by_attribute(conn, "\\Drafts")
                .ok()
                .flatten()
                .unwrap_or_else(|| "Drafts".to_string());
            Ok(DraftPreState { existing_uid, drafts_folder })
        }
    })
    .await
    .map_err(AppError::InternalError)?;

    let imap_creds = ImapCredentials {
        host: imap_host.to_string(),
        port: config.imap_port,
        tls: config.tls_enabled,
        email: session.email.clone(),
        password: session.password.clone(),
    };

    // Expunge the previous IMAP copy before writing the new one.
    if let Some(old_uid) = existing_uid
        && let Err(e) = imap_client.expunge_message(&imap_creds, &drafts_folder, old_uid).await {
            tracing::warn!(error = %e, uid = old_uid, "Failed to expunge old draft from IMAP");
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

    let _ = db::pool::with_user_db(&db_pool_manager, &session.user_hash, {
        let drafts_folder = drafts_folder.clone();
        move |conn| {
            // Store the mapping so the next save can expunge the right UID.
            // reply_message_id is passed only on first save (COALESCE in the upsert preserves it).
            let reply_mid = req.in_reply_to.as_deref();
            if let Err(e) = db::drafts::upsert_staging(conn, &uuid, new_uid, reply_mid) {
                tracing::warn!(error = %e, "Failed to upsert draft_staging");
            }

            // Update total_count to thread count so the folder badge matches the list.
            // If this is a new draft (no prior IMAP copy) and the append succeeded,
            // add 1 because the new message isn't in the local cache yet.
            let thread_count = db::messages::count_threads(conn, &drafts_folder).unwrap_or(0);
            let adjusted = if existing_uid.is_none() && new_uid.is_some() {
                thread_count + 1
            } else {
                thread_count
            };
            let _ = db::folders::set_folder_total_count(conn, &drafts_folder, adjusted);

            // Invalidate the Drafts folder cache.
            let _ = db::folders::invalidate_folder_freshness(conn, &drafts_folder);
            Ok(())
        }
    })
    .await;

    event_bus.publish(&session.user_hash, MailEvent::FolderUpdated { folder: Some(drafts_folder) }).await;

    Ok(Json(SaveDraftResponse { status: "saved".to_string() }).into_response())
}

/// `DELETE /api/drafts/{uuid}` — Discard a draft: expunge from IMAP and delete staging.
pub async fn delete_draft_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(event_bus): Extension<Arc<EventBus>>,
    Extension(draft_locks): Extension<Arc<DraftLocks>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    AxumPath(uuid): AxumPath<String>,
) -> Result<Response, AppError> {
    let user_hash = session.user_hash.clone();
    draft_locks
        .with_lock(&user_hash, &uuid.clone(), || {
            delete_draft_locked(session, config, imap_client, event_bus, db_pool_manager, uuid)
        })
        .await
}

async fn delete_draft_locked(
    session: SessionState,
    config: Arc<AppConfig>,
    imap_client: Arc<dyn ImapClient>,
    event_bus: Arc<EventBus>,
    db_pool_manager: Arc<db::pool::DbPoolManager>,
    uuid: String,
) -> Result<Response, AppError> {
    let imap_uid = db::pool::with_user_db(&db_pool_manager, &session.user_hash, {
        let uuid = uuid.clone();
        move |conn| {
            let imap_uid = db::drafts::get_staging(conn, &uuid)
                .ok()
                .flatten()
                .and_then(|s| s.imap_uid);
            db::drafts::delete_staging(conn, &uuid)?;
            Ok(imap_uid)
        }
    })
    .await
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
        let drafts_folder = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
            Ok(db::folders::find_folder_by_attribute(conn, "\\Drafts")
                .ok()
                .flatten()
                .unwrap_or_else(|| "Drafts".to_string()))
        })
        .await
        .map_err(AppError::InternalError)?;
        if let Err(e) = imap_client.expunge_message(&imap_creds, &drafts_folder, uid).await {
            tracing::warn!(error = %e, uid = uid, "Failed to expunge draft from IMAP on delete");
        }
        let _ = db::pool::with_user_db(&db_pool_manager, &session.user_hash, {
            let drafts_folder = drafts_folder.clone();
            move |conn| db::folders::invalidate_folder_freshness(conn, &drafts_folder)
        })
        .await;
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
