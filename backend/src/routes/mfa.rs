use std::sync::Arc;

use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;

use crate::auth::imap_auth::{self, AuthResult};
use crate::auth::session::{ServerEndpoint, SessionState, SessionStore};
use crate::auth::user_data;
use crate::config::AppConfig;
use crate::db;
use crate::mail_transport::MailTransport;
use crate::mfa::crypto::MfaCrypto;
use crate::mfa::passkey::{self, PasskeyService};
use crate::mfa::totp;

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// `GET /api/mfa/status`
pub async fn status(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
) -> Response {
    let result = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        let totp_enabled = db::mfa::is_totp_enrolled(conn).unwrap_or(false);
        let passkey_count = db::mfa::list_passkeys_info(conn).map(|v| v.len()).unwrap_or(0);
        let passkey_only = db::mfa::get_mfa_settings(conn)
            .map(|s| s.passkey_only)
            .unwrap_or(false);
        Ok((totp_enabled, passkey_count, passkey_only))
    })
    .await;

    let (totp_enabled, passkey_count, passkey_only) = match result {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "mfa/status: failed to open user DB");
            return internal_error();
        }
    };

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::json!({
            "totp_enabled": totp_enabled,
            "passkey_count": passkey_count,
            "passkey_only": passkey_only,
        })
        .to_string(),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// TOTP
// ---------------------------------------------------------------------------

/// `POST /api/mfa/totp/setup`
pub async fn totp_setup(Extension(session): Extension<SessionState>) -> Response {
    let secret = match totp::generate_secret() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "totp/setup: secret generation failed");
            return internal_error();
        }
    };

    let url = match totp::get_url(&secret, &session.email, "Rav Mail") {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(error = %e, "totp/setup: URL generation failed");
            return internal_error();
        }
    };

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::json!({ "secret": secret, "url": url }).to_string(),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct TotpConfirmRequest {
    pub secret: String,
    pub code: String,
}

/// `POST /api/mfa/totp/confirm`
pub async fn totp_confirm(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Extension(mfa_crypto): Extension<Arc<MfaCrypto>>,
    Json(body): Json<TotpConfirmRequest>,
) -> Response {
    if body.secret.trim().is_empty() || body.code.trim().is_empty() {
        return bad_request("Secret and code are required");
    }

    enum ConfirmOutcome {
        LockedOut,
        Invalid,
        Ok,
    }

    let secret = body.secret.clone();
    let code = body.code.clone();
    let result = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        if totp::is_locked_out(conn).unwrap_or(false) {
            return Ok(ConfirmOutcome::LockedOut);
        }

        let valid = totp::verify_and_record(conn, &secret, &code)?;
        if !valid {
            return Ok(ConfirmOutcome::Invalid);
        }

        let (encrypted, nonce) = mfa_crypto
            .encrypt(secret.as_bytes())
            .map_err(|e| e.to_string())?;
        db::mfa::upsert_totp_secret(conn, &encrypted, &nonce)?;
        Ok(ConfirmOutcome::Ok)
    })
    .await;

    match result {
        Ok(ConfirmOutcome::LockedOut) => return unauthorized_response(),
        Ok(ConfirmOutcome::Invalid) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                [("content-type", "application/json")],
                serde_json::json!({
                    "error": {
                        "code": "INVALID_CODE",
                        "message": "The code is incorrect. Check your authenticator app and try again.",
                        "status": 422
                    }
                })
                .to_string(),
            )
                .into_response();
        }
        Ok(ConfirmOutcome::Ok) => {}
        Err(e) => {
            tracing::error!(error = %e, "totp/confirm: failed");
            return internal_error();
        }
    }

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::json!({ "totp_enabled": true }).to_string(),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct TotpDeleteRequest {
    pub code: String,
}

