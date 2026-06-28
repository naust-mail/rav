use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::db;
use crate::db::vacation::UpdateVacationResponder;
use crate::error::AppError;

/// `GET /api/settings/vacation`
pub async fn get_vacation_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    let vacation = db::vacation::get_vacation(&conn)
        .map_err(AppError::InternalError)?;
    Ok(Json(vacation).into_response())
}

/// `PUT /api/settings/vacation`
pub async fn update_vacation_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Json(body): Json<UpdateVacationResponder>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    let vacation = db::vacation::update_vacation(&conn, &body)
        .map_err(AppError::InternalError)?;
    Ok(Json(vacation).into_response())
}
