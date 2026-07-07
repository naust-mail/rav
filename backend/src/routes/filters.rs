use std::sync::Arc;

use axum::extract::Path;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::db;
use crate::db::filters::{CreateFilterRule, MessageContext, UpdateFilterRule};
use crate::error::AppError;
use crate::imap::client::{ImapClient, ImapCredentials};
use crate::mail_transport::MailTransport;
use crate::smtp::client::{SendableMessage, SmtpClient, SmtpCredentials};

/// Spawn a background task to push the current filter set to ManageSieve.
/// No-op if sieve_host is not configured. Failures are logged, not propagated.
fn push_sieve_async(config: &Arc<AppConfig>, session: &SessionState, conn: &rusqlite::Connection) {
    if config.sieve_host.is_none() {
        return;
    }
    let rules = db::filters::list_filters(conn).unwrap_or_default();
    let config = Arc::clone(config);
    let email = session.email.clone();
    let password = session.password.clone();
    tokio::spawn(async move {
        crate::sieve::push_filters(&config, &email, &password, &rules).await;
    });
}

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
    push_sieve_async(&config, &session, &conn);
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
    push_sieve_async(&config, &session, &conn);
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
    push_sieve_async(&config, &session, &conn);
    Ok(Json(serde_json::json!({ "status": "ok" })).into_response())
}

#[derive(Deserialize)]
pub struct ReorderBody {
    /// Ordered list of filter rule IDs. Rules are assigned priority 0, 1, 2, ... in this order.
    pub ids: Vec<String>,
}

/// `PUT /api/filters/reorder`
pub async fn reorder_filters_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Json(body): Json<ReorderBody>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;
    db::filters::reorder_filters(&conn, &body.ids)
        .map_err(AppError::InternalError)?;
    let rules = db::filters::list_filters(&conn).map_err(AppError::InternalError)?;
    push_sieve_async(&config, &session, &conn);
    Ok(Json(serde_json::json!({ "rules": rules })).into_response())
}

/// `POST /api/filters/apply`
///
/// Runs all enabled filter rules against every message currently cached in INBOX.
/// Useful after creating or editing rules to apply them retroactively.
/// Returns the number of messages that matched at least one rule.
pub async fn apply_filters_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(transport): Extension<Arc<MailTransport>>,
    Extension(smtp_client): Extension<Arc<dyn SmtpClient>>,
) -> Result<Response, AppError> {
    let conn = db::pool::open_user_db(&config.data_dir, &session.user_hash)
        .map_err(|e| AppError::InternalError(format!("Database error: {e}")))?;

    let messages = db::messages::list_messages_in_folder(&conn, "INBOX")
        .map_err(|e| AppError::InternalError(format!("Failed to list messages: {e}")))?;

    let creds = ImapCredentials {
        host: session.imap_host.clone(),
        port: session.imap_port,
        tls: session.imap_tls,
        email: session.email.clone(),
        password: session.password.clone(),
    };

    let smtp_host = config.smtp_host.as_deref().unwrap_or_default().to_string();
    let smtp_creds_opt: Option<SmtpCredentials> = if smtp_host.is_empty() {
        None
    } else {
        Some(SmtpCredentials {
            host: smtp_host.clone(),
            connect_host: transport.smtp_connect_host.clone(),
            port: config.smtp_port,
            tls: config.tls_enabled,
            email: session.email.clone(),
            password: session.password.clone(),
            tls_params: transport.smtp_tls_params.clone(),
        })
    };

    let mut applied = 0u32;
    let mut errors: Vec<String> = Vec::new();

    'msg: for msg in &messages {
        let ctx = MessageContext {
            from_address: &msg.from_address,
            to_addresses: &msg.to_addresses,
            cc_addresses: &msg.cc_addresses,
            subject: &msg.subject,
            body_snippet: &msg.snippet,
            size: msg.size,
            has_attachments: msg.has_attachments,
            is_reply: msg.in_reply_to.is_some(),
        };

        let matched = match db::filters::matching_rules(&conn, &ctx) {
            Ok(m) => m,
            Err(e) => { errors.push(format!("uid {}: {e}", msg.uid)); continue; }
        };

        if matched.is_empty() {
            continue;
        }
        applied += 1;

        for rule in matched {
            for action in &rule.actions {
                match action.action_type.as_str() {
                    "mark_read" => {
                        match imap_client.add_flags(&creds, "INBOX", msg.uid, &["\\Seen"]).await {
                            Ok(_) => { let _ = db::messages::update_message_flags(&conn, "INBOX", msg.uid, "\\Seen"); }
                            Err(e) => errors.push(format!("uid {} mark_read: {e}", msg.uid)),
                        }
                    }
                    "mark_starred" => {
                        match imap_client.add_flags(&creds, "INBOX", msg.uid, &["\\Flagged"]).await {
                            Ok(_) => { let _ = db::messages::update_message_flags(&conn, "INBOX", msg.uid, "\\Flagged"); }
                            Err(e) => errors.push(format!("uid {} mark_starred: {e}", msg.uid)),
                        }
                    }
                    "move" => {
                        if let Some(ref target) = action.action_value {
                            match imap_client.move_message(&creds, "INBOX", msg.uid, target).await {
                                Ok(_) => {
                                    let _ = db::messages::delete_message(&conn, "INBOX", msg.uid);
                                    continue 'msg;
                                }
                                Err(e) => errors.push(format!("uid {} move: {e}", msg.uid)),
                            }
                        }
                    }
                    "delete" => {
                        match imap_client.move_message(&creds, "INBOX", msg.uid, "Trash").await {
                            Ok(_) => {
                                let _ = db::messages::delete_message(&conn, "INBOX", msg.uid);
                                continue 'msg;
                            }
                            Err(e) => errors.push(format!("uid {} delete: {e}", msg.uid)),
                        }
                    }
                    "tag" => {
                        if let Some(ref tag_id) = action.action_value {
                            if let Err(e) = db::tags::add_tag_to_message(&conn, tag_id, msg.uid, "INBOX") {
                                errors.push(format!("uid {} tag: {e}", msg.uid));
                            }
                        }
                    }
                    "forward" => {
                        if let (Some(forward_to), Some(smtp_creds)) = (&action.action_value, &smtp_creds_opt) {
                            let fwd = SendableMessage {
                                from: session.email.clone(),
                                to: vec![forward_to.clone()],
                                cc: vec![],
                                bcc: vec![],
                                subject: format!("Fwd: {}", msg.subject),
                                text_body: format!(
                                    "---------- Forwarded message ----------\nFrom: {}\nSubject: {}\n\n{}",
                                    msg.from_address, msg.subject, msg.snippet
                                ),
                                html_body: None,
                                in_reply_to: None,
                                references: None,
                                attachments: vec![],
                                auto_submitted: false,
                                pgp: None,
                            };
                            if let Err(e) = smtp_client.send_message(smtp_creds, &fwd).await {
                                errors.push(format!("uid {} forward: {e}", msg.uid));
                            }
                        }
                    }
                    _ => {}
                }
            }
            if rule.stop_processing {
                break;
            }
        }
    }

    Ok(Json(serde_json::json!({ "applied": applied, "errors": errors })).into_response())
}
