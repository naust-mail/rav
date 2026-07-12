use std::sync::Arc;

use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;

use crate::auth::imap_auth::{self, AuthResult};
use crate::auth::session::{ServerEndpoint, SessionState, SessionStore};
use crate::auth::user_data;
use crate::config::AppConfig;
use crate::db;
use crate::mfa::crypto::MfaCrypto;
use crate::mfa::totp;

pub const BROWSER_COOKIE: &str = "rav_browser";

/// JSON body expected on `POST /api/auth/login`.
#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub remember: bool,
    pub imap_host: Option<String>,
    pub imap_port: Option<u16>,
    pub imap_tls: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_tls: Option<bool>,
    pub browser_id: Option<String>,
    /// 6-digit TOTP code. Required when TOTP is enrolled for this account.
    pub totp_code: Option<String>,
}

impl LoginRequest {
    /// Returns true if the request contains any mail server override fields.
    /// Used to reject client-supplied server config when custom servers are disabled.
    fn has_server_config(&self) -> bool {
        self.imap_host.is_some()
            || self.imap_port.is_some()
            || self.imap_tls.is_some()
            || self.smtp_host.is_some()
            || self.smtp_port.is_some()
            || self.smtp_tls.is_some()
    }
}

/// Resolved mail server configuration for a login attempt.
/// Either sourced entirely from app config (locked mode) or merged from
/// request overrides and app config defaults (custom mode).
#[derive(Debug)]
struct EffectiveServerConfig {
    imap_host: String,
    imap_port: u16,
    imap_tls: bool,
    smtp_host: String,
    smtp_port: u16,
    smtp_tls: bool,
}

impl EffectiveServerConfig {
    fn build(request: &LoginRequest, config: &AppConfig) -> Result<Self, (StatusCode, String)> {
        if !config.allow_custom_mail_servers {
            if request.has_server_config() {
                return Err((
                    StatusCode::FORBIDDEN,
                    "Custom mail server configuration is not allowed. \
                     Provide only email and password."
                        .to_string(),
                ));
            }

            let imap_host = config.imap_host.clone().ok_or_else(|| {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Server not configured with default IMAP host".to_string(),
                )
            })?;

            let smtp_host = config
                .smtp_host
                .clone()
                .unwrap_or_else(|| imap_host.clone());

            Ok(Self {
                imap_host,
                imap_port: config.imap_port,
                imap_tls: config.tls_enabled,
                smtp_host,
                smtp_port: config.smtp_port,
                smtp_tls: config.tls_enabled,
            })
        } else {
            let imap_host = request
                .imap_host
                .clone()
                .or_else(|| config.imap_host.clone())
                .ok_or_else(|| {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "IMAP server not configured".to_string(),
                    )
                })?;

            let smtp_host = request
                .smtp_host
                .clone()
                .or_else(|| config.smtp_host.clone())
                .unwrap_or_else(|| imap_host.clone());

            Ok(Self {
                imap_host,
                imap_port: request.imap_port.unwrap_or(config.imap_port),
                imap_tls: request.imap_tls.unwrap_or(config.tls_enabled),
                smtp_host,
                smtp_port: request.smtp_port.unwrap_or(config.smtp_port),
                smtp_tls: request.smtp_tls.unwrap_or(config.tls_enabled),
            })
        }
    }
}

fn browser_cookie(browser_id: &str, max_age_secs: u64, secure: bool) -> String {
    let secure_flag = if secure { " Secure;" } else { "" };
    format!(
        "{}={};{} SameSite=Strict; Path=/; Max-Age={}",
        BROWSER_COOKIE, browser_id, secure_flag, max_age_secs
    )
}

fn account_session_cookie(account_id: &str, token: &str, max_age_secs: u64, secure: bool) -> String {
    let secure_flag = if secure { " Secure;" } else { "" };
    format!(
        "rav_session_{}={};{} HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
        account_id, token, secure_flag, max_age_secs
    )
}

fn clearing_browser_cookie(secure: bool) -> String {
    let secure_flag = if secure { " Secure;" } else { "" };
    format!(
        "{}=;{} HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
        BROWSER_COOKIE, secure_flag
    )
}