/// `DELETE /api/mfa/totp`
///
/// Requires the current TOTP code to confirm the user still controls the authenticator.
pub async fn totp_delete(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Extension(mfa_crypto): Extension<Arc<MfaCrypto>>,
    Json(body): Json<TotpDeleteRequest>,
) -> Response {
    if body.code.trim().is_empty() {
        return bad_request("A verification code is required to remove the authenticator app.");
    }

    enum DeleteOutcome {
        NotEnrolled,
        NotUtf8,
        LockedOut,
        Invalid,
        Ok,
    }

    let code = body.code.clone();
    let result = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        let (encrypted, nonce) = match db::mfa::get_totp_secret(conn)? {
            Some(pair) => pair,
            None => return Ok(DeleteOutcome::NotEnrolled),
        };

        let secret_bytes = mfa_crypto
            .decrypt(&encrypted, &nonce)
            .map_err(|e| e.to_string())?;
        let secret_b32 = match String::from_utf8(secret_bytes) {
            Ok(s) => s,
            Err(_) => return Ok(DeleteOutcome::NotUtf8),
        };

        if totp::is_locked_out(conn).unwrap_or(false) {
            return Ok(DeleteOutcome::LockedOut);
        }

        let valid = totp::verify_and_record(conn, &secret_b32, &code)?;
        if !valid {
            return Ok(DeleteOutcome::Invalid);
        }

        db::mfa::delete_totp(conn)?;
        Ok(DeleteOutcome::Ok)
    })
    .await;

    match result {
        Ok(DeleteOutcome::NotEnrolled) => return bad_request("No authenticator app is enrolled."),
        Ok(DeleteOutcome::NotUtf8) => {
            tracing::error!("totp/delete: stored secret is not valid UTF-8");
            return internal_error();
        }
        Ok(DeleteOutcome::LockedOut) => return unauthorized_response(),
        Ok(DeleteOutcome::Invalid) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                [("content-type", "application/json")],
                serde_json::json!({
                    "error": {
                        "code": "INVALID_CODE",
                        "message": "The code is incorrect. Check your authenticator app and try again.",
                        "status": 422
                    }
                })
                .to_string(),
            )
                .into_response();
        }
        Ok(DeleteOutcome::Ok) => {}
        Err(e) => {
            tracing::error!(error = %e, "totp/delete: failed");
            return internal_error();
        }
    }

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::json!({ "totp_enabled": false }).to_string(),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Passkey registration
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PasskeyRegisterBeginRequest {
    #[serde(default)]
    pub name: String,
}

/// `POST /api/mfa/passkey/register/begin`
pub async fn passkey_register_begin(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Extension(pk_svc): Extension<Arc<PasskeyService>>,
    Json(body): Json<PasskeyRegisterBeginRequest>,
) -> Response {
    if pk_svc.webauthn.is_none() {
        return service_unavailable("Passkeys are not configured on this server");
    }

    // Collect existing credential IDs so the authenticator can exclude them.
    let existing_ids_result = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        Ok(db::mfa::list_passkeys_info(conn)
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.credential_id)
            .collect::<Vec<String>>())
    })
    .await;
    let existing_ids: Vec<String> = match existing_ids_result {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "passkey/register/begin: failed to open user DB");
            return internal_error();
        }
    };

    let user_id = uuid::Uuid::new_v4();
    let key_name = if body.name.trim().is_empty() {
        "Passkey".to_string()
    } else {
        body.name.trim().to_string()
    };

    match pk_svc.begin_registration(user_id, &session.email, key_name, existing_ids) {
        Ok((nonce, options)) => (
            StatusCode::OK,
            [("content-type", "application/json")],
            serde_json::json!({ "nonce": nonce, "options": options }).to_string(),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "passkey/register/begin: failed");
            internal_error()
        }
    }
}

#[derive(Deserialize)]
pub struct PasskeyRegisterCompleteRequest {
    pub nonce: String,
    pub credential: serde_json::Value,
}

