use figment::{Figment, providers::Env};
use serde::Deserialize;

/// Application configuration loaded via figment.
///
/// Layers (lowest to highest priority):
///   1. Serde defaults
///   2. Environment variables (flat, e.g. `PORT`, `IMAP_HOST`)
#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct AppConfig {
    /// Bind address for the HTTP server.
    #[serde(default = "default_host")]
    pub host: String,

    /// Port for the HTTP server.
    #[serde(default = "default_port")]
    pub port: u16,

    /// IMAP server hostname. Required in production.
    #[serde(default)]
    pub imap_host: Option<String>,

    /// IMAP server port.
    #[serde(default = "default_imap_port")]
    pub imap_port: u16,

    /// SMTP server hostname. Required in production.
    #[serde(default)]
    pub smtp_host: Option<String>,

    /// SMTP server port.
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,

    /// Whether TLS is enabled for mail connections.
    #[serde(default = "default_tls_enabled")]
    pub tls_enabled: bool,

    /// Path to a PEM certificate to add to the in-process TLS trust store.
    /// Use this when the mail server uses a self-signed cert not in the system
    /// CA bundle. The cert is trusted only within Rav - no system-wide changes.
    #[serde(default)]
    pub tls_ca_cert_path: Option<String>,

    /// TCP address for IMAP connections. Defaults to imap_host when not set.
    /// Set to 127.0.0.1 on servers that cannot reach their own public IP (hairpin NAT).
    /// The imap_host value is still used for TLS SNI.
    #[serde(default)]
    pub imap_connect_host: Option<String>,

    /// TCP address for SMTP connections. Defaults to smtp_host when not set.
    /// Same rationale as imap_connect_host.
    #[serde(default)]
    pub smtp_connect_host: Option<String>,

    /// Directory for persistent data storage.
    #[serde(default = "default_data_dir")]
    pub data_dir: String,

    /// Session timeout in hours.
    #[serde(default = "default_session_timeout_hours")]
    pub session_timeout_hours: u64,

    /// Directory to serve static frontend files from.
    #[serde(default = "default_static_dir")]
    pub static_dir: String,

    /// Environment the application is running in (development, production)
    #[serde(default = "default_environment")]
    pub environment: String,

    /// Optional base path prefix (e.g. "/rav") for serving behind a reverse proxy subpath.
    #[serde(default)]
    pub base_path: Option<String>,

    /// Base URL for the rspamd HTTP API (e.g. "http://127.0.0.1:11334").
    /// When unset, spam/ham reporting is disabled and messages are still moved
    /// to Junk/Inbox but rspamd is not trained.
    #[serde(default)]
    pub rspamd_url: Option<String>,

    /// Allow users to configure their own mail servers.
    /// If false, IMAP_HOST must be configured and users cannot override mail server settings.
    /// SMTP_HOST is optional and falls back to IMAP_HOST when not set.
    #[serde(default = "default_allow_custom_mail_servers")]
    pub allow_custom_mail_servers: bool,

    /// Rewrite http(s) links in rendered emails to pass through /api/v1/link.
    /// Each link is HMAC-signed to prevent open redirect abuse.
    /// Requires a secret persisted in data_dir/link_proxy.key (auto-generated on first run).
    #[serde(default)]
    pub link_proxy_enabled: bool,

    /// WebAuthn relying party ID (e.g. "box.example.com").
    /// Required for passkey enrollment and authentication. When unset, passkey routes
    /// return 503.
    #[serde(default)]
    pub webauthn_rp_id: Option<String>,

    /// WebAuthn relying party origin URL (e.g. "https://box.example.com").
    /// Must match the browser's origin exactly. Required with webauthn_rp_id.
    #[serde(default)]
    pub webauthn_rp_origin: Option<String>,

    /// Comma-separated list of trusted proxy CIDRs whose forwarded IP headers
    /// are accepted for rate limiting (e.g. "127.0.0.1/32,172.28.0.0/24").
    /// When unset, X-Real-IP and X-Forwarded-For are ignored and the raw socket
    /// peer address is used. Set this to your reverse proxy's IP or subnet when
    /// running behind nginx, Apache, Caddy, or any other proxy.
    #[serde(default)]
    pub trusted_proxies: String,

    /// Enable in-browser PGP (sign, encrypt, verify, decrypt).
    /// Controlled by WEBMAIL_PGP in the box wizard; defaults to true.
    #[serde(default = "default_pgp_enabled")]
    pub pgp_enabled: bool,

    /// ManageSieve server hostname. When set, filter rules are pushed as Sieve scripts
    /// in addition to being stored in SQLite. The IDLE filter application skips rules
    /// that are Sieve-capable when this is configured.
    #[serde(default)]
    pub sieve_host: Option<String>,

    /// ManageSieve server port.
    #[serde(default = "default_sieve_port")]
    pub sieve_port: u16,

    /// Max concurrent SQLite connections held per user in the connection pool.
    #[serde(default = "default_db_pool_max_connections_per_user")]
    pub db_pool_max_connections_per_user: u32,

    /// Seconds a user's connection pool may sit unused before the eviction
    /// sweep drops it. Freed on next access by opening a new pool.
    #[serde(default = "default_db_pool_idle_timeout_secs")]
    pub db_pool_idle_timeout_secs: u64,

    /// Max number of per-user connection pools held in memory at once.
    /// When exceeded, the least-recently-used pool is evicted to make room.
    #[serde(default = "default_db_pool_max_users")]
    pub db_pool_max_users: usize,
}

