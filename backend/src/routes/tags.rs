use std::sync::Arc;

use axum::extract::{Path, Query};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::db;
use crate::db::tags::{MessageTag, Tag};
use crate::error::AppError;
use crate::folder_cipher::FolderId;
use crate::routes::messages::types::{default_per_page, MessageSummary};

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct CreateTagBody {
    pub name: String,
    pub color: Option<String>,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct UpdateTagBody {
    pub name: String,
    pub color: String,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct TagMessageBody {
    pub message_uid: u32,
    pub message_folder: FolderId,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct BulkTagBody {
    pub messages: Vec<TagMessageRef>,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct TagMessageRef {
    pub uid: u32,
    pub folder: FolderId,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct PaginationQuery {
    #[serde(default)]
    pub page: u32,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
}

#[derive(Serialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct ListTagsResponse {
    pub tags: Vec<Tag>,
}

#[derive(Serialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct TagMessagesResponse {
    pub messages: Vec<MessageSummary>,
    pub total_count: u32,
    pub page: u32,
    pub per_page: u32,
}

#[derive(Serialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct MessageTagsResponse {
    pub tags: Vec<MessageTag>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `MessageSummary` from a cached message and its tags.
fn build_summary(
    m: db::messages::CachedMessage,
    tags: Vec<MessageTag>,
) -> MessageSummary {
    let unread = if m.flags.contains("\\Seen") { 0 } else { 1 };
    MessageSummary {
        uid: m.uid,
        // Placeholder - callers overwrite folder_id with an encrypted token
        // before serializing (the plaintext must never reach the response).
        folder_id: FolderId::default(),
        folder_name: m.folder,
        subject: m.subject,
        from_address: m.from_address,
        from_name: m.from_name,
        to_addresses: m.to_addresses,
        date: m.date,
        flags: m.flags,
        size: m.size,
        has_attachments: m.has_attachments,
        snippet: m.snippet,
        reaction: m.reaction,
        tags,
        thread_count: 1,
        unread_count: unread,
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/tags`
pub async fn list_tags_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    let tags = db::tags::list_tags(&conn)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(ListTagsResponse { tags }).into_response())
}

/// `POST /api/tags`
pub async fn create_tag_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Json(body): Json<CreateTagBody>,
) -> Result<Response, AppError> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("Tag name is required".to_string()));
    }

    let color = body.color.as_deref().unwrap_or("#6b7280");

    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    let id = Uuid::new_v4().to_string();
    db::tags::create_tag(&conn, &id, name, color)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(serde_json::json!({ "id": id, "name": name, "color": color })).into_response())
}

/// `PUT /api/tags/{id}`
pub async fn update_tag_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTagBody>,
) -> Result<Response, AppError> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("Tag name is required".to_string()));
    }

    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    let updated = db::tags::update_tag(&conn, &id, name, &body.color)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    if updated {
        Ok(Json(serde_json::json!({ "id": id, "name": name, "color": body.color })).into_response())
    } else {
        Err(AppError::NotFound(format!("Tag '{id}' not found")))
    }
}

/// `DELETE /api/tags/{id}`
pub async fn delete_tag_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    let deleted = db::tags::delete_tag(&conn, &id)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    if deleted {
        Ok(Json(serde_json::json!({ "status": "deleted" })).into_response())
    } else {
        Err(AppError::NotFound(format!("Tag '{id}' not found")))
    }
}

/// `POST /api/tags/{id}/messages`
pub async fn tag_message_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Path(id): Path<String>,
    Json(body): Json<TagMessageBody>,
) -> Result<Response, AppError> {
    let folder = crate::folder_cipher::FolderCipher::new(&session.folder_key).decrypt(&body.message_folder)?;
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    db::tags::add_tag_to_message(&conn, &id, body.message_uid, &folder)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(serde_json::json!({ "status": "ok" })).into_response())
}

/// `DELETE /api/tags/{id}/messages/{folder}/{uid}`
pub async fn untag_message_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Path((id, folder_id, uid)): Path<(String, FolderId, u32)>,
) -> Result<Response, AppError> {
    let folder = crate::folder_cipher::FolderCipher::new(&session.folder_key).decrypt(&folder_id)?;
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    let removed = db::tags::remove_tag_from_message(&conn, &id, uid, &folder)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    if removed {
        Ok(Json(serde_json::json!({ "status": "ok" })).into_response())
    } else {
        Err(AppError::NotFound("Tag not found on message".to_string()))
    }
}

/// `POST /api/tags/{id}/messages/bulk`
pub async fn bulk_tag_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Path(id): Path<String>,
    Json(body): Json<BulkTagBody>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    let cipher = crate::folder_cipher::FolderCipher::new(&session.folder_key);
    let tx = conn.unchecked_transaction()
        .map_err(|e| AppError::InternalError(format!("Transaction error: {e}")))?;

    for msg in &body.messages {
        let folder = cipher.decrypt(&msg.folder)?;
        db::tags::add_tag_to_message(&tx, &id, msg.uid, &folder)
            .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    }

    tx.commit()
        .map_err(|e| AppError::InternalError(format!("Transaction commit error: {e}")))?;

    Ok(Json(serde_json::json!({ "status": "ok", "count": body.messages.len() })).into_response())
}

/// `GET /api/tags/{id}/messages`
pub async fn list_tag_messages_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Path(id): Path<String>,
    Query(query): Query<PaginationQuery>,
) -> Result<Response, AppError> {
    let cipher = crate::folder_cipher::FolderCipher::new(&session.folder_key);
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    let messages = db::tags::get_messages_by_tag(&conn, &id, query.page, query.per_page)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    let total_count = db::tags::count_messages_by_tag(&conn, &id)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    // Batch-fetch tags for returned messages
    let message_refs: Vec<(u32, &str)> = messages.iter().map(|m| (m.uid, m.folder.as_str())).collect();
    let tags_map = db::tags::get_tags_for_messages(&conn, &message_refs).unwrap_or_default();

    let summaries: Vec<MessageSummary> = messages
        .into_iter()
        .map(|m| {
            let msg_tags = tags_map
                .get(&(m.uid, m.folder.clone()))
                .cloned()
                .unwrap_or_default();
            let encrypted_folder = cipher.encrypt(&m.folder);
            let mut summary = build_summary(m, msg_tags);
            summary.folder_id = encrypted_folder;
            summary
        })
        .collect();

    Ok(Json(TagMessagesResponse {
        messages: summaries,
        total_count,
        page: query.page,
        per_page: query.per_page,
    })
    .into_response())
}

/// `GET /api/messages/{folder}/{uid}/tags`
pub async fn get_message_tags_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Path((folder_id, uid)): Path<(FolderId, u32)>,
) -> Result<Response, AppError> {
    let folder = crate::folder_cipher::FolderCipher::new(&session.folder_key).decrypt(&folder_id)?;
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    let tags = db::tags::get_message_tags(&conn, uid, &folder)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(MessageTagsResponse { tags }).into_response())
}