/// `POST /api/mfa/passkey/register/complete`
pub async fn passkey_register_complete(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Extension(pk_svc): Extension<Arc<PasskeyService>>,
    Json(body): Json<PasskeyRegisterCompleteRequest>,
) -> Response {
    if pk_svc.webauthn.is_none() {
        return service_unavailable("Passkeys are not configured on this server");
    }

    // Extract PRF output before handing credential to webauthn-rs.
    let prf_output = match passkey::extract_prf_output(&body.credential) {
        Some(b) => b,
        None => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                [("content-type", "application/json")],
                serde_json::json!({
                    "error": {
                        "code": "PRF_NOT_SUPPORTED",
                        "message": "Your authenticator or browser does not support the PRF extension. \
                                    Use Chrome with a platform authenticator (Touch ID, Face ID, Windows Hello) \
                                    or a compatible hardware key.",
                        "status": 422
                    }
                })
                .to_string(),
            )
                .into_response();
        }
    };

    let (new_passkey, prf_salt, key_name) =
        match pk_svc.finish_registration(&body.nonce, body.credential) {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = %e, "passkey/register/complete: finish failed");
                return bad_request(&format!("Registration failed: {e}"));
            }
        };

    // Encrypt the user's IMAP password with the PRF output.
    let (encrypted_imap, imap_nonce) =
        match passkey::encrypt_with_prf(&prf_output, session.password.as_bytes()) {
            Ok(pair) => pair,
            Err(e) => {
                tracing::error!(error = %e, "passkey/register/complete: encrypt failed");
                return internal_error();
            }
        };

    let cred_id = passkey::passkey_cred_id(&new_passkey);
    let passkey_json = match passkey::serialize_passkey(&new_passkey) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!(error = %e, "passkey/register/complete: serialize failed");
            return internal_error();
        }
    };

    let result = db::pool::with_user_db(&db_pool_manager, &session.user_hash, {
        let cred_id = cred_id.clone();
        let passkey_json = passkey_json.clone();
        let prf_salt = prf_salt.clone();
        let key_name = key_name.clone();
        let imap_host = session.imap_host.clone();
        let imap_port = session.imap_port;
        let imap_tls = session.imap_tls;
        let smtp_host = session.smtp_host.clone();
        let smtp_port = session.smtp_port;
        let smtp_tls = session.smtp_tls;
        move |conn| {
            db::mfa::insert_passkey(
                conn,
                db::mfa::NewPasskey {
                    credential_id: &cred_id,
                    passkey_json: &passkey_json,
                    prf_salt: &prf_salt,
                    encrypted_imap: &encrypted_imap,
                    imap_nonce: &imap_nonce,
                    name: &key_name,
                },
                ServerEndpoint { host: imap_host, port: imap_port, tls: imap_tls },
                ServerEndpoint { host: smtp_host, port: smtp_port, tls: smtp_tls },
            )
        }
    })
    .await;

    match result {
        Ok(()) => (
            StatusCode::CREATED,
            [("content-type", "application/json")],
            serde_json::json!({
                "id": cred_id,
                "name": key_name,
            })
            .to_string(),
        )
            .into_response(),
        Err(e) if e.contains("UNIQUE constraint") => (
            StatusCode::CONFLICT,
            [("content-type", "application/json")],
            serde_json::json!({
                "error": {
                    "code": "ALREADY_ENROLLED",
                    "message": "This passkey is already enrolled.",
                    "status": 409
                }
            })
            .to_string(),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "passkey/register/complete: DB insert failed");
            internal_error()
        }
    }
}

// ---------------------------------------------------------------------------
// Passkey authentication (public routes - no auth_guard)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PasskeyLoginBeginRequest {
    pub email: String,
}

