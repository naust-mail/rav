use std::sync::Arc;

use axum::extract::Path;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::db;
use crate::db::filters::{CreateFilterRule, UpdateFilterRule};
use crate::error::AppError;

/// `GET /api/filters`
pub async fn list_filters_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    let rules = db::filters::list_filters(&conn).map_err(AppError::InternalError)?;
    Ok(Json(serde_json::json!({ "rules": rules })).into_response())
}

/// `POST /api/filters`
pub async fn create_filter_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Json(body): Json<CreateFilterRule>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    let rule = db::filters::create_filter(&conn, &body)
        .map_err(AppError::BadRequest)?;
    Ok(Json(rule).into_response())
}

/// `PUT /api/filters/{id}`
pub async fn update_filter_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateFilterRule>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    let rule = db::filters::update_filter(&conn, &id, &body)
        .map_err(AppError::BadRequest)?
        .ok_or_else(|| AppError::NotFound("Filter rule not found".to_string()))?;
    Ok(Json(rule).into_response())
}

/// `DELETE /api/filters/{id}`
pub async fn delete_filter_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    let deleted = db::filters::delete_filter(&conn, &id)
        .map_err(AppError::InternalError)?;
    if !deleted {
        return Err(AppError::NotFound("Filter rule not found".to_string()));
    }
    Ok(Json(serde_json::json!({ "status": "ok" })).into_response())
}
