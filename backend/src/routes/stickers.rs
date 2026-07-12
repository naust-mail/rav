use axum::extract::{Path, Query};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};

use crate::auth::session::SessionState;
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
    Extension(db_pool_manager): Extension<std::sync::Arc<db::pool::DbPoolManager>>,
    Extension(session): Extension<SessionState>,
    Query(params): Query<ListStickersParams>,
) -> Result<Response, AppError> {
    let stickers = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::stickers::list_stickers(conn, &params.from, &params.to)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    Ok(Json(StickersResponse { stickers }).into_response())
}

/// `PUT /api/calendar/stickers/{date}` - assign or replace a sticker on a date.
pub async fn put_sticker(
    Extension(db_pool_manager): Extension<std::sync::Arc<db::pool::DbPoolManager>>,
    Extension(session): Extension<SessionState>,
    Path(date): Path<String>,
    Json(body): Json<PutStickerRequest>,
) -> Result<Response, AppError> {
    if body.sticker_id.is_empty() {
        return Err(AppError::BadRequest("sticker_id is required".to_string()));
    }
    let sticker = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::stickers::put_sticker(conn, &date, &body.sticker_id)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    Ok(Json(sticker).into_response())
}

/// `DELETE /api/calendar/stickers/{date}` - remove sticker from a date.
pub async fn delete_sticker(
    Extension(db_pool_manager): Extension<std::sync::Arc<db::pool::DbPoolManager>>,
    Extension(session): Extension<SessionState>,
    Path(date): Path<String>,
) -> Result<Response, AppError> {
    db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::stickers::delete_sticker(conn, &date)
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    Ok(axum::http::StatusCode::NO_CONTENT.into_response())
}