/// `POST /api/mfa/passkey/login/begin`
pub async fn passkey_login_begin(
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Extension(pk_svc): Extension<Arc<PasskeyService>>,
    Json(body): Json<PasskeyLoginBeginRequest>,
) -> Response {
    if pk_svc.webauthn.is_none() {
        return service_unavailable("Passkeys are not configured on this server");
    }

    let user_hash = user_data::hash_email(&body.email);
    let rp_id = config.webauthn_rp_id.as_deref().unwrap_or("localhost");

    // If the user directory does not exist the account has never been provisioned.
    // Return a fake challenge without touching the filesystem - this prevents both
    // a 500/200 existence oracle and DB/directory creation for arbitrary emails.
    let user_dir = std::path::Path::new(&config.data_dir).join(&user_hash);
    if !user_dir.exists() {
        let (nonce, options) = pk_svc.begin_authentication_fake(body.email, rp_id);
        return (
            StatusCode::OK,
            [("content-type", "application/json")],
            serde_json::json!({ "nonce": nonce, "options": options }).to_string(),
        )
            .into_response();
    }

    let rows = match db::pool::with_user_db(&db_pool_manager, &user_hash, |conn| {
        db::mfa::list_passkeys_full(conn)
    })
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "passkey/login/begin: DB error");
            let (nonce, options) = pk_svc.begin_authentication_fake(body.email, rp_id);
            return (
                StatusCode::OK,
                [("content-type", "application/json")],
                serde_json::json!({ "nonce": nonce, "options": options }).to_string(),
            )
                .into_response();
        }
    };

    if rows.is_empty() {
        // Anti-enumeration: return a fake challenge identical in shape to a real one.
        // The stored state has no passkey data, so complete always rejects it with 401.
        let (nonce, options) = pk_svc.begin_authentication_fake(body.email, rp_id);
        return (
            StatusCode::OK,
            [("content-type", "application/json")],
            serde_json::json!({ "nonce": nonce, "options": options }).to_string(),
        )
            .into_response();
    }

    let mut passkeys = Vec::with_capacity(rows.len());
    let mut cred_salts = std::collections::HashMap::new();

    for row in &rows {
        match passkey::deserialize_passkey(&row.passkey_json) {
            Ok(pk) => {
                cred_salts.insert(row.credential_id.clone(), row.prf_salt.clone());
                passkeys.push(pk);
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    credential_id = %row.credential_id,
                    "passkey/login/begin: failed to deserialize passkey, skipping"
                );
            }
        }
    }

    if passkeys.is_empty() {
        return internal_error();
    }

    match pk_svc.begin_authentication(body.email, &passkeys, cred_salts) {
        Ok((nonce, options)) => (
            StatusCode::OK,
            [("content-type", "application/json")],
            serde_json::json!({ "nonce": nonce, "options": options }).to_string(),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "passkey/login/begin: ceremony error");
            internal_error()
        }
    }
}

#[derive(Deserialize)]
pub struct PasskeyLoginCompleteRequest {
    pub nonce: String,
    pub credential: serde_json::Value,
    #[serde(default)]
    pub remember: bool,
    pub browser_id: Option<String>,
}

