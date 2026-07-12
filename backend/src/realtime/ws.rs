use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::WebSocketUpgrade;
use axum::response::{IntoResponse, Response};
use axum::Extension;
use futures::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc};

use crate::auth::session::{AccountSession, SessionStore};
use crate::imap::client::ImapCredentials;
use crate::mail_transport::MailTransport;
use crate::realtime::events::EventBus;
use crate::realtime::idle::IdleManager;

fn extract_browser_id(cookie_header: &str) -> Option<String> {
    for segment in cookie_header.split(';') {
        let trimmed = segment.trim();
        if let Some(id) = trimmed.strip_prefix("rav_browser=") {
            let id = id.trim();
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
    Extension(store): Extension<Arc<SessionStore>>,
    Extension(transport): Extension<Arc<MailTransport>>,
    Extension(event_bus): Extension<Arc<EventBus>>,
    Extension(idle_manager): Extension<Arc<IdleManager>>,
    Extension(sync_worker_manager): Extension<Arc<super::worker::SyncWorkerManager>>,
) -> Response {
    let browser_id = headers
        .get_all("cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(extract_browser_id);

    let Some(browser_id) = browser_id else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            "Missing browser session",
        )
            .into_response();
    };

    let accounts = store.get_browser_accounts(&browser_id);

    if accounts.is_empty() {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            "No active accounts",
        )
            .into_response();
    }

    ws.on_upgrade(move |socket| {
        handle_socket_multi_account(
            socket,
            accounts,
            transport,
            event_bus,
            idle_manager,
            sync_worker_manager,
        )
    })
}

async fn handle_socket_multi_account(
    socket: WebSocket,
    accounts: Vec<AccountSession>,
    transport: Arc<MailTransport>,
    event_bus: Arc<EventBus>,
    idle_manager: Arc<IdleManager>,
    sync_worker_manager: Arc<super::worker::SyncWorkerManager>,
) {
    tracing::info!(account_count = accounts.len(), "WebSocket connected");

    let mut sync_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let mut forward_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    let (event_tx, mut event_rx) = mpsc::channel::<(String, crate::realtime::events::MailEvent)>(64);

    for session in &accounts {
        let user_hash = session.user_hash.clone();
        let account_id = session.account_id.clone();

        let mut rx = event_bus.subscribe(&user_hash).await;
        let tx = event_tx.clone();
        let aid = account_id.clone();

        let forward_handle = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if tx.send((aid.clone(), event)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        });
        forward_handles.push(forward_handle);

        let creds = ImapCredentials {
            host: session.imap_host.clone(),
            port: session.imap_port,
            tls: session.imap_tls,
            email: session.email.clone(),
            password: session.password.clone(),
        };

        idle_manager
            .start_idle(
                user_hash.clone(),
                "INBOX".to_string(),
                creds.clone(),
                transport.clone(),
                sync_worker_manager.clone(),
            )
            .await;

        let bell = sync_worker_manager.ensure_worker(user_hash, creds);

        // Keep poking the worker every SYNC_INTERVAL_SECS for as long as this
        // connection is open — the worker itself has no opinion on cadence,
        // it just reacts to wake-ups. Aborted on disconnect below; the worker
        // keeps running for other tabs/callers and self-terminates if nobody
        // rings its bell for a while.
        let keepalive_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(super::sync::SYNC_INTERVAL_SECS),
            );
            // Skip the first immediate tick — IDLE + initial list_messages handles that.
            interval.tick().await;
            loop {
                interval.tick().await;
                bell.notify_one();
            }
        });
        sync_handles.push(keepalive_handle);
    }

    drop(event_tx);

    let (mut ws_tx, mut ws_rx) = socket.split();
    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(30));

    loop {
        tokio::select! {
            Some((account_id, mail_event)) = event_rx.recv() => {
                let msg = serde_json::json!({
                    "accountId": account_id,
                    "event": mail_event,
                });
                if let Ok(json) = serde_json::to_string(&msg)
                    && ws_tx.send(Message::Text(json.into())).await.is_err()
                {
                    break;
                }
            }

            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }

            _ = ping_interval.tick() => {
                if ws_tx.send(Message::Ping(vec![].into())).await.is_err() {
                    break;
                }
            }
        }
    }

    tracing::info!("WebSocket disconnected");

    for handle in sync_handles {
        handle.abort();
    }

    for handle in forward_handles {
        handle.abort();
    }

    for session in &accounts {
        idle_manager.stop_all(&session.user_hash).await;
        event_bus.cleanup(&session.user_hash).await;
    }
}
