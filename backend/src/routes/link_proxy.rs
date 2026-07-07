use std::sync::Arc;

use axum::Extension;
use axum::extract::Query;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::auth::session::SessionState;
use crate::link_proxy::{LinkProxySecret, verify};

/// Query params for GET /api/v1/link
#[derive(Deserialize)]
pub struct LinkParams {
    /// Base64url-encoded destination URL
    pub u: String,
    /// Unix expiry timestamp
    pub exp: i64,
    /// Hex-encoded HMAC tag (first 16 bytes of HMAC-SHA256)
    pub s: String,
}

pub async fn redirect(
    Extension(session): Extension<SessionState>,
    secret: Option<Extension<Arc<LinkProxySecret>>>,
    Query(params): Query<LinkParams>,
) -> Response {
    let Some(Extension(secret)) = secret else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match verify(&secret.0, &params.u, params.exp, &params.s, &session.user_hash) {
        Ok(url) => {
            let mut res = Response::new(axum::body::Body::empty());
            *res.status_mut() = StatusCode::FOUND;
            if let Ok(v) = HeaderValue::from_str(&url) {
                res.headers_mut().insert(header::LOCATION, v);
            }
            res.headers_mut().insert(
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            );
            res.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store"),
            );
            res
        }
        Err(reason) => {
            tracing::warn!("Link proxy rejected: {reason}");
            let mut res = Response::new(axum::body::Body::from(
                format!("Link invalid or expired: {reason}"),
            ));
            *res.status_mut() = StatusCode::BAD_REQUEST;
            res
        }
    }
}