/// `POST /api/mfa/passkey/login/complete`
pub async fn passkey_login_complete(
    Extension(store): Extension<Arc<SessionStore>>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Extension(transport): Extension<Arc<MailTransport>>,
    Extension(pk_svc): Extension<Arc<PasskeyService>>,
    Json(body): Json<PasskeyLoginCompleteRequest>,
) -> Response {
    if pk_svc.webauthn.is_none() {
        return service_unavailable("Passkeys are not configured on this server");
    }

    let prf_output = match passkey::extract_prf_output(&body.credential) {
        Some(b) => b,
        None => {
            return unauthorized_response();
        }
    };

    let (auth_result, _prf_salt, email) =
        match pk_svc.finish_authentication(&body.nonce, body.credential.clone()) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "passkey/login/complete: authentication failed");
                return unauthorized_response();
            }
        };

    let user_hash = user_data::hash_email(&email);
    let cred_id = URL_SAFE_NO_PAD.encode(auth_result.cred_id().as_ref());

    let row_result = db::pool::with_user_db(&db_pool_manager, &user_hash, {
        let cred_id = cred_id.clone();
        move |conn| db::mfa::get_passkey(conn, &cred_id)
    })
    .await;

    let row = match row_result {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::error!(cred_id = %cred_id, "passkey/login/complete: credential not found in DB");
            return unauthorized_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "passkey/login/complete: DB error");
            return internal_error();
        }
    };

    // Decrypt IMAP password using PRF output.
    let imap_password = match passkey::decrypt_with_prf(&prf_output, &row.encrypted_imap, &row.imap_nonce) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => {
                tracing::error!("passkey/login/complete: decrypted IMAP password is not UTF-8");
                return internal_error();
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, "passkey/login/complete: PRF decrypt failed");
            return unauthorized_response();
        }
    };

    // Validate IMAP credentials with the decrypted password.
    let imap_result = imap_auth::validate_imap_credentials(
        &row.imap_host,
        &transport.imap_connect_host,
        row.imap_port,
        row.imap_tls,
        &email,
        &imap_password,
        &transport.imap_connector,
    )
    .await;

    if !matches!(imap_result, AuthResult::Success) {
        tracing::warn!(email = %email, "passkey/login/complete: IMAP auth failed with decrypted credential - password may have changed");
        // The PRF decryption succeeded but IMAP rejected the result, which means
        // the mail password changed after this passkey was enrolled. Return a
        // distinct error so the client can prompt re-enrollment rather than
        // showing a generic "wrong password" message.
        return (
            StatusCode::UNAUTHORIZED,
            [("content-type", "application/json")],
            serde_json::json!({
                "error": {
                    "code": "PASSKEY_CREDENTIAL_STALE",
                    "message": "Your passkey is linked to an old password. Sign in with your password and re-enroll your passkey.",
                    "status": 401
                }
            })
            .to_string(),
        )
            .into_response();
    }

    // Update sign count - deserialize passkey, update, re-serialize and store.
    if let Ok(mut stored_pk) = passkey::deserialize_passkey(&row.passkey_json) {
        stored_pk.update_credential(&auth_result);
        if let Ok(updated_json) = passkey::serialize_passkey(&stored_pk) {
            let _ = db::pool::with_user_db(&db_pool_manager, &user_hash, {
                let cred_id = cred_id.clone();
                move |conn| db::mfa::update_passkey_json(conn, &cred_id, &updated_json)
            })
            .await;
        }
    }

    // Provision user data and create session (mirrors create_login_session in auth.rs).
    if let Err(e) = user_data::provision_user_data(&config.data_dir, &user_hash) {
        tracing::error!(error = %e, "passkey/login/complete: failed to provision user data");
        return internal_error();
    }

    let _ = db::pool::with_user_db(&db_pool_manager, &user_hash, {
        let email = email.clone();
        move |conn| {
            if let Ok(false) = db::identities::has_identities(conn) {
                let default_identity = db::identities::CreateIdentity {
                    email: email.clone(),
                    display_name: String::new(),
                    signature_html: String::new(),
                    is_default: true,
                };
                let _ = db::identities::create_identity(conn, &default_identity);
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

    let browser_id = body
        .browser_id
        .clone()
        .unwrap_or_else(|| store.create_browser());

    let (token, account_id) = store.add_account_to_browser(&browser_id, email.clone(), imap_password, user_hash, ServerEndpoint { host: row.imap_host.clone(), port: row.imap_port, tls: row.imap_tls }, ServerEndpoint { host: row.smtp_host.clone(), port: row.smtp_port, tls: row.smtp_tls });

    let max_age = session_hours * 3600;
    let secure = config.environment != "development";

    let browser_cookie = format!(
        "rav_browser={};{} SameSite=Strict; Path=/; Max-Age={}",
        browser_id,
        if secure { " Secure;" } else { "" },
        max_age,
    );
    let session_cookie = format!(
        "rav_session_{}={};{} HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
        account_id,
        token,
        if secure { " Secure;" } else { "" },
        max_age,
    );

    let body_json = serde_json::json!({
        "account": {
            "id": account_id,
            "email": email,
            "imapHost": row.imap_host,
            "smtpHost": row.smtp_host,
        }
    })
    .to_string();

    let mut response = (
        StatusCode::CREATED,
        [("content-type", "application/json")],
        body_json,
    )
        .into_response();

    let headers = response.headers_mut();
    if let Ok(v) = browser_cookie.parse() {
        headers.append("set-cookie", v);
    }
    if let Ok(v) = session_cookie.parse() {
        headers.append("set-cookie", v);
    }
    response
}

// ---------------------------------------------------------------------------
// Passkey management (authenticated routes)
// ---------------------------------------------------------------------------

/// `GET /api/mfa/passkeys`
pub async fn passkey_list(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
) -> Response {
    let infos = match db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        db::mfa::list_passkeys_info(conn)
    })
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "passkey/list: DB error");
            return internal_error();
        }
    };

    let items: Vec<serde_json::Value> = infos
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "id": r.credential_id,
                "name": r.name,
                "created_at": r.created_at,
            })
        })
        .collect();

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::json!({ "passkeys": items }).to_string(),
    )
        .into_response()
}

