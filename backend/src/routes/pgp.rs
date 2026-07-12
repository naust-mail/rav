use std::net::IpAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::extract::{Path, Query};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde::{Deserialize, Serialize};

use crate::auth::session::SessionState;
use crate::config::AppConfig;
use crate::db;
use crate::error::AppError;

// ---------------------------------------------------------------------------
// WKD-specific HTTP client: no redirects, short timeout, private-IP filter
// ---------------------------------------------------------------------------

/// Custom DNS resolver that rejects any name resolving to a non-public IP,
/// closing the DNS-rebinding TOCTOU gap. The check runs inside reqwest's own
/// connection setup - the same lookup drives both validation and the socket
/// connect, so no second independent lookup can be substituted.
struct PrivateFilterResolver;

impl Resolve for PrivateFilterResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let hostname = name.as_str().to_owned();
        Box::pin(async move {
            let addrs = tokio::net::lookup_host(format!("{hostname}:0")).await?;
            let mut out: Vec<std::net::SocketAddr> = Vec::new();
            for addr in addrs {
                if is_non_public_ip(addr.ip()) {
                    return Err(format!(
                        "WKD domain resolves to non-public IP: {}",
                        addr.ip()
                    )
                    .into());
                }
                out.push(addr);
            }
            if out.is_empty() {
                return Err("Domain does not resolve".into());
            }
            let addrs: Addrs = Box::new(out.into_iter());
            Ok(addrs)
        })
    }
}

static WKD_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn wkd_client() -> &'static reqwest::Client {
    WKD_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .dns_resolver(Arc::new(PrivateFilterResolver))
            .build()
            .expect("WKD HTTP client should build")
    })
}

/// Returns true for IPs that must never be the target of an outbound WKD request.
fn is_non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            // IPv4-mapped addresses like ::ffff:127.0.0.1 are handled in the V6 arm.
            v4.is_loopback() || v4.is_private() || v4.is_link_local()
                || v4.is_unspecified() || v4.is_broadcast()
        }
        IpAddr::V6(v6) => {
            // Unwrap IPv4-mapped (::ffff:x.x.x.x) and classify as IPv4.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_loopback() || v4.is_private() || v4.is_link_local()
                    || v4.is_unspecified() || v4.is_broadcast();
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00  // ULA fc00::/7
                || (v6.segments()[0] & 0xffc0) == 0xfe80  // link-local fe80::/10
        }
    }
}

