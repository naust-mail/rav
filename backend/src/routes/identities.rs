use std::sync::Arc;

use axum::extract::Path;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

use crate::auth::session::SessionState;
use crate::db;
use crate::db::identities::{CreateIdentity, UpdateIdentity};
use crate::error::AppError;

/// `GET /api/identities` — list all sender identities.
pub async fn list_identities_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
) -> Result<Response, AppError> {
    let identities = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        db::identities::list_identities(conn)
    })
    .await
    .map_err(AppError::InternalError)?;

    Ok(Json(serde_json::json!({ "identities": identities })).into_response())
}

/// `GET /api/identities/:id` — get a single identity.
pub async fn get_identity_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let identity = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::identities::get_identity(conn, id)
    })
    .await
    .map_err(AppError::InternalError)?;

    match identity {
        Some(i) => Ok(Json(i).into_response()),
        None => Err(AppError::NotFound("Identity not found".to_string())),
    }
}

/// `POST /api/identities` — create a new identity.
pub async fn create_identity_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(data): Json<CreateIdentity>,
) -> Result<Response, AppError> {
    if data.email.trim().is_empty() {
        return Err(AppError::BadRequest("Email is required".to_string()));
    }

    let identity = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::identities::create_identity(conn, &data)
    })
    .await
    .map_err(AppError::InternalError)?;

    Ok((axum::http::StatusCode::CREATED, Json(identity)).into_response())
}

/// `PUT /api/identities/:id` — update an identity.
pub async fn update_identity_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(id): Path<i64>,
    Json(data): Json<UpdateIdentity>,
) -> Result<Response, AppError> {
    let identity = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::identities::update_identity(conn, id, &data)
    })
    .await
    .map_err(AppError::InternalError)?;

    match identity {
        Some(identity) => Ok(Json(identity).into_response()),
        None => Err(AppError::NotFound("Identity not found".to_string())),
    }
}

/// `DELETE /api/identities/:id` — delete an identity.
pub async fn delete_identity_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let deleted = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::identities::delete_identity(conn, id)
    })
    .await
    .map_err(AppError::InternalError)?;

    if deleted {
        Ok(Json(serde_json::json!({ "status": "deleted" })).into_response())
    } else {
        Err(AppError::NotFound("Identity not found".to_string()))
    }
}
