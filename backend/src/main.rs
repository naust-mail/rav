mod calendar;
mod config;
mod error;
mod db;
mod email_theme;
mod folder_cipher;
mod imap;
mod link_proxy;
mod mail_transport;
mod mfa;
mod sieve;
mod smtp;
mod auth;
mod realtime;
mod routes;
mod search;

use std::sync::Arc;
use std::time::Duration;

use config::AppConfig;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::auth::session::SessionStore;
use crate::imap::client::RealImapClient;
use crate::mail_transport::MailTransport;
use crate::mfa::crypto::MfaCrypto;
use crate::mfa::passkey::PasskeyService;
use crate::routes::AppServices;
use crate::smtp::client::RealSmtpClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize structured JSON logging with env filter.
    // Default to INFO level; override with RUST_LOG env var.
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().json())
        .init();

    // Load configuration via figment (serde defaults + env vars).
    let config = Arc::new(AppConfig::load()?);

    // In locked mode IMAP_HOST must be set - there is no fallback at login time.
    if !config.allow_custom_mail_servers && config.imap_host.is_none() {
        return Err(
            "ALLOW_CUSTOM_MAIL_SERVERS is false but IMAP_HOST is not set. \
             Set IMAP_HOST or enable custom mail servers."
                .into(),
        );
    }

    // Create the in-memory session store with the configured timeout.
    let store = Arc::new(SessionStore::new(Duration::from_secs(
        config.session_timeout_hours * 3600,
    )));

    // Spawn a background task that periodically purges expired sessions.
    {
        let store = Arc::clone(&store);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                interval.tick().await;
                store.purge_expired();
                tracing::debug!("Purged expired sessions");
            }
        });
    }

    // Build transport config once: cert loading, TLS connector, connect addresses.
    let transport = Arc::new(MailTransport::from_config(&config));

    // Create the IMAP and SMTP clients for production use.
    let imap_client: Arc<dyn imap::client::ImapClient> = Arc::new(RealImapClient::new(Arc::clone(&transport)));
    let smtp_client: Arc<dyn smtp::client::SmtpClient> = Arc::new(RealSmtpClient);

    // Shared HTTP client for rspamd and any other outbound HTTP calls.
    let http_client = Arc::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("HTTP client should build"),
    );

    // Create the Tantivy search engine for full-text indexing.
    let search_engine = Arc::new(search::engine::SearchEngine::new(
        std::path::PathBuf::from(&config.data_dir),
    ));

    // Create the real-time event bus and IDLE manager.
    let event_bus = Arc::new(realtime::events::EventBus::new());
    let idle_manager = Arc::new(realtime::idle::IdleManager::new());

    // Load or generate the MFA encryption key from the data directory.
    let mfa_crypto = Arc::new(MfaCrypto::from_data_dir(&config.data_dir)?);

    // If link proxy is enabled, load or generate the signing secret.
    let link_proxy_secret = if config.link_proxy_enabled {
        let secret = link_proxy::load_or_create_secret(&config.data_dir)?;
        Some(Arc::new(link_proxy::LinkProxySecret(secret)))
    } else {
        None
    };

    // Build the passkey service (disabled when RP_ID / RP_ORIGIN are not set).
    let passkey_service = Arc::new(PasskeyService::from_config(&config)?);

    // Spawn background task to purge stale pending ceremonies every 5 minutes.
    {
        let pk_svc = Arc::clone(&passkey_service);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                pk_svc.purge_stale();
            }
        });
    }

    // Build the application router with auth, session, and static file serving.
    let app = routes::create_router(AppServices {
        config: config.clone(),
        transport,
        store,
        imap_client,
        smtp_client,
        http_client,
        search_engine,
        event_bus,
        idle_manager,
        mfa_crypto,
        passkey_service,
        link_proxy_secret,
    });

    // Bind to the configured host and port.
    let bind_addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    tracing::info!(
        host = %config.host,
        port = %config.port,
        static_dir = %config.static_dir,
        "rav-email server starting"
    );

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;

    Ok(())
}
