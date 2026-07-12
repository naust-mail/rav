use std::sync::Arc;

use axum::extract::Path as AxumPath;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::db;
use crate::db::outbox::NewOutboxEntry;
use crate::error::AppError;
use crate::realtime::events::{EventBus, MailEvent};
use crate::realtime::outbox_worker::OutboxWorkerManager;
use crate::routes::send::{self, SendRequest};

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct EnqueueResponse {
    pub id: String,
    pub send_after: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct ListResponse {
    pub entries: Vec<db::outbox::OutboxEntry>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct StatusResponse {
    pub status: String,
}

/// `POST /api/outbox` — Queue a message for sending after the user's
/// undo-send delay. Returns immediately; the actual SMTP send happens in
/// the background via `OutboxWorkerManager`.
pub async fn enqueue_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(outbox_worker_manager): Extension<Arc<OutboxWorkerManager>>,
    Extension(event_bus): Extension<Arc<EventBus>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(req): Json<SendRequest>,
) -> Result<Response, AppError> {
    send::validate_send_request(&req)?;

    if config.smtp_host.is_none() {
        return Err(AppError::ServiceUnavailable("SMTP server not configured".to_string()));
    }

    let pgp_json = req
        .pgp
        .as_ref()
        .map(send::resolve_pgp_params)
        .transpose()?
        .map(|p| serde_json::to_string(&p).map_err(|e| AppError::InternalError(e.to_string())))
        .transpose()?;

    let entry = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        let delay_secs = db::display_preferences::get_preferences(conn)?.undo_send_delay;
        let send_after = (Utc::now() + chrono::Duration::seconds(delay_secs.max(0)))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        let id = Uuid::new_v4().to_string();
        db::outbox::enqueue(conn, &NewOutboxEntry {
            id: &id,
            draft_id: req.draft_id.as_deref(),
            to_addrs: &req.to,
            cc_addrs: &req.cc,
            bcc_addrs: &req.bcc,
            subject: &req.subject,
            text_body: req.text_body.as_deref().unwrap_or(""),
            html_body: req.html_body.as_deref(),
            in_reply_to: req.in_reply_to.as_deref(),
            references_hdr: req.references.as_deref(),
            from_identity_id: req.from_identity_id,
            pgp_json: pgp_json.as_deref(),
            send_after: &send_after,
        })
    })
    .await
    .map_err(AppError::InternalError)?;

    tracing::debug!(
        id = %entry.id, user_hash = %session.user_hash, state = %entry.state, send_after = %entry.send_after,
        "Outbox: entry enqueued",
    );

    outbox_worker_manager
        .ensure_worker(session.user_hash.clone(), session.email.clone(), session.password.clone())
        .notify_one();

    event_bus
        .publish(&session.user_hash, MailEvent::OutboxStateChanged {
            id: entry.id.clone(),
            state: "scheduled".to_string(),
            fail_reason: None,
        })
        .await;

    Ok(Json(EnqueueResponse { id: entry.id, send_after: entry.send_after }).into_response())
}

/// `GET /api/outbox` — List entries still scheduled or permanently failed.
pub async fn list_handler(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
) -> Result<Response, AppError> {
    let entries = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        db::outbox::list_visible(conn)
    })
    .await
    .map_err(AppError::InternalError)?;

    Ok(Json(ListResponse { entries }).into_response())
}

/// `DELETE /api/outbox/{id}` — Undo (while still scheduled) or discard
/// (while failed) a queued send. Rejected if the send is already in flight.
pub async fn delete_handler(
    Extension(session): Extension<SessionState>,
    Extension(event_bus): Extension<Arc<EventBus>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response, AppError> {
    enum DeleteOutcome {
        Deleted,
        NotFound,
        InProgress,
    }

    let outcome = db::pool::with_user_db(&db_pool_manager, &session.user_hash, {
        let id = id.clone();
        move |conn| {
            let Some(entry) = db::outbox::get(conn, &id)? else {
                return Ok(DeleteOutcome::NotFound);
            };

            if entry.state == "sending" {
                return Ok(DeleteOutcome::InProgress);
            }

            db::outbox::delete(conn, &id)?;
            Ok(DeleteOutcome::Deleted)
        }
    })
    .await
    .map_err(AppError::InternalError)?;

    match outcome {
        DeleteOutcome::Deleted => {}
        DeleteOutcome::NotFound => return Err(AppError::NotFound("Outbox entry not found".to_string())),
        DeleteOutcome::InProgress => return Err(AppError::BadRequest("Send already in progress, cannot cancel".to_string())),
    }

    event_bus
        .publish(&session.user_hash, MailEvent::OutboxStateChanged {
            id,
            state: "cancelled".to_string(),
            fail_reason: None,
        })
        .await;

    Ok(Json(StatusResponse { status: "cancelled".to_string() }).into_response())
}

/// `POST /api/outbox/{id}/retry` — Requeue a permanently failed entry for
/// an immediate retry (no additional undo delay).
pub async fn retry_handler(
    Extension(session): Extension<SessionState>,
    Extension(outbox_worker_manager): Extension<Arc<OutboxWorkerManager>>,
    Extension(event_bus): Extension<Arc<EventBus>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response, AppError> {
    let requeued = db::pool::with_user_db(&db_pool_manager, &session.user_hash, {
        let id = id.clone();
        move |conn| {
            let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            db::outbox::requeue(conn, &id, &now)
        }
    })
    .await
    .map_err(AppError::InternalError)?;
    if !requeued {
        return Err(AppError::NotFound("No failed outbox entry with that id".to_string()));
    }

    outbox_worker_manager
        .ensure_worker(session.user_hash.clone(), session.email.clone(), session.password.clone())
        .notify_one();

    event_bus
        .publish(&session.user_hash, MailEvent::OutboxStateChanged {
            id,
            state: "scheduled".to_string(),
            fail_reason: None,
        })
        .await;

    Ok(Json(StatusResponse { status: "scheduled".to_string() }).into_response())
}
