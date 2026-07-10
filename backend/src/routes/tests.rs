    use super::*;
    use crate::mfa::passkey::PasskeyService;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::imap::client::mock::MockImapClient;
    use crate::imap::client::{
        EmailAddress, ImapAttachment, ImapError, ImapFolder, ImapMessageBody, ImapMessageHeader,
    };
    use crate::smtp::client::SmtpError;
    use crate::smtp::client::mock::MockSmtpClient;

    /// Helper: create a test AppConfig with the given static dir.
    fn test_config(static_dir: &str) -> Arc<AppConfig> {
        Arc::new(AppConfig {
            host: "127.0.0.1".to_string(),
            port: 3100,
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
            static_dir: static_dir.to_string(),
            environment: "development".to_string(),
            base_path: None,
            allow_custom_mail_servers: true,
            rspamd_url: None,
            link_proxy_enabled: false,
            pgp_enabled: true,
            webauthn_rp_id: None,
            webauthn_rp_origin: None,
            trusted_proxies: String::new(),
            sieve_host: None,
            sieve_port: 4190,
        })
    }

    /// Helper: create a test AppConfig with IMAP host configured and a custom data dir.
    fn test_config_with_imap(static_dir: &str, data_dir: &str) -> Arc<AppConfig> {
        Arc::new(AppConfig {
            host: "127.0.0.1".to_string(),
            port: 3100,
            imap_host: Some("imap.example.com".to_string()),
            imap_port: 993,
            smtp_host: None,
            smtp_port: 587,
            tls_enabled: true,
            tls_ca_cert_path: None,
            imap_connect_host: None,
            smtp_connect_host: None,
            data_dir: data_dir.to_string(),
            session_timeout_hours: 24,
            static_dir: static_dir.to_string(),
            environment: "development".to_string(),
            base_path: None,
            allow_custom_mail_servers: true,
            rspamd_url: None,
            link_proxy_enabled: false,
            pgp_enabled: true,
            webauthn_rp_id: None,
            webauthn_rp_origin: None,
            trusted_proxies: String::new(),
            sieve_host: None,
            sieve_port: 4190,
        })
    }

    /// Helper: create a test AppConfig with IMAP + SMTP hosts configured.
    fn test_config_with_smtp(static_dir: &str, data_dir: &str) -> Arc<AppConfig> {
        Arc::new(AppConfig {
            host: "127.0.0.1".to_string(),
            port: 3001,
            imap_host: Some("imap.example.com".to_string()),
            imap_port: 993,
            smtp_host: Some("smtp.example.com".to_string()),
            smtp_port: 587,
            tls_enabled: true,
            tls_ca_cert_path: None,
            imap_connect_host: None,
            smtp_connect_host: None,
            data_dir: data_dir.to_string(),
            session_timeout_hours: 24,
            static_dir: static_dir.to_string(),
            environment: "development".to_string(),
            base_path: None,
            allow_custom_mail_servers: true,
            rspamd_url: None,
            link_proxy_enabled: false,
            pgp_enabled: true,
            webauthn_rp_id: None,
            webauthn_rp_origin: None,
            trusted_proxies: String::new(),
            sieve_host: None,
            sieve_port: 4190,
        })
    }

    /// Helper: create a default MailTransport with system CA roots and no connect-host overrides.
    fn test_transport() -> Arc<crate::mail_transport::MailTransport> {
        Arc::new(crate::mail_transport::MailTransport {
            imap_connector: async_native_tls::TlsConnector::new(),
            imap_connect_host: "127.0.0.1".to_string(),
            smtp_connect_host: "127.0.0.1".to_string(),
            smtp_tls_params: None,
        })
    }

    /// Helper: create a fresh SessionStore for tests.
    fn test_store() -> Arc<SessionStore> {
        Arc::new(SessionStore::new(Duration::from_secs(3600)))
    }

    /// Helper: create a default mock IMAP client.
    fn test_imap_client() -> Arc<dyn ImapClient> {
        Arc::new(MockImapClient::new())
    }

    /// Helper: create a default mock SMTP client.
    fn test_smtp_client() -> Arc<dyn SmtpClient> {
        Arc::new(MockSmtpClient::new())
    }

    /// Helper: create a shared HTTP client for tests.
    fn test_http_client() -> Arc<reqwest::Client> {
        Arc::new(reqwest::Client::new())
    }

    /// Helper: create a temporary static directory with an index.html.
    fn setup_static_dir() -> TempDir {
        let dir = TempDir::new().expect("should create temp dir");
        fs::write(
            dir.path().join("index.html"),
            "<!DOCTYPE html><html><body>SPA</body></html>",
        )
        .expect("should write index.html");
        dir
    }

    /// Helper: create a test SearchEngine backed by the given data directory.
    fn test_search_engine(data_dir: &str) -> Arc<crate::search::engine::SearchEngine> {
        Arc::new(crate::search::engine::SearchEngine::new(
            std::path::PathBuf::from(data_dir),
        ))
    }

    /// Helper: create a test EventBus.
    fn test_event_bus() -> Arc<crate::realtime::events::EventBus> {
        Arc::new(crate::realtime::events::EventBus::new())
    }

    /// Helper: create a test IdleManager.
    fn test_idle_manager() -> Arc<crate::realtime::idle::IdleManager> {
        Arc::new(crate::realtime::idle::IdleManager::new())
    }

    /// Helper: create an in-memory MfaCrypto using a fixed test key.
    fn test_mfa_crypto() -> Arc<crate::mfa::crypto::MfaCrypto> {
        Arc::new(
            crate::mfa::crypto::MfaCrypto::from_data_dir("/tmp/rav-test-mfa")
                .expect("test MfaCrypto"),
        )
    }

    /// Helper: create a PasskeyService with WebAuthn enabled (localhost RP).
    fn test_passkey_service_with_webauthn() -> Arc<PasskeyService> {
        Arc::new(PasskeyService::from_config(&AppConfig {
            host: "127.0.0.1".to_string(),
            port: 3100,
            imap_host: None,
            imap_port: 993,
            smtp_host: None,
            smtp_port: 587,
            tls_enabled: false,
            tls_ca_cert_path: None,
            imap_connect_host: None,
            smtp_connect_host: None,
            data_dir: "/tmp/rav-test".to_string(),
            session_timeout_hours: 24,
            static_dir: "/tmp".to_string(),
            environment: "development".to_string(),
            base_path: None,
            allow_custom_mail_servers: true,
            rspamd_url: None,
            link_proxy_enabled: false,
            pgp_enabled: true,
            webauthn_rp_id: Some("localhost".to_string()),
            webauthn_rp_origin: Some("http://localhost:3100".to_string()),
            trusted_proxies: String::new(),
            sieve_host: None,
            sieve_port: 4190,
        }).expect("test PasskeyService with WebAuthn"))
    }

    /// Helper: create a PasskeyService with WebAuthn disabled (no RP configured).
    fn test_passkey_service() -> Arc<PasskeyService> {
        Arc::new(PasskeyService::from_config(&AppConfig {
            host: "127.0.0.1".to_string(),
            port: 3100,
            imap_host: None,
            imap_port: 993,
            smtp_host: None,
            smtp_port: 587,
            tls_enabled: false,
            tls_ca_cert_path: None,
            imap_connect_host: None,
            smtp_connect_host: None,
            data_dir: "/tmp/rav-test".to_string(),
            session_timeout_hours: 24,
            static_dir: "/tmp".to_string(),
            environment: "development".to_string(),
            base_path: None,
            allow_custom_mail_servers: true,
            rspamd_url: None,
            link_proxy_enabled: false,
            pgp_enabled: true,
            webauthn_rp_id: None,
            webauthn_rp_origin: None,
            trusted_proxies: String::new(),
            sieve_host: None,
            sieve_port: 4190,
        }).expect("test PasskeyService"))
    }

    /// Helper: build a default AppServices for tests using the given static dir.
    /// Override individual fields with struct update syntax for custom behaviour.
    fn test_services(static_dir: &str) -> AppServices {
        AppServices {
            config: test_config(static_dir),
            transport: test_transport(),
            store: test_store(),
            imap_client: test_imap_client(),
            smtp_client: test_smtp_client(),
            http_client: test_http_client(),
            search_engine: test_search_engine("/tmp/rav-test"),
            event_bus: test_event_bus(),
            idle_manager: test_idle_manager(),
            mfa_crypto: test_mfa_crypto(),
            passkey_service: test_passkey_service(),
            link_proxy_secret: None,
        }
    }

    /// Helper: create a multi-account session for testing protected routes.
    /// Returns (browser_id, account_id, token) for use in request headers.
    fn setup_test_account(
        store: &SessionStore,
        email: &str,
    ) -> (String, String, String) {
        let browser_id = store.create_browser();
        let (token, account_id) = store.add_account_to_browser(
            &browser_id,
            email.to_string(),
            "password".to_string(),
            crate::auth::user_data::hash_email(email),
            "imap.example.com".to_string(),
            993,
            true,
            "smtp.example.com".to_string(),
            587,
            true,
        );
        (browser_id, account_id, token)
    }

    /// Helper: build auth headers for multi-account requests.
    fn auth_headers(browser_id: &str, account_id: &str, token: &str) -> Vec<(&'static str, String)> {
        vec![
            ("cookie", format!("rav_browser={browser_id}; rav_session_{account_id}={token}")),
            ("x-active-account", account_id.to_string()),
        ]
    }

    /// Helper: provision a user database so that route handlers can open it.
    fn provision_user_db(data_dir: &str, user_hash: &str) {
        crate::auth::user_data::provision_user_data(data_dir, user_hash).unwrap();
    }

    // -----------------------------------------------------------------------
    // Existing tests (updated to pass imap_client)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn api_health_works_with_static_fallback() {
        let dir = setup_static_dir();
        let config = test_config(dir.path().to_str().unwrap());
        let store = test_store();
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn root_serves_index_html() {
        let dir = setup_static_dir();
        let config = test_config(dir.path().to_str().unwrap());
        let store = test_store();
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("SPA"));
    }

    #[tokio::test]
    async fn unknown_path_falls_back_to_index_html() {
        let dir = setup_static_dir();
        let config = test_config(dir.path().to_str().unwrap());
        let store = test_store();
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("SPA"));
    }

    #[tokio::test]
    async fn static_file_is_served_directly() {
        let dir = setup_static_dir();
        fs::write(dir.path().join("style.css"), "body { color: red; }").unwrap();
        let config = test_config(dir.path().to_str().unwrap());
        let store = test_store();
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/style.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let css = String::from_utf8(body.to_vec()).unwrap();
        assert!(css.contains("color: red"));
    }

    #[tokio::test]
    async fn nested_spa_path_falls_back_to_index() {
        let dir = setup_static_dir();
        let config = test_config(dir.path().to_str().unwrap());
        let store = test_store();
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/inbox/some-message-id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("SPA"));
    }

    #[tokio::test]
    async fn login_without_csrf_header_returns_403() {
        let dir = setup_static_dir();
        let config = test_config(dir.path().to_str().unwrap());
        let store = test_store();
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"email":"test@test.com","password":"pass"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn login_no_imap_host_returns_503() {
        let dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let mut cfg = (*test_config(dir.path().to_str().unwrap())).clone();
        cfg.data_dir = data_dir.path().to_str().unwrap().to_string();
        let config = Arc::new(cfg);
        let store = test_store();
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .header("x-requested-with", "XMLHttpRequest")
                    .body(Body::from(
                        r#"{"email":"test@test.com","password":"pass"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "SERVICE_UNAVAILABLE");
        assert_eq!(json["error"]["message"], "IMAP server not configured");
    }

    #[tokio::test]
    async fn login_empty_email_returns_400() {
        let dir = setup_static_dir();
        let mut cfg = (*test_config(dir.path().to_str().unwrap())).clone();
        cfg.imap_host = Some("127.0.0.1".to_string());
        let config = Arc::new(cfg);
        let store = test_store();
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .header("x-requested-with", "XMLHttpRequest")
                    .body(Body::from(r#"{"email":"","password":"pass"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "BAD_REQUEST");
    }

    #[tokio::test]
    async fn login_empty_password_returns_400() {
        let dir = setup_static_dir();
        let mut cfg = (*test_config(dir.path().to_str().unwrap())).clone();
        cfg.imap_host = Some("127.0.0.1".to_string());
        let config = Arc::new(cfg);
        let store = test_store();
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .header("x-requested-with", "XMLHttpRequest")
                    .body(Body::from(
                        r#"{"email":"test@test.com","password":""}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn login_unreachable_imap_returns_503() {
        let dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let mut cfg = (*test_config(dir.path().to_str().unwrap())).clone();
        cfg.imap_host = Some("127.0.0.1".to_string());
        cfg.imap_port = 19999; // Nothing listening here
        cfg.tls_enabled = false;
        cfg.data_dir = data_dir.path().to_str().unwrap().to_string();
        let config = Arc::new(cfg);
        let store = test_store();
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .header("x-requested-with", "XMLHttpRequest")
                    .body(Body::from(
                        r#"{"email":"test@test.com","password":"pass"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "SERVER_UNREACHABLE");
        assert!(json["error"]["message"].as_str().unwrap().contains("Connection refused"));
    }

    #[tokio::test]
    async fn get_session_without_auth_returns_401() {
        let dir = setup_static_dir();
        let config = test_config(dir.path().to_str().unwrap());
        let store = test_store();
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/session")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_session_with_valid_session_returns_200() {
        let dir = setup_static_dir();
        let config = test_config(dir.path().to_str().unwrap());
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let mut req = Request::builder()
            .uri("/api/auth/session");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["user"]["email"], "alice@example.com");
    }

    #[tokio::test]
    async fn logout_without_auth_returns_401() {
        let dir = setup_static_dir();
        let config = test_config(dir.path().to_str().unwrap());
        let store = test_store();
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/logout")
                    .header("x-requested-with", "XMLHttpRequest")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn logout_with_valid_session_returns_200() {
        let dir = setup_static_dir();
        let config = test_config(dir.path().to_str().unwrap());
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let app = create_router(AppServices { config, store: store.clone(), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("POST")
            .uri("/api/auth/logout")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "logged_out");

        // Session should be removed from the store.
        assert!(store.get(&token).is_none());
    }

    #[tokio::test]
    async fn logout_clears_cookie() {
        let dir = setup_static_dir();
        let config = test_config(dir.path().to_str().unwrap());
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("POST")
            .uri("/api/auth/logout")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let set_cookie = response
            .headers()
            .get_all("set-cookie");
        // Should have clearing cookies for both browser and account session
        let cookie_str = format!("{:?}", set_cookie);
        assert!(cookie_str.contains("rav_browser=;") || cookie_str.contains("Max-Age=0"));
    }

    // -----------------------------------------------------------------------
    // New tests for folders and messages routes
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_folders_returns_200_with_folder_list() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");

        // Provision user database.
        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        let mock = MockImapClient::new().with_folders(vec![
            ImapFolder {
                name: "INBOX".to_string(),
                delimiter: Some("/".to_string()),
                attributes: vec!["\\HasNoChildren".to_string()],
            },
            ImapFolder {
                name: "Sent".to_string(),
                delimiter: Some("/".to_string()),
                attributes: vec![],
            },
        ]);
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .uri("/api/folders")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let folders = json["folders"].as_array().unwrap();
        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0]["name"], "INBOX");
        assert_eq!(folders[1]["name"], "Sent");
        // recent_messages is always present, empty when no messages are cached.
        assert_eq!(folders[0]["recent_messages"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_folders_includes_recent_messages_when_cached() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        // Seed a folder and a message before the request so they appear in recent_messages.
        let conn = crate::db::pool::open_user_db(data_dir.path().to_str().unwrap(), &user_hash).unwrap();
        crate::db::folders::upsert_folder(&conn, "INBOX", None, None, "", true, 0, 0, 0, 0).unwrap();
        crate::db::messages::upsert_message(
            &conn, "INBOX", 1, None, None, None, "Hello world", "a@b.com", "Alice",
            "[]", "[]", "2024-01-01", 0, "", 0, false, "", None,
        ).unwrap();
        drop(conn);

        let mock = MockImapClient::new().with_folders(vec![ImapFolder {
            name: "INBOX".to_string(),
            delimiter: Some("/".to_string()),
            attributes: vec![],
        }]);
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices {
            config,
            store,
            imap_client,
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let mut req = Request::builder()
            .uri("/api/folders")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let inbox = &json["folders"][0];
        assert_eq!(inbox["name"], "INBOX");
        let recent = inbox["recent_messages"].as_array().unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0]["subject"], "Hello world");
        assert_eq!(recent[0]["uid"], 1);
    }

    #[tokio::test]
    async fn get_folders_returns_503_when_imap_fails() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        let mock = MockImapClient::new()
            .with_error(ImapError::ConnectionFailed(crate::error::ConnectError::Unreachable));
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .uri("/api/folders")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn get_folders_returns_401_without_auth() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let app = create_router(AppServices { config, store, ..test_services("/tmp") });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/folders")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_messages_returns_200_with_paginated_list() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let cipher = crate::folder_cipher::FolderCipher::from_session_token(&token);
        let inbox_id = cipher.encrypt("INBOX");

        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        let mock = MockImapClient::new().with_headers(vec![
            ImapMessageHeader {
                uid: 1,
                subject: Some("Hello World".to_string()),
                from: vec![EmailAddress {
                    name: Some("Alice".to_string()),
                    address: "alice@example.com".to_string(),
                }],
                to: vec![EmailAddress {
                    name: None,
                    address: "bob@example.com".to_string(),
                }],
                date: Some("2024-01-01T10:00:00Z".to_string()),
                date_epoch: 0,
                flags: vec!["\\Seen".to_string()],
                has_attachments: false,
                size: 2048,
                message_id: None,
                in_reply_to: None,
                references: None,
                cc: vec![],
                reaction: None,
            },
            ImapMessageHeader {
                uid: 2,
                subject: Some("Second message".to_string()),
                from: vec![EmailAddress {
                    name: Some("Bob".to_string()),
                    address: "bob@example.com".to_string(),
                }],
                to: vec![EmailAddress {
                    name: None,
                    address: "alice@example.com".to_string(),
                }],
                date: Some("2024-01-02T10:00:00Z".to_string()),
                date_epoch: 0,
                flags: vec![],
                has_attachments: false,
                size: 4096,
                message_id: None,
                in_reply_to: None,
                references: None,
                cc: vec![],
                reaction: None,
            },
        ]);
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .uri(format!("/api/folders/{inbox_id}/messages?page=0&per_page=50"))
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["total_count"], 2);
        assert_eq!(json["page"], 0);
        assert_eq!(json["per_page"], 50);

        let messages = json["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
    }

    #[tokio::test]
    async fn get_message_returns_sanitized_html() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let cipher = crate::folder_cipher::FolderCipher::from_session_token(&token);
        let inbox_id = cipher.encrypt("INBOX");

        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        // First, we need the message header in cache (fetch_headers first).
        let mock = MockImapClient::new()
            .with_headers(vec![ImapMessageHeader {
                uid: 42,
                subject: Some("Test Subject".to_string()),
                from: vec![EmailAddress {
                    name: Some("Alice".to_string()),
                    address: "alice@example.com".to_string(),
                }],
                to: vec![EmailAddress {
                    name: None,
                    address: "bob@example.com".to_string(),
                }],
                date: Some("2024-01-01T10:00:00Z".to_string()),
                date_epoch: 0,
                flags: vec!["\\Seen".to_string()],
                has_attachments: false,
                size: 1024,
                message_id: None,
                in_reply_to: None,
                references: None,
                cc: vec![],
                reaction: None,
            }])
            .with_bodies(vec![ImapMessageBody {
                uid: 42,
                text_plain: Some("Hello plain text".to_string()),
                text_html: Some(
                    "<p>Hello</p><script>alert('xss')</script><b>bold</b>".to_string(),
                ),
                attachments: vec![],
                raw_headers: String::new(),
                pgp_status: None,
            }]);
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices { config: config.clone(), store: store.clone(), imap_client: imap_client.clone(), search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        // First, populate the message cache by listing messages.
        let mut req1 = Request::builder()
            .uri(format!("/api/folders/{inbox_id}/messages"))
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req1 = req1.header(name, value);
        }
        let response = app
            .oneshot(req1.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Now get the full message.
        let app2 = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });
        let inbox_id2 = cipher.encrypt("INBOX");
        let mut req2 = Request::builder()
            .uri(format!("/api/messages/{inbox_id2}/42"))
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req2 = req2.header(name, value);
        }
        let response = app2
            .oneshot(req2.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["uid"], 42);
        assert_eq!(json["subject"], "Test Subject");

        // Script tag is preserved (frontend handles security via iframe sandbox).
        let html = json["html"].as_str().unwrap();
        assert!(html.contains("script"));
        assert!(html.contains("<b>bold</b>"));
        assert!(html.contains("<p>Hello</p>"));

        // Plain text should be preserved.
        assert_eq!(json["text"], "Hello plain text");

        // Flags should be an array.
        assert!(json["flags"].is_array());

        // to_addresses should be an array.
        assert!(json["to_addresses"].is_array());
    }

    #[tokio::test]
    async fn update_flags_returns_200() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let cipher = crate::folder_cipher::FolderCipher::from_session_token(&token);
        let inbox_id = cipher.encrypt("INBOX");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        // Seed a message in the cache.
        let conn = crate::db::pool::open_user_db(
            data_dir.path().to_str().unwrap(),
            &user_hash,
        )
        .unwrap();
        crate::db::folders::upsert_folder(&conn, "INBOX", None, None, "", true, 0, 0, 0, 0)
            .unwrap();
        crate::db::messages::upsert_message(
            &conn, "INBOX", 1, None, None, None, "Test", "a@b.com", "A", "[]", "[]",
            "2024-01-01", 0, "", 0, false, "", None,
        )
        .unwrap();
        drop(conn);

        let mock = MockImapClient::new();
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("PATCH")
            .uri(format!("/api/messages/{inbox_id}/1/flags"))
            .header("x-requested-with", "XMLHttpRequest")
            .header("content-type", "application/json");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(
                r#"{"flags":["\\Seen","\\Flagged"],"add":true}"#,
            )).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn update_flags_rejects_invalid_flag_characters() {
        // Flag values must be IMAP atom characters (RFC 3501 §9). A value
        // containing a space — e.g. two flags accidentally joined into one
        // string — must be rejected with 400 before reaching the IMAP client.
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");

        let mock = MockImapClient::new();
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("PATCH")
            .uri("/api/messages/INBOX/1/flags")
            .header("x-requested-with", "XMLHttpRequest")
            .header("content-type", "application/json");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(
                r#"{"flags":["\\Seen \\Flagged"],"add":true}"#,
            )).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        // BAD_REQUEST confirms rejection at the validation layer — the IMAP
        // error path returns SERVICE_UNAVAILABLE (503), not 400.
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "BAD_REQUEST");
    }

    #[tokio::test]
    async fn update_flags_rejects_empty_flag() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");

        let mock = MockImapClient::new();
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("PATCH")
            .uri("/api/messages/INBOX/1/flags")
            .header("x-requested-with", "XMLHttpRequest")
            .header("content-type", "application/json");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(
                r#"{"flags":[""],"add":true}"#,
            )).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn move_message_returns_200() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let cipher = crate::folder_cipher::FolderCipher::from_session_token(&token);
        let inbox_id = cipher.encrypt("INBOX");
        let archive_id = cipher.encrypt("Archive");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        // Seed a message in the cache.
        let conn = crate::db::pool::open_user_db(
            data_dir.path().to_str().unwrap(),
            &user_hash,
        )
        .unwrap();
        crate::db::folders::upsert_folder(&conn, "INBOX", None, None, "", true, 0, 0, 0, 0)
            .unwrap();
        crate::db::messages::upsert_message(
            &conn, "INBOX", 42, None, None, None, "Test", "a@b.com", "A", "[]", "[]",
            "2024-01-01", 0, "", 0, false, "", None,
        )
        .unwrap();
        drop(conn);

        let mock = MockImapClient::new();
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let move_body = format!(r#"{{"from_folder":"{inbox_id}","to_folder":"{archive_id}","uid":42}}"#);
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/messages/move")
            .header("x-requested-with", "XMLHttpRequest")
            .header("content-type", "application/json");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(move_body)).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn delete_message_returns_200() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let cipher = crate::folder_cipher::FolderCipher::from_session_token(&token);
        let inbox_id = cipher.encrypt("INBOX");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        // Seed a message in the cache.
        let conn = crate::db::pool::open_user_db(
            data_dir.path().to_str().unwrap(),
            &user_hash,
        )
        .unwrap();
        crate::db::folders::upsert_folder(&conn, "INBOX", None, None, "", true, 0, 0, 0, 0)
            .unwrap();
        crate::db::messages::upsert_message(
            &conn, "INBOX", 7, None, None, None, "Test", "a@b.com", "A", "[]", "[]",
            "2024-01-01", 0, "", 0, false, "", None,
        )
        .unwrap();
        drop(conn);

        let mock = MockImapClient::new();
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("DELETE")
            .uri(format!("/api/messages/{inbox_id}/7"))
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    // -----------------------------------------------------------------------
    // Attachment download tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn download_attachment_returns_binary_with_correct_headers() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let cipher = crate::folder_cipher::FolderCipher::from_session_token(&token);
        let inbox_id = cipher.encrypt("INBOX");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        let attachment_data: Vec<u8> = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let mock = MockImapClient::new().with_bodies(vec![ImapMessageBody {
            uid: 42,
            text_plain: Some("text".to_string()),
            text_html: None,
            attachments: vec![ImapAttachment {
                filename: Some("document.pdf".to_string()),
                content_type: "application/pdf".to_string(),
                size: 4,
                data: attachment_data.clone(),
                content_id: None,
            }],
            raw_headers: String::new(),
            pgp_status: None,
        }]);
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .uri(format!("/api/messages/{inbox_id}/42/attachments/0"))
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Verify content-type header.
        let ct = response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(ct, "application/pdf");

        // Verify content-disposition header.
        let cd = response
            .headers()
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cd.contains("inline"));
        assert!(cd.contains("document.pdf"));

        // Verify body bytes match the attachment data.
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body.as_ref(), &attachment_data);
    }

    #[tokio::test]
    async fn download_attachment_strips_control_chars_from_filename() {
        // MIME decoders can surface control characters in attachment filenames
        // via quoted-pair escapes. Control characters must be stripped before
        // the filename is used in the Content-Disposition header.
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let cipher = crate::folder_cipher::FolderCipher::from_session_token(&token);
        let inbox_id = cipher.encrypt("INBOX");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        let mock = MockImapClient::new().with_bodies(vec![ImapMessageBody {
            uid: 1,
            text_plain: Some("text".to_string()),
            text_html: None,
            attachments: vec![ImapAttachment {
                filename: Some("doc\r\nument.pdf".to_string()),
                content_type: "application/pdf".to_string(),
                size: 4,
                data: vec![0u8; 4],
                content_id: None,
            }],
            raw_headers: String::new(),
            pgp_status: None,
        }]);
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .uri(format!("/api/messages/{inbox_id}/1/attachments/0"))
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let cd = response
            .headers()
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap();
        // Control characters stripped; the rest of the filename is intact.
        assert!(cd.contains("document.pdf"), "got: {cd}");
        assert!(!cd.contains('\r'));
        assert!(!cd.contains('\n'));
    }

    #[tokio::test]
    async fn download_attachment_returns_404_for_invalid_index() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let cipher = crate::folder_cipher::FolderCipher::from_session_token(&token);
        let inbox_id = cipher.encrypt("INBOX");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        let mock = MockImapClient::new().with_bodies(vec![ImapMessageBody {
            uid: 42,
            text_plain: Some("text".to_string()),
            text_html: None,
            attachments: vec![ImapAttachment {
                filename: Some("document.pdf".to_string()),
                content_type: "application/pdf".to_string(),
                size: 4,
                data: vec![0xDE, 0xAD, 0xBE, 0xEF],
                content_id: None,
            }],
            raw_headers: String::new(),
            pgp_status: None,
        }]);
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .uri(format!("/api/messages/{inbox_id}/42/attachments/99"))
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn download_attachment_returns_400_for_non_numeric_id() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        let mock = MockImapClient::new().with_bodies(vec![ImapMessageBody {
            uid: 42,
            text_plain: Some("text".to_string()),
            text_html: None,
            attachments: vec![ImapAttachment {
                filename: Some("document.pdf".to_string()),
                content_type: "application/pdf".to_string(),
                size: 4,
                data: vec![0xDE, 0xAD, 0xBE, 0xEF],
                content_id: None,
            }],
            raw_headers: String::new(),
            pgp_status: None,
        }]);
        let imap_client: Arc<dyn ImapClient> = Arc::new(mock);
        let app = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .uri("/api/messages/INBOX/42/attachments/abc")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    // -----------------------------------------------------------------------
    // Send message endpoint tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn send_returns_200_on_success() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_smtp(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        let mock_smtp: Arc<dyn SmtpClient> = Arc::new(MockSmtpClient::new());
        let mock_imap: Arc<dyn ImapClient> = Arc::new(MockImapClient::new());
        let app = create_router(AppServices { config, store, imap_client: mock_imap, smtp_client: mock_smtp, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("POST")
            .uri("/api/messages/send")
            .header("x-requested-with", "XMLHttpRequest")
            .header("content-type", "application/json");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(
                r#"{"to":["bob@example.com"],"subject":"Hello","text_body":"Hi Bob"}"#,
            )).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "sent");
        assert!(json["message_id"].as_str().is_some());
    }

    #[tokio::test]
    async fn send_returns_400_without_recipients() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_smtp(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        let app = create_router(AppServices { config, store, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("POST")
            .uri("/api/messages/send")
            .header("x-requested-with", "XMLHttpRequest")
            .header("content-type", "application/json");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(
                r#"{"to":[],"subject":"Hello","text_body":"Hi"}"#,
            )).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "BAD_REQUEST");
    }

    #[tokio::test]
    async fn send_returns_400_with_empty_body_and_subject() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_smtp(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        let app = create_router(AppServices { config, store, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("POST")
            .uri("/api/messages/send")
            .header("x-requested-with", "XMLHttpRequest")
            .header("content-type", "application/json");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(
                r#"{"to":["bob@example.com"],"subject":"","text_body":""}"#,
            )).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn send_returns_503_when_smtp_not_configured() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        // Use config WITHOUT smtp_host
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        let app = create_router(AppServices { config, store, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("POST")
            .uri("/api/messages/send")
            .header("x-requested-with", "XMLHttpRequest")
            .header("content-type", "application/json");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(
                r#"{"to":["bob@example.com"],"subject":"Hello","text_body":"Hi"}"#,
            )).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "SERVICE_UNAVAILABLE");
    }

    #[tokio::test]
    async fn send_returns_503_when_smtp_fails() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_smtp(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        let failing_smtp: Arc<dyn SmtpClient> = Arc::new(
            MockSmtpClient::new()
                .with_error(SmtpError::SendFailed("relay denied".to_string())),
        );
        let app = create_router(AppServices { config, store, smtp_client: failing_smtp, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("POST")
            .uri("/api/messages/send")
            .header("x-requested-with", "XMLHttpRequest")
            .header("content-type", "application/json");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(
                r#"{"to":["bob@example.com"],"subject":"Hello","text_body":"Hi"}"#,
            )).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("relay denied"));
    }

    #[tokio::test]
    async fn export_contact_strips_control_chars_from_filename() {
        // A contact name containing control characters must have them stripped
        // before the name is used in the Content-Disposition header.
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        // Insert a contact whose name contains a newline.
        let conn = crate::db::pool::open_user_db(
            data_dir.path().to_str().unwrap(),
            &user_hash,
        ).unwrap();
        let contact = crate::db::contacts::Contact {
            id: "c1".to_string(),
            email: "bob@example.com".to_string(),
            name: "Bob\nSmith".to_string(),
            company: String::new(),
            notes: String::new(),
            is_favorite: false,
            last_contacted: None,
            contact_count: 0,
            source: "manual".to_string(),
            created_at: "2024-01-01 00:00:00".to_string(),
            updated_at: "2024-01-01 00:00:00".to_string(),
        };
        crate::db::contacts::upsert_contact(&conn, &contact).unwrap();
        drop(conn);

        let imap_client: Arc<dyn ImapClient> = Arc::new(MockImapClient::new());
        let app = create_router(AppServices { config, store, imap_client, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .uri("/api/contacts/c1/export")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let cd = response
            .headers()
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap();
        // Control character stripped; the rest of the name is intact.
        assert!(cd.contains("BobSmith.vcf"), "got: {cd}");
        assert!(!cd.contains('\n'));
    }

    // -----------------------------------------------------------------------
    // Spam reporting
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn report_spam_without_rspamd_returns_trained_false() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let cipher = crate::folder_cipher::FolderCipher::from_session_token(&token);
        let inbox_id = cipher.encrypt("INBOX");
        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        let app = create_router(AppServices { config, store, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("POST")
            .uri(format!("/api/messages/{inbox_id}/42/report-spam"))
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["trained"], false);
    }

    #[tokio::test]
    async fn report_ham_without_rspamd_returns_trained_false() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        let cipher = crate::folder_cipher::FolderCipher::from_session_token(&token);
        let junk_id = cipher.encrypt("Junk");
        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        let app = create_router(AppServices { config, store, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("POST")
            .uri(format!("/api/messages/{junk_id}/42/report-ham"))
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["trained"], false);
    }

    // -----------------------------------------------------------------------
    // Vacation responder routes
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_vacation_returns_defaults() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        let app = create_router(AppServices { config, store, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .uri("/api/settings/vacation")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["enabled"], false);
        assert_eq!(json["subject"], "Out of office");
        assert_eq!(json["reply_interval_hours"], 24);
    }

    #[tokio::test]
    async fn put_vacation_updates_and_returns_new_values() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        let app = create_router(AppServices { config, store, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("PUT")
            .uri("/api/settings/vacation")
            .header("content-type", "application/json")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(r#"{"enabled":true,"subject":"Away","body":"Back Monday","reply_interval_hours":48}"#)).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["enabled"], true);
        assert_eq!(json["subject"], "Away");
        assert_eq!(json["body"], "Back Monday");
        assert_eq!(json["reply_interval_hours"], 48);
    }

    // -----------------------------------------------------------------------
    // Filter rule routes
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_filters_returns_empty_initially() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        let app = create_router(AppServices { config, store, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .uri("/api/filters")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["rules"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn create_filter_then_list_and_delete() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        let app = create_router(AppServices { config, store, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        // POST - create
        let create_body = r#"{
            "name": "Move newsletters",
            "conditions": [{"field":"subject","op":"contains","value":"newsletter"}],
            "actions": [{"action_type":"move","action_value":"Newsletters"}]
        }"#;
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/filters")
            .header("content-type", "application/json")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .clone()
            .oneshot(req.body(Body::from(create_body)).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(created["name"], "Move newsletters");
        assert_eq!(created["enabled"], true);
        let filter_id = created["id"].as_str().unwrap().to_string();

        // DELETE
        let mut req = Request::builder()
            .method("DELETE")
            .uri(format!("/api/filters/{filter_id}"))
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn delete_nonexistent_filter_returns_404() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        let app = create_router(AppServices { config, store, search_engine: test_search_engine(data_dir.path().to_str().unwrap()), ..test_services("/tmp") });

        let mut req = Request::builder()
            .method("DELETE")
            .uri("/api/filters/nonexistent-id")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // MFA / passkey route tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn mfa_status_returns_passkey_fields() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        provision_user_db(
            data_dir.path().to_str().unwrap(),
            &crate::auth::user_data::hash_email("alice@example.com"),
        );

        let app = create_router(AppServices {
            config,
            store,
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let mut req = Request::builder()
            .uri("/api/mfa/status")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["passkey_count"], 0);
        assert_eq!(json["passkey_only"], false);
        assert_eq!(json["totp_enabled"], false);
    }

    #[tokio::test]
    async fn passkey_register_begin_without_auth_returns_401() {
        let app = create_router(test_services("/tmp"));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/mfa/passkey/register/begin")
                    .header("content-type", "application/json")
                    .header("x-requested-with", "XMLHttpRequest")
                    .body(Body::from(r#"{"name":"My Key"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn passkey_login_begin_without_csrf_returns_403() {
        let app = create_router(test_services("/tmp"));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/passkey/login/begin")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"email":"alice@example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn passkey_login_begin_without_webauthn_returns_503() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        provision_user_db(
            data_dir.path().to_str().unwrap(),
            &crate::auth::user_data::hash_email("alice@example.com"),
        );

        let app = create_router(AppServices {
            config,
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/passkey/login/begin")
                    .header("content-type", "application/json")
                    .header("x-requested-with", "XMLHttpRequest")
                    .body(Body::from(r#"{"email":"alice@example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn passkey_list_returns_empty_initially() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        provision_user_db(
            data_dir.path().to_str().unwrap(),
            &crate::auth::user_data::hash_email("alice@example.com"),
        );

        let app = create_router(AppServices {
            config,
            store,
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let mut req = Request::builder()
            .uri("/api/mfa/passkeys")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["passkeys"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn passkey_only_set_without_passkeys_returns_400() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        provision_user_db(
            data_dir.path().to_str().unwrap(),
            &crate::auth::user_data::hash_email("alice@example.com"),
        );

        let app = create_router(AppServices {
            config,
            store,
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let mut req = Request::builder()
            .method("PUT")
            .uri("/api/mfa/settings/passkey-only")
            .header("content-type", "application/json")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(r#"{"enabled":true}"#)).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    // -----------------------------------------------------------------------
    // Security audit regression tests
    // -----------------------------------------------------------------------

    /// Fix #2: passkey login begin for a non-existent email must return 200
    /// (anti-enumeration fake challenge) rather than 500.
    #[tokio::test]
    async fn passkey_login_begin_nonexistent_user_returns_200() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap(); // empty - no user dirs
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let passkey_service = test_passkey_service_with_webauthn();
        let app = create_router(AppServices {
            config,
            passkey_service,
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/passkey/login/begin")
                    .header("content-type", "application/json")
                    .header("x-requested-with", "XMLHttpRequest")
                    .body(Body::from(r#"{"email":"ghost@example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Non-existent user must not return 500 - anti-enumeration: return a fake challenge.
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Fix #1: the fake challenge returned for an unknown user must include a
    /// non-empty allowCredentials array so a timing observer cannot distinguish
    /// it from a real challenge.
    #[tokio::test]
    async fn passkey_login_begin_fake_challenge_has_nonempty_allow_credentials() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let passkey_service = test_passkey_service_with_webauthn();
        let app = create_router(AppServices {
            config,
            passkey_service,
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/mfa/passkey/login/begin")
                    .header("content-type", "application/json")
                    .header("x-requested-with", "XMLHttpRequest")
                    .body(Body::from(r#"{"email":"ghost@example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let allow_creds = json["options"]["publicKey"]["allowCredentials"]
            .as_array()
            .expect("allowCredentials should be an array");
        assert!(
            !allow_creds.is_empty(),
            "fake challenge must have a non-empty allowCredentials to prevent enumeration"
        );
    }

    /// Fix #8: deleting the last passkey while passkey-only mode is enabled
    /// must be refused with 400 to prevent account lockout.
    #[tokio::test]
    async fn passkey_delete_last_key_with_passkey_only_returns_400() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        let conn = crate::db::pool::open_user_db(data_dir.path().to_str().unwrap(), &user_hash).unwrap();
        crate::db::mfa::insert_passkey(
            &conn, "cred-solo", r#"{"test":true}"#, &[0u8; 32],
            &[0u8; 16], &[0u8; 12], "Solo Key",
            "imap.example.com", 993, true, "smtp.example.com", 587, true,
        ).unwrap();
        crate::db::mfa::set_passkey_only(&conn, true).unwrap();
        drop(conn);

        let app = create_router(AppServices {
            config,
            store,
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let mut req = Request::builder()
            .method("DELETE")
            .uri("/api/mfa/passkeys/cred-solo")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "BAD_REQUEST");
    }

    /// Fix #7: TOTP delete while locked out must return 401 regardless of the
    /// supplied code, preventing brute-force through the delete endpoint.
    #[tokio::test]
    async fn totp_delete_when_locked_out_returns_401() {
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let mfa_crypto = test_mfa_crypto();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        let conn = crate::db::pool::open_user_db(data_dir.path().to_str().unwrap(), &user_hash).unwrap();

        // Store a dummy encrypted TOTP secret so the route doesn't short-circuit with 400.
        let secret = crate::mfa::totp::generate_secret().unwrap();
        let (encrypted, nonce) = mfa_crypto.encrypt(secret.as_bytes()).unwrap();
        crate::db::mfa::upsert_totp_secret(&conn, &encrypted, &nonce).unwrap();

        // Activate the lockout directly via SQL.
        conn.execute(
            "UPDATE mfa_lockout SET locked_until = unixepoch() + 3600 WHERE id = 1",
            [],
        ).unwrap();
        drop(conn);

        let app = create_router(AppServices {
            config,
            store,
            mfa_crypto: mfa_crypto.clone(),
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let mut req = Request::builder()
            .method("DELETE")
            .uri("/api/mfa/totp")
            .header("content-type", "application/json")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(r#"{"code":"000000"}"#)).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // -----------------------------------------------------------------------
    // Folder ID security
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn raw_folder_name_in_url_returns_bad_request() {
        // Passing a plaintext folder name instead of an encrypted folder ID
        // must return 400 - the decryption guard rejects anything that is not
        // a valid AES-GCM ciphertext produced by the session cipher.
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        let app = create_router(AppServices {
            config,
            store,
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let mut req = Request::builder()
            .uri("/api/messages/INBOX/1")
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "BAD_REQUEST");
    }

    #[tokio::test]
    async fn cross_session_folder_id_is_rejected() {
        // An encrypted folder ID from session A is silently invalid in session B
        // because the key is derived from the session token. Different sessions
        // produce different keys, so decryption fails and returns 400.
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (_, _, token_a) = setup_test_account(&store, "alice@example.com");
        let (browser_b, account_b, token_b) = setup_test_account(&store, "bob@example.com");

        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("bob@example.com"));

        // Encrypt a folder ID under session A's key.
        let folder_id_from_a = crate::folder_cipher::FolderCipher::from_session_token(&token_a)
            .encrypt("INBOX");

        // Sanity: session B's cipher must fail to decrypt session A's ID.
        assert!(
            crate::folder_cipher::FolderCipher::from_session_token(&token_b)
                .decrypt(&folder_id_from_a)
                .is_err()
        );

        let app = create_router(AppServices {
            config,
            store,
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let mut req = Request::builder()
            .uri(format!("/api/messages/{folder_id_from_a}/1"))
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_b, &account_b, &token_b) {
            req = req.header(name, value);
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn tampered_folder_id_returns_bad_request() {
        // A folder ID with a flipped bit in the ciphertext is rejected by AES-GCM
        // tag verification. The server returns 400 with a generic "Invalid folder ID"
        // error and does not leak whether the key or the ciphertext was wrong.
        use base64::Engine;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;

        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        let cipher = crate::folder_cipher::FolderCipher::from_session_token(&token);
        let valid_id = cipher.encrypt("INBOX");

        // Flip a bit in the ciphertext area (bytes 12+ are ciphertext+tag).
        let mut bytes = URL_SAFE_NO_PAD.decode(valid_id.as_str()).unwrap();
        bytes[20] ^= 0xFF;
        let tampered_id = URL_SAFE_NO_PAD.encode(&bytes);

        let app = create_router(AppServices {
            config,
            store,
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let mut req = Request::builder()
            .uri(format!("/api/messages/{tampered_id}/1"))
            .header("x-requested-with", "XMLHttpRequest");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn search_without_q_returns_bad_request() {
        // POST /api/search with no q field, an empty q, or a whitespace-only q
        // must all return 400.
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        for body in [r#"{}"#, r#"{"q":""}"#, r#"{"q":"   "}"#] {
            let config = test_config_with_imap(
                static_dir.path().to_str().unwrap(),
                data_dir.path().to_str().unwrap(),
            );
            let app = create_router(AppServices {
                config,
                store: store.clone(),
                search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
                ..test_services("/tmp")
            });
            let mut req = Request::builder()
                .method("POST")
                .uri("/api/search")
                .header("x-requested-with", "XMLHttpRequest")
                .header("content-type", "application/json");
            for (name, value) in auth_headers(&browser_id, &account_id, &token) {
                req = req.header(name, value);
            }
            let response = app
                .oneshot(req.body(Body::from(body)).unwrap())
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "expected 400 for body: {body}"
            );
        }
    }

    #[tokio::test]
    async fn search_with_post_body_returns_results() {
        // POST /api/search with a valid q returns 200 with matching results.
        // The folder field in each result is an encrypted ID, not the raw IMAP name.
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");

        let user_hash = crate::auth::user_data::hash_email("alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &user_hash);

        let conn = crate::db::pool::open_user_db(data_dir.path().to_str().unwrap(), &user_hash).unwrap();
        crate::db::folders::upsert_folder(&conn, "INBOX", None, None, "", true, 0, 0, 0, 0).unwrap();
        crate::db::messages::upsert_message(
            &conn, "INBOX", 1, None, None, None, "Budget Report", "finance@example.com", "Finance",
            "[]", "[]", "2024-01-01", 1_704_067_200, "", 1024, false, "", None,
        )
        .unwrap();
        drop(conn);

        let app = create_router(AppServices {
            config,
            store,
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let mut req = Request::builder()
            .method("POST")
            .uri("/api/search")
            .header("x-requested-with", "XMLHttpRequest")
            .header("content-type", "application/json");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(r#"{"q":"Budget"}"#)).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["query"], "Budget");
        let results = json["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);

        // folder_id must be opaque - not the raw IMAP folder name.
        let result_folder_id = results[0]["folder_id"].as_str().unwrap();
        assert_ne!(result_folder_id, "INBOX", "folder_id must be encrypted in search results");
        let decrypted = crate::folder_cipher::FolderCipher::from_session_token(&token)
            .decrypt(&crate::folder_cipher::FolderId::from(result_folder_id))
            .unwrap();
        assert_eq!(decrypted, "INBOX");

        // folder_name is sent alongside it in plaintext, for display.
        assert_eq!(results[0]["folder_name"], "INBOX");
    }

    #[tokio::test]
    async fn search_with_raw_folder_filter_returns_bad_request() {
        // The `folder` field in the POST /api/search body expects an encrypted
        // folder ID from the UI folder picker. Passing a plaintext folder name
        // must return 400 because decryption fails.
        let static_dir = setup_static_dir();
        let data_dir = TempDir::new().unwrap();
        let config = test_config_with_imap(
            static_dir.path().to_str().unwrap(),
            data_dir.path().to_str().unwrap(),
        );
        let store = test_store();
        let (browser_id, account_id, token) = setup_test_account(&store, "alice@example.com");
        provision_user_db(data_dir.path().to_str().unwrap(), &crate::auth::user_data::hash_email("alice@example.com"));

        let app = create_router(AppServices {
            config,
            store,
            search_engine: test_search_engine(data_dir.path().to_str().unwrap()),
            ..test_services("/tmp")
        });

        let mut req = Request::builder()
            .method("POST")
            .uri("/api/search")
            .header("x-requested-with", "XMLHttpRequest")
            .header("content-type", "application/json");
        for (name, value) in auth_headers(&browser_id, &account_id, &token) {
            req = req.header(name, value);
        }
        let response = app
            .oneshot(req.body(Body::from(r#"{"q":"hello","folder":"INBOX"}"#)).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
