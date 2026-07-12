use std::sync::Arc;

use axum::extract::Path;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::session::SessionState;
use crate::db;
use crate::db::contact_groups::ContactGroup;
use crate::db::contacts::Contact;
use crate::error::AppError;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateGroupBody {
    pub name: String,
}

#[derive(Deserialize)]
pub struct AddMemberBody {
    pub contact_id: String,
}

#[derive(Serialize)]
pub struct ListGroupsResponse {
    pub groups: Vec<ContactGroup>,
}

#[derive(Serialize)]
pub struct GroupMembersResponse {
    pub members: Vec<Contact>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/contact-groups`
pub async fn list_groups_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
) -> Result<Response, AppError> {
    let groups = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        db::contact_groups::list_groups(conn)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(ListGroupsResponse { groups }).into_response())
}

/// `POST /api/contact-groups`
pub async fn create_group_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(body): Json<CreateGroupBody>,
) -> Result<Response, AppError> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("Group name is required".to_string()));
    }

    let id = Uuid::new_v4().to_string();
    db::pool::with_user_db(&db_pool_manager, &session.user_hash, {
        let id = id.clone();
        let name = name.clone();
        move |conn| db::contact_groups::create_group(conn, &id, &name)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(serde_json::json!({ "id": id, "name": name })).into_response())
}

/// `PUT /api/contact-groups/{id}`
pub async fn update_group_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(id): Path<String>,
    Json(body): Json<CreateGroupBody>,
) -> Result<Response, AppError> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("Group name is required".to_string()));
    }

    let updated = db::pool::with_user_db(&db_pool_manager, &session.user_hash, {
        let id = id.clone();
        let name = name.clone();
        move |conn| db::contact_groups::update_group(conn, &id, &name)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    if updated {
        Ok(Json(serde_json::json!({ "id": id, "name": name })).into_response())
    } else {
        Err(AppError::NotFound(format!("Group '{id}' not found")))
    }
}

/// `DELETE /api/contact-groups/{id}`
pub async fn delete_group_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let deleted = db::pool::with_user_db(&db_pool_manager, &session.user_hash, {
        let id = id.clone();
        move |conn| db::contact_groups::delete_group(conn, &id)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    if deleted {
        Ok(Json(serde_json::json!({ "status": "deleted" })).into_response())
    } else {
        Err(AppError::NotFound(format!("Group '{id}' not found")))
    }
}

/// `GET /api/contact-groups/{id}/members`
pub async fn list_members_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let members = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::contact_groups::list_group_members(conn, &id)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(GroupMembersResponse { members }).into_response())
}

/// `POST /api/contact-groups/{id}/members`
pub async fn add_member_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(id): Path<String>,
    Json(body): Json<AddMemberBody>,
) -> Result<Response, AppError> {
    db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::contact_groups::add_member(conn, &id, &body.contact_id)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    Ok(Json(serde_json::json!({ "status": "ok" })).into_response())
}

/// `DELETE /api/contact-groups/{id}/members/{contact_id}`
pub async fn remove_member_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path((id, contact_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let removed = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::contact_groups::remove_member(conn, &id, &contact_id)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    if removed {
        Ok(Json(serde_json::json!({ "status": "ok" })).into_response())
    } else {
        Err(AppError::NotFound("Member not found in group".to_string()))
    }
}