/// `DELETE /api/mfa/passkeys/{id}`
pub async fn passkey_delete(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(cred_id): Path<String>,
) -> Response {
    enum DeleteOutcome {
        LastPasskey,
        Deleted(bool),
    }

    let result = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        // Refuse to delete the last passkey while passkey-only mode is enabled -
        // that would fully lock the account out with no recovery path except admin
        // intervention.
        if db::mfa::get_mfa_settings(conn)
            .map(|s| s.passkey_only)
            .unwrap_or(false)
        {
            let count = db::mfa::list_passkeys_info(conn).map(|v| v.len()).unwrap_or(0);
            if count <= 1 {
                return Ok(DeleteOutcome::LastPasskey);
            }
        }

        let deleted = db::mfa::delete_passkey(conn, &cred_id)?;
        Ok(DeleteOutcome::Deleted(deleted))
    })
    .await;

    match result {
        Ok(DeleteOutcome::LastPasskey) => bad_request(
            "Cannot remove the last passkey while passkey-only mode is enabled. \
             Disable passkey-only mode first.",
        ),
        Ok(DeleteOutcome::Deleted(true)) => (
            StatusCode::OK,
            [("content-type", "application/json")],
            serde_json::json!({ "deleted": true }).to_string(),
        )
            .into_response(),
        Ok(DeleteOutcome::Deleted(false)) => (
            StatusCode::NOT_FOUND,
            [("content-type", "application/json")],
            serde_json::json!({
                "error": {
                    "code": "NOT_FOUND",
                    "message": "Passkey not found.",
                    "status": 404
                }
            })
            .to_string(),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "passkey/delete: DB error");
            internal_error()
        }
    }
}

#[derive(Deserialize)]
pub struct PasskeyOnlyRequest {
    pub enabled: bool,
}

/// `PUT /api/mfa/settings/passkey-only`
pub async fn passkey_only_set(
    Extension(session): Extension<SessionState>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(body): Json<PasskeyOnlyRequest>,
) -> Response {
    enum SetOutcome {
        NoPasskeys,
        Ok,
    }

    let enabled = body.enabled;
    let result = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        // Disallow enabling passkey-only when no passkeys are enrolled.
        if enabled && !db::mfa::has_passkeys(conn)? {
            return Ok(SetOutcome::NoPasskeys);
        }

        db::mfa::set_passkey_only(conn, enabled)?;
        Ok(SetOutcome::Ok)
    })
    .await;

    match result {
        Ok(SetOutcome::NoPasskeys) => {
            return bad_request("Cannot enable passkey-only mode without at least one enrolled passkey.");
        }
        Ok(SetOutcome::Ok) => {}
        Err(e) => {
            tracing::error!(error = %e, "passkey-only/set: DB error");
            return internal_error();
        }
    }

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::json!({ "passkey_only": body.enabled }).to_string(),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn internal_error() -> Response {
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

fn bad_request(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [("content-type", "application/json")],
        serde_json::json!({
            "error": {
                "code": "BAD_REQUEST",
                "message": msg,
                "status": 400
            }
        })
        .to_string(),
    )
        .into_response()
}

fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [("content-type", "application/json")],
        serde_json::json!({
            "error": {
                "code": "UNAUTHORIZED",
                "message": "Authentication failed",
                "status": 401
            }
        })
        .to_string(),
    )
        .into_response()
}

fn service_unavailable(msg: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [("content-type", "application/json")],
        serde_json::json!({
            "error": {
                "code": "SERVICE_UNAVAILABLE",
                "message": msg,
                "status": 503
            }
        })
        .to_string(),
    )
        .into_response()
}
