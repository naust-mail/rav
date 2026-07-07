use std::sync::Arc;

use axum::{Extension, Json};
use serde::Serialize;

use crate::config::AppConfig;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    /// Enabled optional capabilities on this server instance.
    pub capabilities: Vec<&'static str>,
}

/// Handler for `GET /api/health`.
///
/// Returns `200 OK` with server status and the set of enabled capabilities.
pub async fn health_check(
    Extension(config): Extension<Arc<AppConfig>>,
) -> Json<HealthResponse> {
    let mut capabilities: Vec<&'static str> = Vec::new();
    if config.pgp_enabled {
        capabilities.push("pgp");
    }
    Json(HealthResponse {
        status: "ok".to_string(),
        capabilities,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::routes::{AppServices, create_router};
    use crate::mfa::passkey::PasskeyService;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_check_returns_ok() {
        use std::time::Duration;
        use crate::auth::session::SessionStore;
        use crate::config::AppConfig;

        let config = Arc::new(AppConfig {
            host: "127.0.0.1".to_string(),
            port: 3001,
            imap_host: None,
            imap_port: 993,
            smtp_host: None,
            smtp_port: 587,
            tls_enabled: true,
            tls_ca_cert_path: None,
            imap_connect_host: None,
            smtp_connect_host: None,
            data_dir: "/tmp/oxi-test".to_string(),
            session_timeout_hours: 24,
            static_dir: "nonexistent_static_dir".to_string(),
            environment: "development".to_string(),
            base_path: None,
            allow_custom_mail_servers: true,
            rspamd_url: None,
            link_proxy_enabled: false,
            webauthn_rp_id: None,
            webauthn_rp_origin: None,
            trusted_proxies: String::new(),
            pgp_enabled: true,
            sieve_host: None,
            sieve_port: 4190,
        });
        let store = Arc::new(SessionStore::new(Duration::from_secs(3600)));
        let transport = Arc::new(crate::mail_transport::MailTransport {
            imap_connector: async_native_tls::TlsConnector::new(),
            imap_connect_host: "127.0.0.1".to_string(),
            smtp_connect_host: "127.0.0.1".to_string(),
            smtp_tls_params: None,
        });
        let passkey_service = Arc::new(
            PasskeyService::from_config(&config).expect("test passkey_service"),
        );
        let app = create_router(AppServices {
            config,
            transport,
            store,
            imap_client: Arc::new(crate::imap::client::mock::MockImapClient::new()),
            smtp_client: Arc::new(crate::smtp::client::mock::MockSmtpClient::new()),
            http_client: Arc::new(reqwest::Client::new()),
            search_engine: Arc::new(crate::search::engine::SearchEngine::new(
                std::path::PathBuf::from("/tmp/oxi-test"),
            )),
            event_bus: Arc::new(crate::realtime::events::EventBus::new()),
            idle_manager: Arc::new(crate::realtime::idle::IdleManager::new()),
            mfa_crypto: Arc::new(
                crate::mfa::crypto::MfaCrypto::from_data_dir("/tmp/oxi-test-mfa")
                    .expect("test mfa_crypto"),
            ),
            passkey_service,
            link_proxy_secret: None,
        });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should succeed");

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();

        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("body should be valid JSON");
        assert_eq!(json["status"], "ok");
        assert_eq!(json["capabilities"], serde_json::json!(["pgp"]));
    }
}