fn clearing_account_cookie(account_id: &str, secure: bool) -> String {
    let secure_flag = if secure { " Secure;" } else { "" };
    format!(
        "rav_session_{}=;{} HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
        account_id, secure_flag
    )
}

fn extract_browser_id(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get_all("cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|cookie| {
            for segment in cookie.split(';') {
                let trimmed = segment.trim();
                if let Some(id) = trimmed.strip_prefix("rav_browser=") {
                    let id = id.trim();
                    if !id.is_empty() {
                        return Some(id.to_string());
                    }
                }
            }
            None
        })
}

fn extract_session_cookies(headers: &axum::http::HeaderMap) -> Vec<(String, String)> {
    let prefix = "rav_session_";
    let mut sessions = Vec::new();
    
    for cookie in headers.get_all("cookie").iter().filter_map(|v| v.to_str().ok()) {
        for segment in cookie.split(';') {
            let trimmed = segment.trim();
            if let Some(rest) = trimmed.strip_prefix(prefix)
                && let Some(eq_pos) = rest.find('=')
            {
                let account_id = &rest[..eq_pos];
                let token = &rest[eq_pos + 1..];
                if !account_id.is_empty() && !token.is_empty() {
                    sessions.push((account_id.to_string(), token.to_string()));
                }
            }
        }
    }
    sessions
}

/// `POST /api/auth/login`
///
/// Validates the user's credentials against the configured IMAP server.
/// When TOTP is enrolled for the account, the request must also include
/// `totp_code`; otherwise returns `{"mfa_required": true}` without
/// attempting IMAP authentication (so the response does not confirm
/// whether the password is correct).
///
/// On success, creates a session, provisions the user's data directory,
/// and returns session cookies.
/// Outcome of the pre-IMAP TOTP gate check in `login`.
enum MfaGate {
    /// No MFA to enforce (unenrolled, or the user directory doesn't exist yet) - proceed with the normal flow.
    Skip,
    PasskeyOnly,
    NeedsCode,
    LockedOut,
    SecretMissing,
    Ready { secret: String, code: String },
}

