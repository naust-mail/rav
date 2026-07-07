/// Pre-built transport configuration for IMAP and SMTP connections.
///
/// Built once at startup from AppConfig. Routes and clients extract this from
/// axum extensions instead of handling certs or connection addresses themselves.
/// All TLS cert loading, trust store setup, and connect-address resolution
/// happens here so nothing leaks into HTTP or business logic layers.
use lettre::transport::smtp::client::TlsParameters;

use crate::config::AppConfig;

pub struct MailTransport {
    /// Pre-built TLS connector for IMAP. Includes any custom CA cert.
    /// Used by RealImapClient for all server connections.
    pub imap_connector: async_native_tls::TlsConnector,

    /// TCP address for IMAP connections. May differ from the IMAP hostname
    /// when the server cannot reach its own public IP (hairpin NAT).
    /// The IMAP hostname is still used for TLS SNI.
    pub imap_connect_host: String,

    /// TCP address for SMTP connections. Same hairpin NAT rationale.
    /// The SMTP hostname is still used for TLS SNI and Message-ID.
    pub smtp_connect_host: String,

    /// Pre-built TLS parameters for SMTP. Includes any custom CA cert.
    /// None when SMTP host is not configured or TLS params fail to build.
    pub smtp_tls_params: Option<TlsParameters>,
}

impl MailTransport {
    pub fn from_config(config: &AppConfig) -> Self {
        // Load extra CA cert from disk once. Warn and continue with system
        // roots if the path is missing or the cert is malformed.
        let extra_cert: Option<native_tls::Certificate> =
            config.tls_ca_cert_path.as_ref().and_then(|path| {
                match std::fs::read(path) {
                    Ok(pem) => match native_tls::Certificate::from_pem(&pem) {
                        Ok(cert) => Some(cert),
                        Err(e) => {
                            tracing::warn!(%path, error = %e, "TLS_CA_CERT_PATH cert is invalid, falling back to system roots");
                            None
                        }
                    },
                    Err(e) => {
                        tracing::warn!(%path, error = %e, "Cannot read TLS_CA_CERT_PATH, falling back to system roots");
                        None
                    }
                }
            });

        // Build the IMAP TLS connector.
        // async_native_tls::TlsConnector wraps a TlsConnectorBuilder (not a built connector),
        // so we convert the builder directly via From<TlsConnectorBuilder>.
        let imap_connector = {
            let mut builder = native_tls::TlsConnector::builder();
            if let Some(ref cert) = extra_cert {
                builder.add_root_certificate(cert.clone());
            }
            builder.into()
        };

        let imap_host = config.imap_host.as_deref().unwrap_or("localhost");
        let smtp_host_ref = config.smtp_host.as_deref().unwrap_or(imap_host);

        let imap_connect_host = config
            .imap_connect_host
            .clone()
            .unwrap_or_else(|| imap_host.to_string());

        let smtp_connect_host = config
            .smtp_connect_host
            .clone()
            .unwrap_or_else(|| smtp_host_ref.to_string());

        // Build SMTP TLS params, adding the same CA cert if provided.
        // lettre has its own Certificate wrapper distinct from native_tls::Certificate,
        // so we re-parse from the raw PEM bytes.
        let smtp_ca_cert: Option<lettre::transport::smtp::client::Certificate> =
            config.tls_ca_cert_path.as_ref().and_then(|path| {
                std::fs::read(path).ok().and_then(|pem| {
                    lettre::transport::smtp::client::Certificate::from_pem(&pem).ok()
                })
            });

        let smtp_tls_params = config.smtp_host.as_deref().and_then(|smtp_host| {
            let mut builder = TlsParameters::builder(smtp_host.to_string());
            if let Some(cert) = smtp_ca_cert {
                builder = builder.add_root_certificate(cert);
            }
            match builder.build_native() {
                Ok(params) => Some(params),
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to build SMTP TLS params, falling back to lettre defaults");
                    None
                }
            }
        });

        Self {
            imap_connector,
            imap_connect_host,
            smtp_connect_host,
            smtp_tls_params,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use std::io::Write;

    fn base_config() -> AppConfig {
        AppConfig {
            host: "127.0.0.1".to_string(),
            port: 3001,
            imap_host: Some("mail.example.com".to_string()),
            imap_port: 993,
            smtp_host: Some("mail.example.com".to_string()),
            smtp_port: 587,
            tls_enabled: true,
            tls_ca_cert_path: None,
            imap_connect_host: None,
            smtp_connect_host: None,
            data_dir: "/tmp".to_string(),
            session_timeout_hours: 24,
            static_dir: "/tmp".to_string(),
            environment: "test".to_string(),
            base_path: None,
            allow_custom_mail_servers: false,
            rspamd_url: None,
            link_proxy_enabled: false,
            pgp_enabled: true,
            webauthn_rp_id: None,
            webauthn_rp_origin: None,
            trusted_proxies: String::new(),
            sieve_host: None,
            sieve_port: 4190,
        }
    }

    #[test]
    fn connect_host_defaults_to_imap_host() {
        let t = MailTransport::from_config(&base_config());
        assert_eq!(t.imap_connect_host, "mail.example.com");
        assert_eq!(t.smtp_connect_host, "mail.example.com");
    }

    #[test]
    fn connect_host_override_takes_precedence() {
        let mut config = base_config();
        config.imap_connect_host = Some("127.0.0.1".to_string());
        config.smtp_connect_host = Some("127.0.0.1".to_string());
        let t = MailTransport::from_config(&config);
        assert_eq!(t.imap_connect_host, "127.0.0.1");
        assert_eq!(t.smtp_connect_host, "127.0.0.1");
    }

    #[test]
    fn no_smtp_host_gives_none_tls_params() {
        let mut config = base_config();
        config.smtp_host = None;
        let t = MailTransport::from_config(&config);
        assert!(t.smtp_tls_params.is_none());
    }

    #[test]
    fn smtp_host_set_gives_some_tls_params() {
        let t = MailTransport::from_config(&base_config());
        assert!(t.smtp_tls_params.is_some());
    }

    #[test]
    fn missing_cert_path_does_not_panic() {
        let mut config = base_config();
        config.tls_ca_cert_path = Some("/nonexistent/cert.pem".to_string());
        // Should not panic, falls back gracefully
        let t = MailTransport::from_config(&config);
        // SMTP params still built (without the extra cert)
        assert!(t.smtp_tls_params.is_some());
    }

    #[test]
    fn invalid_cert_content_does_not_panic() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "this is not a pem certificate").unwrap();
        let mut config = base_config();
        config.tls_ca_cert_path = Some(file.path().to_str().unwrap().to_string());
        // Should not panic
        let _ = MailTransport::from_config(&config);
    }
}
