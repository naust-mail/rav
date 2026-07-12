use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

use crate::auth::session::SessionState;
use crate::db;
use crate::db::vacation::UpdateVacationResponder;
use crate::error::AppError;

/// `GET /api/settings/vacation`
pub async fn get_vacation_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
) -> Result<Response, AppError> {
    let vacation = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        db::vacation::get_vacation(conn)
    })
    .await
    .map_err(AppError::InternalError)?;
    Ok(Json(vacation).into_response())
}

/// `PUT /api/settings/vacation`
pub async fn update_vacation_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(body): Json<UpdateVacationResponder>,
) -> Result<Response, AppError> {
    let vacation = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::vacation::update_vacation(conn, &body)
    })
    .await
    .map_err(AppError::InternalError)?;
    Ok(Json(vacation).into_response())
}
