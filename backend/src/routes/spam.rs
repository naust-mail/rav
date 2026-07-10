use std::sync::Arc;

use axum::extract::Path;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::error::AppError;
use crate::folder_cipher::FolderId;
use crate::imap::client::{ImapClient, ImapCredentials};

/// `POST /api/messages/{folder}/{uid}/report-spam`
///
/// Fetches the raw RFC 822 bytes of the message and POSTs them to rspamd's
/// /learnspam endpoint. The frontend is responsible for moving the message to
/// Junk separately via the existing move API.
///
/// Returns 200 with `{ "trained": true }` when rspamd was called successfully,
/// or `{ "trained": false }` when RSPAMD_URL is not configured (not an error).
pub async fn report_spam_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(http_client): Extension<Arc<reqwest::Client>>,
    Path((folder_id, uid)): Path<(FolderId, u32)>,
) -> Result<Response, AppError> {
    let folder = crate::folder_cipher::FolderCipher::new(&session.folder_key).decrypt(&folder_id)?;
    let trained = learn_message(&session, &config, &imap_client, &http_client, &folder, uid, "learnspam").await?;
    Ok(Json(serde_json::json!({ "trained": trained })).into_response())
}

/// `POST /api/messages/{folder}/{uid}/report-ham`
///
/// Same as report-spam but teaches rspamd that this message is legitimate.
/// The frontend is responsible for moving the message to Inbox separately.
pub async fn report_ham_handler(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(imap_client): Extension<Arc<dyn ImapClient>>,
    Extension(http_client): Extension<Arc<reqwest::Client>>,
    Path((folder_id, uid)): Path<(FolderId, u32)>,
) -> Result<Response, AppError> {
    let folder = crate::folder_cipher::FolderCipher::new(&session.folder_key).decrypt(&folder_id)?;
    let trained = learn_message(&session, &config, &imap_client, &http_client, &folder, uid, "learnham").await?;
    Ok(Json(serde_json::json!({ "trained": trained })).into_response())
}

/// Shared implementation: fetch raw bytes then POST to rspamd.
/// Returns true if rspamd was contacted, false if RSPAMD_URL is not set.
async fn learn_message(
    session: &SessionState,
    config: &AppConfig,
    imap_client: &Arc<dyn ImapClient>,
    http_client: &reqwest::Client,
    folder: &str,
    uid: u32,
    endpoint: &str,
) -> Result<bool, AppError> {
    let Some(ref rspamd_url) = config.rspamd_url else {
        return Ok(false);
    };

    let creds = ImapCredentials {
        host: session.imap_host.clone(),
        port: session.imap_port,
        tls: session.imap_tls,
        email: session.email.clone(),
        password: session.password.clone(),
    };

    let raw = imap_client
        .fetch_raw_bytes(&creds, folder, uid)
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("IMAP error: {e}")))?;

    let url = format!("{rspamd_url}/{endpoint}");
    let resp = http_client
        .post(&url)
        .header("Content-Type", "message/rfc822")
        .body(raw)
        .send()
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("rspamd unreachable: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(AppError::ServiceUnavailable(format!(
            "rspamd returned {status}"
        )));
    }

    Ok(true)
}
