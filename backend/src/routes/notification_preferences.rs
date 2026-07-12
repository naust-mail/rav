use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

use crate::auth::session::SessionState;
use crate::db;
use crate::db::notification_preferences::UpdateNotificationPreferences;
use crate::error::AppError;

/// `GET /api/settings/notifications`
pub async fn get_notification_preferences(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
) -> Result<Response, AppError> {
    let prefs = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        db::notification_preferences::get_preferences(conn)
    })
    .await
    .map_err(AppError::InternalError)?;

    Ok(Json(prefs).into_response())
}

/// `PUT /api/settings/notifications`
pub async fn update_notification_preferences(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(data): Json<UpdateNotificationPreferences>,
) -> Result<Response, AppError> {
    let prefs = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::notification_preferences::update_preferences(conn, &data)
    })
    .await
    .map_err(AppError::InternalError)?;

    Ok(Json(prefs).into_response())
}
