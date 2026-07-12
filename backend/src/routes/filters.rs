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

/// Owned copy of `MessageContext`'s fields, used to carry a message's
/// filter-matching context into a `with_user_db` closure (which must be
/// `'static`, so it can't hold borrows into the message list).
struct OwnedMessageContext {
    from_address: String,
    to_addresses: String,
    cc_addresses: String,
    subject: String,
    body_snippet: String,
    size: u32,
    has_attachments: bool,
    is_reply: bool,
}

/// Spawn a background task to push the given filter set to ManageSieve.
/// No-op if sieve_host is not configured. Failures are logged, not propagated.
fn push_sieve_async(config: &Arc<AppConfig>, session: &SessionState, rules: Vec<db::filters::FilterRule>) {
    if config.sieve_host.is_none() {
        return;
    }
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
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
) -> Result<Response, AppError> {
    let rules = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        db::filters::list_filters(conn)
    })
    .await
    .map_err(AppError::InternalError)?;
    Ok(Json(serde_json::json!({ "rules": rules })).into_response())
}

/// `POST /api/filters`
pub async fn create_filter_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(body): Json<CreateFilterRule>,
) -> Result<Response, AppError> {
    let (rule, rules) = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        let rule = db::filters::create_filter(conn, &body)?;
        let rules = db::filters::list_filters(conn).unwrap_or_default();
        Ok((rule, rules))
    })
    .await
    .map_err(AppError::BadRequest)?;
    push_sieve_async(&config, &session, rules);
    Ok(Json(rule).into_response())
}

/// `PUT /api/filters/{id}`
pub async fn update_filter_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateFilterRule>,
) -> Result<Response, AppError> {
    let (rule, rules) = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        let rule = db::filters::update_filter(conn, &id, &body)?;
        let rules = db::filters::list_filters(conn).unwrap_or_default();
        Ok((rule, rules))
    })
    .await
    .map_err(AppError::BadRequest)?;
    let rule = rule.ok_or_else(|| AppError::NotFound("Filter rule not found".to_string()))?;
    push_sieve_async(&config, &session, rules);
    Ok(Json(rule).into_response())
}

/// `DELETE /api/filters/{id}`
pub async fn delete_filter_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let (deleted, rules) = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        let deleted = db::filters::delete_filter(conn, &id)?;
        let rules = db::filters::list_filters(conn).unwrap_or_default();
        Ok((deleted, rules))
    })
    .await
    .map_err(AppError::InternalError)?;
    if !deleted {
        return Err(AppError::NotFound("Filter rule not found".to_string()));
    }
    push_sieve_async(&config, &session, rules);
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
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(body): Json<ReorderBody>,
) -> Result<Response, AppError> {
    let rules = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::filters::reorder_filters(conn, &body.ids)?;
        db::filters::list_filters(conn)
    })
    .await
    .map_err(AppError::InternalError)?;
    push_sieve_async(&config, &session, rules.clone());
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
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
) -> Result<Response, AppError> {
    let messages = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        db::messages::list_messages_in_folder(conn, "INBOX")
    })
    .await
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
        let owned_ctx = OwnedMessageContext {
            from_address: msg.from_address.clone(),
            to_addresses: msg.to_addresses.clone(),
            cc_addresses: msg.cc_addresses.clone(),
            subject: msg.subject.clone(),
            body_snippet: msg.snippet.clone(),
            size: msg.size,
            has_attachments: msg.has_attachments,
            is_reply: msg.in_reply_to.is_some(),
        };
        let matched = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
            db::filters::matching_rules(conn, &MessageContext {
                from_address: &owned_ctx.from_address,
                to_addresses: &owned_ctx.to_addresses,
                cc_addresses: &owned_ctx.cc_addresses,
                subject: &owned_ctx.subject,
                body_snippet: &owned_ctx.body_snippet,
                size: owned_ctx.size,
                has_attachments: owned_ctx.has_attachments,
                is_reply: owned_ctx.is_reply,
            })
        })
        .await;
        let matched = match matched {
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
                            Ok(_) => {
                                let uid = msg.uid;
                                let _ = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
                                    db::messages::update_message_flags(conn, "INBOX", uid, "\\Seen")
                                }).await;
                            }
                            Err(e) => errors.push(format!("uid {} mark_read: {e}", msg.uid)),
                        }
                    }
                    "mark_starred" => {
                        match imap_client.add_flags(&creds, "INBOX", msg.uid, &["\\Flagged"]).await {
                            Ok(_) => {
                                let uid = msg.uid;
                                let _ = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
                                    db::messages::update_message_flags(conn, "INBOX", uid, "\\Flagged")
                                }).await;
                            }
                            Err(e) => errors.push(format!("uid {} mark_starred: {e}", msg.uid)),
                        }
                    }
                    "move" => {
                        if let Some(ref target) = action.action_value {
                            match imap_client.move_message(&creds, "INBOX", msg.uid, target).await {
                                Ok(_) => {
                                    let uid = msg.uid;
                                    let _ = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
                                        db::messages::delete_message(conn, "INBOX", uid)
                                    }).await;
                                    continue 'msg;
                                }
                                Err(e) => errors.push(format!("uid {} move: {e}", msg.uid)),
                            }
                        }
                    }
                    "delete" => {
                        match imap_client.move_message(&creds, "INBOX", msg.uid, "Trash").await {
                            Ok(_) => {
                                let uid = msg.uid;
                                let _ = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
                                    db::messages::delete_message(conn, "INBOX", uid)
                                }).await;
                                continue 'msg;
                            }
                            Err(e) => errors.push(format!("uid {} delete: {e}", msg.uid)),
                        }
                    }
                    "tag" => {
                        if let Some(ref tag_id) = action.action_value {
                            let uid = msg.uid;
                            let tag_id = tag_id.clone();
                            let result = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
                                db::tags::add_tag_to_message(conn, &tag_id, uid, "INBOX")
                            }).await;
                            if let Err(e) = result {
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
