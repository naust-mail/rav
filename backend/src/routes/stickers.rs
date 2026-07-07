use axum::extract::{Path, Query};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::db;
use crate::error::AppError;

#[derive(Deserialize)]
pub struct ListStickersParams {
    /// Inclusive start date (YYYY-MM-DD).
    pub from: String,
    /// Inclusive end date (YYYY-MM-DD).
    pub to: String,
}

#[derive(Deserialize)]
pub struct PutStickerRequest {
    pub sticker_id: String,
}

#[derive(Serialize)]
pub struct StickersResponse {
    pub stickers: Vec<db::stickers::CalendarSticker>,
}

/// `GET /api/calendar/stickers?from=YYYY-MM-DD&to=YYYY-MM-DD`
pub async fn list_stickers(
    Extension(config): Extension<std::sync::Arc<AppConfig>>,
    Extension(session): Extension<SessionState>,
    Query(params): Query<ListStickersParams>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    let stickers = db::stickers::list_stickers(&conn, &params.from, &params.to)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    Ok(Json(StickersResponse { stickers }).into_response())
}

/// `PUT /api/calendar/stickers/{date}` - assign or replace a sticker on a date.
pub async fn put_sticker(
    Extension(config): Extension<std::sync::Arc<AppConfig>>,
    Extension(session): Extension<SessionState>,
    Path(date): Path<String>,
    Json(body): Json<PutStickerRequest>,
) -> Result<Response, AppError> {
    if body.sticker_id.is_empty() {
        return Err(AppError::BadRequest("sticker_id is required".to_string()));
    }
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    let sticker = db::stickers::put_sticker(&conn, &date, &body.sticker_id)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    Ok(Json(sticker).into_response())
}

/// `DELETE /api/calendar/stickers/{date}` - remove sticker from a date.
pub async fn delete_sticker(
    Extension(config): Extension<std::sync::Arc<AppConfig>>,
    Extension(session): Extension<SessionState>,
    Path(date): Path<String>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    db::stickers::delete_sticker(&conn, &date)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    Ok(axum::http::StatusCode::NO_CONTENT.into_response())
}