fn default_pgp_enabled() -> bool {
    true
}

fn default_sieve_port() -> u16 {
    4190
}

fn default_allow_custom_mail_servers() -> bool {
    true
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    3001
}

fn default_imap_port() -> u16 {
    993
}

fn default_smtp_port() -> u16 {
    587
}

fn default_tls_enabled() -> bool {
    true
}

fn default_data_dir() -> String {
    "/data".to_string()
}

fn default_session_timeout_hours() -> u64 {
    24
}

fn default_static_dir() -> String {
    "./static".to_string()
}

fn default_environment() -> String {
    "development".to_string()
}

fn default_db_pool_max_connections_per_user() -> u32 {
    4
}

fn default_db_pool_idle_timeout_secs() -> u64 {
    600
}

fn default_db_pool_max_users() -> usize {
    500
}

impl AppConfig {
    /// Load configuration by layering serde defaults with environment variables.
    ///
    /// Environment variables are read without a prefix and mapped directly to
    /// struct fields via case-insensitive matching (e.g. `IMAP_HOST` → `imap_host`).
    #[allow(clippy::result_large_err)]
    pub fn load() -> Result<Self, figment::Error> {
        Figment::new()
            .merge(Env::raw())
            .extract()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values_load_correctly() {
        // With no env vars set, all defaults should apply.
        // We use figment with no providers (only serde defaults).
        let config: AppConfig = Figment::new().extract().expect("defaults should load");

        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 3001);
        assert!(config.imap_host.is_none());
        assert_eq!(config.imap_port, 993);
        assert!(config.smtp_host.is_none());
        assert_eq!(config.smtp_port, 587);
        assert!(config.tls_enabled);
        assert_eq!(config.data_dir, "/data");
        assert_eq!(config.session_timeout_hours, 24);
        assert_eq!(config.static_dir, "./static");
        assert_eq!(config.environment, "development");
        assert!(config.allow_custom_mail_servers);
        assert_eq!(config.db_pool_max_connections_per_user, 4);
        assert_eq!(config.db_pool_idle_timeout_secs, 600);
        assert_eq!(config.db_pool_max_users, 500);
    }

    #[test]
    fn env_var_overrides_work() {
        // Simulate env overrides via figment's tuple provider
        // to verify all fields are overridable.
        let config: AppConfig = Figment::new()
            .merge(("host", "127.0.0.1"))
            .merge(("port", 8080u16))
            .merge(("imap_host", "mail.example.com"))
            .merge(("imap_port", 143u16))
            .merge(("smtp_host", "smtp.example.com"))
            .merge(("smtp_port", 465u16))
            .merge(("tls_enabled", false))
            .merge(("data_dir", "/var/rav"))
            .merge(("session_timeout_hours", 48u64))
            .merge(("static_dir", "/srv/static"))
            .merge(("environment", "production"))
            .merge(("allow_custom_mail_servers", false))
            .merge(("db_pool_max_connections_per_user", 8u32))
            .merge(("db_pool_idle_timeout_secs", 120u64))
            .merge(("db_pool_max_users", 50usize))
            .extract()
            .expect("overrides should load");

        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert_eq!(config.imap_host.as_deref(), Some("mail.example.com"));
        assert_eq!(config.imap_port, 143);
        assert_eq!(config.smtp_host.as_deref(), Some("smtp.example.com"));
        assert_eq!(config.smtp_port, 465);
        assert!(!config.tls_enabled);
        assert_eq!(config.data_dir, "/var/rav");
        assert_eq!(config.session_timeout_hours, 48);
        assert_eq!(config.static_dir, "/srv/static");
        assert_eq!(config.environment, "production");
        assert!(!config.allow_custom_mail_servers);
        assert_eq!(config.db_pool_max_connections_per_user, 8);
        assert_eq!(config.db_pool_idle_timeout_secs, 120);
        assert_eq!(config.db_pool_max_users, 50);
    }

    #[test]
    fn real_env_vars_override_multi_word_fields() {
        // Verify that actual environment variables with underscores
        // (e.g. IMAP_HOST) correctly map to struct fields (imap_host).
        // SAFETY: This test runs single-threaded (--test-threads=1)
        // and cleans up env vars after use.
        unsafe {
            std::env::set_var("IMAP_HOST", "test-imap.example.com");
            std::env::set_var("DATA_DIR", "/tmp/rav-test");
        }

        let config = AppConfig::load().expect("load with env vars should succeed");

        assert_eq!(config.imap_host.as_deref(), Some("test-imap.example.com"));
        assert_eq!(config.data_dir, "/tmp/rav-test");

        // Clean up
        unsafe {
            std::env::remove_var("IMAP_HOST");
            std::env::remove_var("DATA_DIR");
        }
    }
}