pub async fn login(
    Extension(store): Extension<Arc<SessionStore>>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(transport): Extension<Arc<crate::mail_transport::MailTransport>>,
    Extension(mfa_crypto): Extension<Arc<MfaCrypto>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(body): Json<LoginRequest>,
) -> Response {
    if body.email.trim().is_empty() || body.password.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [("content-type", "application/json")],
            serde_json::json!({
                "error": {
                    "code": "BAD_REQUEST",
                    "message": "Email and password are required",
                    "status": 400
                }
            })
            .to_string(),
        )
            .into_response();
    }

    let user_hash = user_data::hash_email(&body.email);

    // Only check MFA state for accounts whose data directory already exists.
    // For a first-time user the directory is not present yet, so there can be
    // no TOTP or passkey-only settings to enforce. This also prevents creating
    // per-user storage (directory + SQLite + 26 migrations) before credentials
    // are verified, which would be an unauthenticated disk-exhaustion vector.
    let user_dir = std::path::Path::new(&config.data_dir).join(&user_hash);
    // Check TOTP enrollment before touching IMAP so that the "MFA required"
    // response does not confirm password validity.
    let gate = if user_dir.exists() {
        db::pool::with_user_db(&db_pool_manager, &user_hash, {
            let mfa_crypto = mfa_crypto.clone();
            let totp_code = body.totp_code.clone();
            move |conn| {
                // Passkey-only accounts cannot log in with a password.
                if db::mfa::get_mfa_settings(conn)
                    .map(|s| s.passkey_only)
                    .unwrap_or(false)
                {
                    return Ok(MfaGate::PasskeyOnly);
                }

                if !db::mfa::is_totp_enrolled(conn).unwrap_or(false) {
                    return Ok(MfaGate::Skip);
                }

                let Some(code) = totp_code else {
                    return Ok(MfaGate::NeedsCode);
                };

                // Lockout check first to prevent brute-force.
                if totp::is_locked_out(conn).unwrap_or(false) {
                    return Ok(MfaGate::LockedOut);
                }

                match load_totp_secret(conn, &mfa_crypto) {
                    Some(secret) => Ok(MfaGate::Ready { secret, code }),
                    None => Ok(MfaGate::SecretMissing),
                }
            }
        })
        .await
        .unwrap_or(MfaGate::Skip)
    } else {
        MfaGate::Skip
    };

    match gate {
        MfaGate::PasskeyOnly | MfaGate::LockedOut => return unauthorized_response(),
        MfaGate::NeedsCode => {
            // Tell the client to show the TOTP field and re-submit.
            return (
                StatusCode::OK,
                [("content-type", "application/json")],
                serde_json::json!({ "mfa_required": true, "mfa_type": "totp" })
                    .to_string(),
            )
                .into_response();
        }
        MfaGate::SecretMissing => {
            tracing::error!(email = %body.email, "TOTP enrolled but secret missing");
            return internal_error_response();
        }
        MfaGate::Ready { secret, code } => {
            // Validate IMAP credentials.
            let server = match EffectiveServerConfig::build(&body, &config) {
                Ok(s) => s,
                Err((status, msg)) => return server_config_error(status, msg),
            };
            let imap_result = imap_auth::validate_imap_credentials(
                &server.imap_host,
                &transport.imap_connect_host,
                server.imap_port,
                server.imap_tls,
                &body.email,
                &body.password,
                &transport.imap_connector,
            )
            .await;

            if !matches!(imap_result, AuthResult::Success) {
                let _ = db::pool::with_user_db(&db_pool_manager, &user_hash, |conn| {
                    db::mfa::increment_lockout(conn, 5, 900)
                })
                .await;
                return match imap_result {
                    AuthResult::ServerUnreachable(e) => (
                        StatusCode::SERVICE_UNAVAILABLE,
                        [("content-type", "application/json")],
                        serde_json::json!({
                            "error": {
                                "code": "SERVER_UNREACHABLE",
                                "message": e.to_string(),
                                "status": 503
                            }
                        })
                        .to_string(),
                    )
                        .into_response(),
                    _ => unauthorized_response(),
                };
            }

            // IMAP succeeded - now verify TOTP.
            let verify_result = db::pool::with_user_db(&db_pool_manager, &user_hash, move |conn| {
                totp::verify_and_record(conn, &secret, &code)
            })
            .await;

            match verify_result {
                Ok(true) => {
                    // Both factors passed - fall through to session creation below.
                    return create_login_session(
                        body,
                        server,
                        user_hash,
                        &store,
                        &config,
                        &db_pool_manager,
                    )
                    .await;
                }
                Ok(false) => return unauthorized_response(),
                Err(e) => {
                    tracing::error!(error = %e, "TOTP verification error");
                    return internal_error_response();
                }
            }
        }
        MfaGate::Skip => {}
    }

    let server = match EffectiveServerConfig::build(&body, &config) {
        Ok(s) => s,
        Err((status, msg)) => return server_config_error(status, msg),
    };

    let result = imap_auth::validate_imap_credentials(
        &server.imap_host,
        &transport.imap_connect_host,
        server.imap_port,
        server.imap_tls,
        &body.email,
        &body.password,
        &transport.imap_connector,
    )
    .await;

    match result {
        AuthResult::Success => {
            return create_login_session(body, server, user_hash, &store, &config, &db_pool_manager).await;
        }
        AuthResult::InvalidCredentials => return unauthorized_response(),
        AuthResult::ServerUnreachable(e) => return (
            StatusCode::SERVICE_UNAVAILABLE,
            [("content-type", "application/json")],
            serde_json::json!({
                "error": {
                    "code": "SERVER_UNREACHABLE",
                    "message": e.to_string(),
                    "status": 503
                }
            })
            .to_string(),
        )
            .into_response(),
    }

    // Unreachable - all arms above return.
    #[allow(unreachable_code)]
    {
        internal_error_response()
    }
}

/// Decrypts and returns the plaintext TOTP secret for a user, or `None` if unavailable.
fn load_totp_secret(conn: &rusqlite::Connection, mfa_crypto: &MfaCrypto) -> Option<String> {
    let (enc, nonce) = db::mfa::get_totp_secret(conn).ok()??;
    let plaintext = mfa_crypto.decrypt(&enc, &nonce).ok()?;
    String::from_utf8(plaintext).ok()
}

fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [("content-type", "application/json")],
        serde_json::json!({
            "error": {
                "code": "UNAUTHORIZED",
                "message": "Invalid credentials",
                "status": 401
            }
        })
        .to_string(),
    )
        .into_response()
}

fn internal_error_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [("content-type", "application/json")],
        serde_json::json!({
            "error": {
                "code": "INTERNAL_ERROR",
                "message": "An internal error occurred",
                "status": 500
            }
        })
        .to_string(),
    )
        .into_response()
}

fn server_config_error(status: StatusCode, msg: String) -> Response {
    let code = match status {
        StatusCode::FORBIDDEN => "FORBIDDEN",
        StatusCode::SERVICE_UNAVAILABLE => "SERVICE_UNAVAILABLE",
        _ => "ERROR",
    };
    (
        status,
        [("content-type", "application/json")],
        serde_json::json!({
            "error": {
                "code": code,
                "message": msg,
                "status": status.as_u16()
            }
        })
        .to_string(),
    )
        .into_response()
}

/// Creates a session and returns the login response with cookies.
/// Called after all auth factors have been verified. Provisions the user data
/// directory here (not before auth) to avoid pre-auth disk writes.
async fn create_login_session(
    body: LoginRequest,
    server: EffectiveServerConfig,
    user_hash: String,
    store: &Arc<SessionStore>,
    config: &Arc<AppConfig>,
    db_pool_manager: &Arc<db::pool::DbPoolManager>,
) -> Response {
    if let Err(e) = user_data::provision_user_data(&config.data_dir, &user_hash) {
        tracing::error!(error = %e, "failed to provision user data directory");
        return internal_error_response();
    }

    if let Some(ref browser_id) = body.browser_id {
        let existing_accounts = store.get_browser_accounts(browser_id);
        let duplicate = existing_accounts
            .iter()
            .find(|a| a.email == body.email && a.imap_host == server.imap_host);

        if let Some(dup) = duplicate {
            tracing::debug!(
                email = %body.email,
                imap_host = %server.imap_host,
                existing_account_id = %dup.account_id,
                "login: duplicate account detected"
            );
            return (
                StatusCode::CONFLICT,
                [("content-type", "application/json")],
                serde_json::json!({
                    "error": {
                        "code": "DUPLICATE_ACCOUNT",
                        "message": "This account is already logged in",
                        "status": 409
                    }
                })
                .to_string(),
            )
                .into_response();
        }
    }

    let _ = db::pool::with_user_db(db_pool_manager, &user_hash, {
        let email = body.email.clone();
        move |conn| {
            match db::identities::has_identities(conn) {
                Ok(false) => {
                    let default_identity = db::identities::CreateIdentity {
                        email,
                        display_name: String::new(),
                        signature_html: String::new(),
                        is_default: true,
                    };
                    if let Err(e) = db::identities::create_identity(conn, &default_identity) {
                        tracing::warn!(error = %e, "Failed to auto-create default identity");
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to check for existing identities");
                }
                _ => {}
            }
            Ok(())
        }
    })
    .await;

    const REMEMBER_ME_HOURS: u64 = 30 * 24;
    let session_hours = if body.remember {
        REMEMBER_ME_HOURS
    } else {
        config.session_timeout_hours
    };

    let provided_browser_id = body.browser_id.clone();
    let browser_id = body
        .browser_id
        .unwrap_or_else(|| store.create_browser());
    tracing::debug!(
        email = %body.email,
        provided_browser_id = ?provided_browser_id,
        effective_browser_id = %browser_id,
        "login: creating session"
    );

    let (token, account_id) = store.add_account_to_browser(&browser_id, body.email.clone(), body.password, user_hash, ServerEndpoint { host: server.imap_host.clone(), port: server.imap_port, tls: server.imap_tls }, ServerEndpoint { host: server.smtp_host.clone(), port: server.smtp_port, tls: server.smtp_tls });

    let max_age = session_hours * 3600;
    let secure = config.environment != "development";
    let browser_cookie_header = browser_cookie(&browser_id, max_age, secure);
    let session_cookie_header = account_session_cookie(&account_id, &token, max_age, secure);

    let response = (
        StatusCode::CREATED,
        [("content-type", "application/json")],
        serde_json::json!({
            "account": {
                "id": account_id,
                "email": body.email,
                "imapHost": server.imap_host,
                "smtpHost": server.smtp_host
            }
        })
        .to_string(),
    )
        .into_response();

    let mut response = response;
    let headers = response.headers_mut();
    if let Ok(header_value) = browser_cookie_header.parse() {
        headers.append("set-cookie", header_value);
    }
    if let Ok(header_value) = session_cookie_header.parse() {
        headers.append("set-cookie", header_value);
    }

    response
}

/// `GET /api/auth/session`
///
/// Returns the current user's session information. Requires authentication
/// (the `auth_guard` middleware injects `SessionState` into extensions).
pub async fn get_session(
    Extension(session): Extension<SessionState>,
    Extension(outbox_worker_manager): Extension<Arc<crate::realtime::outbox_worker::OutboxWorkerManager>>,
) -> Response {
    // The frontend calls this right after establishing auth, so it's a
    // reliable point to resume any outbox entries left scheduled from
    // before a server restart (credentials are never persisted, so a
    // restarted worker needs a live session to pick them back up).
    outbox_worker_manager
        .ensure_worker(session.user_hash.clone(), session.email.clone(), session.password.clone())
        .notify_one();

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::json!({ "user": { "email": session.email } }).to_string(),
    )
        .into_response()
}