/// Validate the format of a WKD target domain before making any request.
/// IP literals bypass the custom DNS resolver entirely, so they must be
/// rejected here. The resolver handles all hostname-based SSRF checks.
fn validate_wkd_domain(domain: &str) -> Result<(), AppError> {
    if domain.contains('@') || domain.starts_with('[') {
        return Err(AppError::BadRequest("Invalid email domain".to_string()));
    }
    if domain.parse::<IpAddr>().is_ok() {
        return Err(AppError::BadRequest("Invalid email domain".to_string()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct StoreKeyRequest {
    pub id: String,
    pub fingerprint: String,
    pub public_key: String,
    pub private_key_enc: String,
    pub identity_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AssignIdentityRequest {
    pub identity_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct WkdQuery {
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct WkdResponse {
    pub found: bool,
    pub public_key: Option<String>,
}

// ---------------------------------------------------------------------------
// Z-base-32 encoding for WKD
// ---------------------------------------------------------------------------

const ZBASE32_ALPHABET: &[u8] = b"ybndrfg8ejkmcpqxot1uwisza345h769";

fn zbase32_encode(data: &[u8]) -> String {
    let mut result = String::new();
    let mut buffer: u32 = 0;
    let mut bits_left: u8 = 0;

    for &byte in data {
        buffer = (buffer << 8) | u32::from(byte);
        bits_left += 8;
        while bits_left >= 5 {
            bits_left -= 5;
            let idx = ((buffer >> bits_left) & 0x1f) as usize;
            result.push(ZBASE32_ALPHABET[idx] as char);
        }
    }

    if bits_left > 0 {
        let idx = ((buffer << (5 - bits_left)) & 0x1f) as usize;
        result.push(ZBASE32_ALPHABET[idx] as char);
    }

    result
}

/// Compute the WKD SHA-1 + Z-base-32 hash of a local-part string.
fn wkd_hash(local: &str) -> String {
    use sha1::{Sha1, Digest};
    let mut hasher = Sha1::new();
    hasher.update(local.to_lowercase().as_bytes());
    let hash = hasher.finalize();
    zbase32_encode(&hash)
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

fn require_pgp(config: &AppConfig) -> Result<(), AppError> {
    if !config.pgp_enabled {
        Err(AppError::NotFound("PGP is not enabled on this server".to_string()))
    } else {
        Ok(())
    }
}

/// `GET /pgp/keys` — list all stored key summaries.
pub async fn list_keys(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
) -> Result<Response, AppError> {
    require_pgp(&config)?;
    let keys = db::pool::with_user_db(&db_pool_manager, &session.user_hash, |conn| {
        db::pgp::list_keys(conn)
    })
    .await
    .map_err(AppError::InternalError)?;
    Ok(Json(serde_json::json!({ "keys": keys })).into_response())
}

/// `POST /pgp/keys` — store a new key.
pub async fn store_key(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Json(req): Json<StoreKeyRequest>,
) -> Result<Response, AppError> {
    require_pgp(&config)?;
    if req.id.trim().is_empty() || req.fingerprint.trim().is_empty() {
        return Err(AppError::BadRequest("id and fingerprint are required".to_string()));
    }

    let key = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::pgp::upsert_key(
            conn,
            &req.id,
            req.identity_id,
            &req.fingerprint,
            &req.public_key,
            &req.private_key_enc,
        )?;

        db::pgp::get_key(conn, &req.id)?
            .ok_or_else(|| "Failed to read back stored key".to_string())
    })
    .await
    .map_err(AppError::InternalError)?;

    Ok((axum::http::StatusCode::CREATED, Json(key)).into_response())
}

/// `GET /pgp/keys/:id` — get full key record including private key.
pub async fn get_key(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    require_pgp(&config)?;
    let key = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::pgp::get_key(conn, &id)
    })
    .await
    .map_err(AppError::InternalError)?;

    match key {
        Some(key) => Ok(Json(key).into_response()),
        None => Err(AppError::NotFound("PGP key not found".to_string())),
    }
}

/// `DELETE /pgp/keys/:id` — delete a key.
pub async fn delete_key(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    require_pgp(&config)?;
    let deleted = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::pgp::delete_key(conn, &id)
    })
    .await
    .map_err(AppError::InternalError)?;

    if deleted {
        Ok(Json(serde_json::json!({ "status": "deleted" })).into_response())
    } else {
        Err(AppError::NotFound("PGP key not found".to_string()))
    }
}

/// `PUT /pgp/keys/:id/identity` — assign or unassign an identity to a key.
pub async fn assign_identity(
    Extension(session): Extension<SessionState>,
    Extension(config): Extension<Arc<AppConfig>>,
    Extension(db_pool_manager): Extension<Arc<db::pool::DbPoolManager>>,
    Path(id): Path<String>,
    Json(req): Json<AssignIdentityRequest>,
) -> Result<Response, AppError> {
    require_pgp(&config)?;
    let updated = db::pool::with_user_db(&db_pool_manager, &session.user_hash, move |conn| {
        db::pgp::assign_identity(conn, &id, req.identity_id)
    })
    .await
    .map_err(AppError::InternalError)?;

    if updated {
        Ok(Json(serde_json::json!({ "status": "updated" })).into_response())
    } else {
        Err(AppError::NotFound("PGP key not found".to_string()))
    }
}

/// `GET /pgp/wkd?email=foo@example.com` — proxy WKD key discovery.
///
/// Attempts the advanced WKD method first, then falls back to direct.
/// Returns the armored key text or base64-encoded binary key data.
pub async fn wkd_lookup(
    Extension(config): Extension<Arc<AppConfig>>,
    Query(params): Query<WkdQuery>,
) -> Result<Response, AppError> {
    require_pgp(&config)?;
    let email = params.email.trim().to_lowercase();

    let (local, domain) = email
        .split_once('@')
        .ok_or_else(|| AppError::BadRequest("Invalid email address".to_string()))?;

    if local.is_empty() || domain.is_empty() {
        return Err(AppError::BadRequest("Invalid email address".to_string()));
    }

    validate_wkd_domain(domain)?;

    let hash = wkd_hash(local);
    let client = wkd_client();

    // A PGP public key is a few KB; cap at 64 KB to prevent memory exhaustion
    // from an attacker-controlled WKD server streaming an unbounded response.
    const MAX_BYTES: usize = 64 * 1024;

    // Advanced WKD method: openpgpkey subdomain.
    let advanced_url = format!(
        "https://openpgpkey.{domain}/.well-known/openpgpkey/{domain}/hu/{hash}?l={local}"
    );
    // Direct WKD method: host-meta at main domain.
    let direct_url = format!("https://{domain}/.well-known/openpgpkey/hu/{hash}?l={local}");

    for url in &[advanced_url, direct_url] {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                // Reject before downloading if Content-Length already exceeds the cap.
                if resp.content_length().is_some_and(|l| l > MAX_BYTES as u64) {
                    continue;
                }
                // Stream chunk-by-chunk so chunked Transfer-Encoding cannot bypass
                // the cap by omitting Content-Length (resp.bytes() would buffer all of it).
                let mut resp = resp;
                let mut bytes: Vec<u8> = Vec::with_capacity(MAX_BYTES);
                let mut too_large = false;
                loop {
                    match resp.chunk().await {
                        Ok(Some(chunk)) => {
                            bytes.extend_from_slice(&chunk);
                            if bytes.len() > MAX_BYTES {
                                too_large = true;
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(_) => { too_large = true; break; }
                    }
                }
                if too_large {
                    continue;
                }

                let key_str = if bytes.starts_with(b"-----BEGIN PGP") {
                    String::from_utf8_lossy(&bytes).into_owned()
                } else {
                    use base64::Engine;
                    base64::engine::general_purpose::STANDARD.encode(&bytes)
                };

                return Ok(Json(WkdResponse {
                    found: true,
                    public_key: Some(key_str),
                })
                .into_response());
            }
            _ => continue,
        }
    }

    Ok(Json(WkdResponse {
        found: false,
        public_key: None,
    })
    .into_response())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Each z-base-32 character encodes 5 bits. 5 bytes = 40 bits = 8 chars exactly.
    // All-zero bytes → index 0 in alphabet → 'y' repeated.
    #[test]
    fn test_zbase32_all_zeros_five_bytes() {
        assert_eq!(zbase32_encode(&[0x00; 5]), "yyyyyyyy");
    }

    // All-one bytes → every 5-bit group is 11111 = 31 → index 31 → '9'.
    #[test]
    fn test_zbase32_all_ones_five_bytes() {
        assert_eq!(zbase32_encode(&[0xff; 5]), "99999999");
    }

    // Input crafted so every 5-bit group = 00001 = 1 → 'b'.
    // 0x08 0x42 0x10 0x84 0x21 = 00001000 01000010 00010000 10000100 00100001
    // Grouped: 00001 00001 00001 00001 00001 00001 00001 00001
    #[test]
    fn test_zbase32_every_group_is_one() {
        assert_eq!(zbase32_encode(&[0x08, 0x42, 0x10, 0x84, 0x21]), "bbbbbbbb");
    }

    // 20-byte input (SHA-1 output length) produces exactly 32 characters.
    #[test]
    fn test_zbase32_twenty_bytes_gives_32_chars() {
        let encoded = zbase32_encode(&[0xAB; 20]);
        assert_eq!(encoded.len(), 32);
        assert!(encoded.chars().all(|c| ZBASE32_ALPHABET.contains(&(c as u8))));
    }

    #[test]
    fn test_is_non_public_ip_loopback() {
        assert!(is_non_public_ip("127.0.0.1".parse().unwrap()));
        assert!(is_non_public_ip("::1".parse().unwrap()));
    }

    #[test]
    fn test_is_non_public_ip_private() {
        assert!(is_non_public_ip("10.0.0.1".parse().unwrap()));
        assert!(is_non_public_ip("172.16.0.1".parse().unwrap()));
        assert!(is_non_public_ip("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn test_is_non_public_ip_link_local() {
        assert!(is_non_public_ip("169.254.169.254".parse().unwrap())); // cloud metadata
        assert!(is_non_public_ip("fe80::1".parse().unwrap()));
    }

    #[test]
    fn test_is_non_public_ip_ipv4_mapped() {
        // ::ffff:127.0.0.1 should be treated as loopback
        assert!(is_non_public_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_non_public_ip("::ffff:192.168.0.1".parse().unwrap()));
    }

    #[test]
    fn test_is_non_public_ip_ula() {
        assert!(is_non_public_ip("fc00::1".parse().unwrap()));
        assert!(is_non_public_ip("fd00::1".parse().unwrap()));
    }

    #[test]
    fn test_is_non_public_ip_public_is_allowed() {
        assert!(!is_non_public_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_non_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn test_wkd_hash_is_lowercase() {
        // wkd_hash lowercases the local part before hashing.
        let h1 = wkd_hash("Joe.Doe");
        let h2 = wkd_hash("joe.doe");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_wkd_hash_length() {
        let h = wkd_hash("user");
        assert_eq!(h.len(), 32);
    }
}
