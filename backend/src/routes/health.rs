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
            data_dir: "/tmp/rav-test".to_string(),
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
            db_pool_max_connections_per_user: 4,
            db_pool_idle_timeout_secs: 600,
            db_pool_max_users: 500,
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
        let imap_client: Arc<dyn crate::imap::client::ImapClient> =
            Arc::new(crate::imap::client::mock::MockImapClient::new());
        let smtp_client: Arc<dyn crate::smtp::client::SmtpClient> =
            Arc::new(crate::smtp::client::mock::MockSmtpClient::new());
        let search_engine = Arc::new(crate::search::engine::SearchEngine::new(
            std::path::PathBuf::from("/tmp/rav-test"),
        ));
        let event_bus = Arc::new(crate::realtime::events::EventBus::new());
        let db_pool_manager = Arc::new(crate::db::pool::DbPoolManager::new(
            "/tmp/rav-test".to_string(),
            4,
            Duration::from_secs(600),
            500,
        ));
        let sync_worker_manager = Arc::new(crate::realtime::worker::SyncWorkerManager::new(
            config.clone(),
            imap_client.clone(),
            event_bus.clone(),
            search_engine.clone(),
            smtp_client.clone(),
            transport.clone(),
            db_pool_manager.clone(),
        ));
        let outbox_worker_manager = Arc::new(crate::realtime::outbox_worker::OutboxWorkerManager::new(
            config.clone(),
            imap_client.clone(),
            smtp_client.clone(),
            transport.clone(),
            event_bus.clone(),
            db_pool_manager.clone(),
        ));
        let app = create_router(AppServices {
            config,
            transport,
            store,
            imap_client,
            smtp_client,
            http_client: Arc::new(reqwest::Client::new()),
            search_engine,
            event_bus,
            idle_manager: Arc::new(crate::realtime::idle::IdleManager::new()),
            sync_worker_manager,
            outbox_worker_manager,
            db_pool_manager,
            mfa_crypto: Arc::new(
                crate::mfa::crypto::MfaCrypto::from_data_dir("/tmp/rav-test-mfa")
                    .expect("test mfa_crypto"),
            ),
            passkey_service,
            link_proxy_secret: None,
            draft_locks: Arc::new(crate::routes::drafts::DraftLocks::new()),
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