pub async fn list_accounts(
    Extension(store): Extension<Arc<SessionStore>>,
    Extension(config): Extension<Arc<AppConfig>>,
    headers: axum::http::HeaderMap,
) -> Response {
    let secure = config.environment != "development";
    
    let browser_id = extract_browser_id(&headers);

    let Some(browser_id) = browser_id else {
        tracing::debug!("list_accounts: no browser_id cookie found");
        return (
            StatusCode::OK,
            [("content-type", "application/json")],
            serde_json::json!({ "accounts": [], "browserSessionExpired": true }).to_string(),
        )
            .into_response();
    };

    let browser_exists = store.browser_exists(&browser_id);
    if !browser_exists {
        tracing::debug!(browser_id = %browser_id, "list_accounts: browser_id not found on server");
        let clear_browser = clearing_browser_cookie(secure);
        return (
            StatusCode::OK,
            [
                ("content-type", "application/json"),
                ("set-cookie", &clear_browser),
            ],
            serde_json::json!({ "accounts": [], "browserSessionExpired": true }).to_string(),
        )
            .into_response();
    }

    let session_cookies = extract_session_cookies(&headers);
    let session_map: std::collections::HashMap<String, String> = session_cookies.into_iter().collect();
    
    let browser_accounts = store.get_browser_accounts(&browser_id);
    let browser_account_ids: Vec<String> = browser_accounts
        .iter()
        .map(|s| s.account_id.clone())
        .collect();
    
    let mut valid_accounts: Vec<serde_json::Value> = Vec::new();
    let mut clear_cookies: Vec<String> = Vec::new();
    
    for session in &browser_accounts {
        if let Some(token) = session_map.get(&session.account_id) {
            if let Some(valid_session) = store.get(token) {
                valid_accounts.push(serde_json::json!({
                    "id": valid_session.account_id,
                    "email": valid_session.email,
                    "imapHost": valid_session.imap_host,
                    "smtpHost": valid_session.smtp_host
                }));
            } else {
                store.remove_account(&session.account_id);
                clear_cookies.push(clearing_account_cookie(&session.account_id, secure));
            }
        } else {
            store.remove_account(&session.account_id);
        }
    }
    
    for account_id in session_map.keys() {
        if !browser_account_ids.contains(account_id) {
            clear_cookies.push(clearing_account_cookie(account_id, secure));
        }
    }

    if valid_accounts.is_empty() {
        tracing::debug!(
            browser_id = %browser_id,
            "list_accounts: no valid accounts found"
        );
        let clear_browser = clearing_browser_cookie(secure);
        let body = serde_json::json!({ "accounts": [], "browserSessionExpired": true }).to_string();
        
        let mut response = axum::response::Response::new(axum::body::Body::from(body));
        *response.status_mut() = StatusCode::OK;
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
        response.headers_mut().append(
            axum::http::header::SET_COOKIE,
            axum::http::HeaderValue::from_str(&clear_browser).unwrap(),
        );
        for cookie in clear_cookies {
            response.headers_mut().append(
                axum::http::header::SET_COOKIE,
                axum::http::HeaderValue::from_str(&cookie).unwrap(),
            );
        }
        return response;
    }

    tracing::debug!(
        browser_id = %browser_id,
        account_count = valid_accounts.len(),
        accounts = ?valid_accounts.iter().filter_map(|a| a.get("email").and_then(|e| e.as_str())).collect::<Vec<_>>(),
        "list_accounts: returning valid accounts"
    );

    let body = serde_json::json!({ "accounts": valid_accounts }).to_string();
    let mut response = axum::response::Response::new(axum::body::Body::from(body));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    for cookie in clear_cookies {
        response.headers_mut().append(
            axum::http::header::SET_COOKIE,
            axum::http::HeaderValue::from_str(&cookie).unwrap(),
        );
    }
    response
}

pub async fn remove_account(
    Extension(store): Extension<Arc<SessionStore>>,
    Extension(config): Extension<Arc<AppConfig>>,
    Path(account_id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let browser_id = extract_browser_id(&headers);

    let Some(browser_id) = browser_id else {
        return (
            StatusCode::UNAUTHORIZED,
            [("content-type", "application/json")],
            serde_json::json!({
                "error": {
                    "code": "UNAUTHORIZED",
                    "message": "No browser session",
                    "status": 401
                }
            })
            .to_string(),
        )
            .into_response();
    };

    let browser_accounts = store.get_browser_accounts(&browser_id);
    let account_belongs_to_browser = browser_accounts
        .iter()
        .any(|s| s.account_id == account_id);

    if !account_belongs_to_browser {
        return (
            StatusCode::FORBIDDEN,
            [("content-type", "application/json")],
            serde_json::json!({
                "error": {
                    "code": "FORBIDDEN",
                    "message": "Account does not belong to this browser session",
                    "status": 403
                }
            })
            .to_string(),
        )
            .into_response();
    }

    store.remove_account(&account_id);

    let secure = config.environment != "development";
    let cookie = clearing_account_cookie(&account_id, secure);

    (
        StatusCode::OK,
        [
            ("content-type", "application/json"),
            ("set-cookie", &cookie),
        ],
        serde_json::json!({ "status": "logged_out" }).to_string(),
    )
        .into_response()
}

/// `POST /api/auth/logout`
///
/// Removes the current session from the store and clears the session cookie.
/// Requires authentication.
pub async fn logout(
    Extension(store): Extension<Arc<SessionStore>>,
    Extension(config): Extension<Arc<AppConfig>>,
    headers: axum::http::HeaderMap,
) -> Response {
    let browser_id = extract_browser_id(&headers);

    let mut cookies_to_clear: Vec<String> = Vec::new();

    if let Some(ref browser_id) = browser_id {
        let accounts = store.get_browser_accounts(browser_id);
        for account in accounts {
            cookies_to_clear.push(clearing_account_cookie(
                &account.account_id,
                config.environment != "development",
            ));
        }
        store.remove_browser(browser_id);
    }

    let secure = config.environment != "development";
    cookies_to_clear.push(clearing_browser_cookie(secure));

    let response = (
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::json!({ "status": "logged_out" }).to_string(),
    )
        .into_response();

    let mut response = response;
    let headers = response.headers_mut();
    for cookie in cookies_to_clear {
        if let Ok(header_value) = cookie.parse() {
            headers.append("set-cookie", header_value);
        }
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct UserResponse {
        user: UserInfo,
    }

    #[derive(Serialize)]
    struct UserInfo {
        email: String,
    }

    #[derive(Serialize)]
    struct LogoutResponse {
        status: &'static str,
    }

    fn locked_config(imap_host: Option<&str>) -> AppConfig {
        let mut f = figment::Figment::new()
            .merge(("allow_custom_mail_servers", false))
            .merge(("imap_port", 143u16))
            .merge(("smtp_port", 587u16))
            .merge(("tls_enabled", false));
        if let Some(h) = imap_host {
            f = f.merge(("imap_host", h));
        }
        f.extract().unwrap()
    }

    fn custom_config(imap_host: Option<&str>) -> AppConfig {
        let mut f = figment::Figment::new()
            .merge(("allow_custom_mail_servers", true))
            .merge(("imap_port", 993u16))
            .merge(("smtp_port", 587u16))
            .merge(("tls_enabled", true));
        if let Some(h) = imap_host {
            f = f.merge(("imap_host", h));
        }
        f.extract().unwrap()
    }

    fn login_request(overrides: Option<(&str, u16, bool)>) -> LoginRequest {
        LoginRequest {
            email: "user@example.com".to_string(),
            password: crate::test_support::FAKE_PASSWORD.to_string(),
            remember: false,
            imap_host: overrides.map(|(h, _, _)| h.to_string()),
            imap_port: overrides.map(|(_, p, _)| p),
            imap_tls: overrides.map(|(_, _, t)| t),
            smtp_host: overrides.map(|(h, _, _)| h.to_string()),
            smtp_port: overrides.map(|(_, p, _)| p),
            smtp_tls: overrides.map(|(_, _, t)| t),
            browser_id: None,
            totp_code: None,
        }
    }

    #[test]
    fn locked_mode_rejects_override_fields() {
        let config = locked_config(Some("127.0.0.1"));
        let req = login_request(Some(("evil.com", 993, true)));
        let err = EffectiveServerConfig::build(&req, &config).unwrap_err();
        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn locked_mode_uses_server_config() {
        let config = locked_config(Some("127.0.0.1"));
        let req = login_request(None);
        let server = EffectiveServerConfig::build(&req, &config).unwrap();
        assert_eq!(server.imap_host, "127.0.0.1");
        assert_eq!(server.imap_port, 143);
        assert!(!server.imap_tls);
        assert_eq!(server.smtp_host, "127.0.0.1");
        assert_eq!(server.smtp_port, 587);
        assert!(!server.smtp_tls);
    }

    #[test]
    fn locked_mode_missing_imap_host_returns_503() {
        let config = locked_config(None);
        let req = login_request(None);
        let err = EffectiveServerConfig::build(&req, &config).unwrap_err();
        assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn custom_mode_request_override_wins() {
        let config = custom_config(Some("default.example.com"));
        let req = login_request(Some(("override.example.com", 143, false)));
        let server = EffectiveServerConfig::build(&req, &config).unwrap();
        assert_eq!(server.imap_host, "override.example.com");
        assert_eq!(server.imap_port, 143);
        assert!(!server.imap_tls);
    }

    #[test]
    fn custom_mode_falls_back_to_config() {
        let config = custom_config(Some("default.example.com"));
        let req = login_request(None);
        let server = EffectiveServerConfig::build(&req, &config).unwrap();
        assert_eq!(server.imap_host, "default.example.com");
        assert_eq!(server.imap_port, 993);
        assert!(server.imap_tls);
    }

    #[test]
    fn smtp_host_falls_back_to_imap_host() {
        // When no smtp_host is set anywhere, it should mirror imap_host.
        let config: AppConfig = figment::Figment::new()
            .merge(("allow_custom_mail_servers", false))
            .merge(("imap_host", "mail.example.com"))
            // smtp_host intentionally absent
            .extract()
            .unwrap();
        let req = login_request(None);
        let server = EffectiveServerConfig::build(&req, &config).unwrap();
        assert_eq!(server.smtp_host, "mail.example.com");
    }

    #[test]
    fn user_response_serialization() {
        let resp = UserResponse {
            user: UserInfo {
                email: "test@example.com".to_string(),
            },
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["user"]["email"], "test@example.com");
    }

    #[test]
    fn logout_response_serialization() {
        let resp = LogoutResponse {
            status: "logged_out",
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "logged_out");
    }
}
