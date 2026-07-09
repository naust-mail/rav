pub mod attachments;
pub mod auth;
pub mod mfa;
pub mod calendar;
#[cfg(feature = "stickers")]
pub mod stickers;
pub mod contact_groups;
pub mod contacts;
pub mod display_preferences;
pub mod drafts;
pub mod filters;
pub mod folder_mgmt;
pub mod folders;
pub mod health;
pub mod identities;
pub mod link_proxy;
pub mod messages;
pub mod notification_preferences;
pub mod quota;
pub mod search;
pub mod pgp;
pub mod send;
pub mod spam;
pub mod tags;
pub mod vacation;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ConnectInfo;
use axum::http::Request;
use axum::routing::{delete, get, patch, post, put};
use axum::{Extension, Router, middleware};
use ipnet::IpNet;
use tower_governor::GovernorError;
use tower_governor::GovernorLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::auth::csrf::csrf_protection;
use crate::auth::middleware::auth_guard;
use crate::auth::session::SessionStore;
use crate::config::AppConfig;
use crate::imap::client::ImapClient;
use crate::mail_transport::MailTransport;
use crate::link_proxy::LinkProxySecret;
use crate::mfa::crypto::MfaCrypto;
use crate::mfa::passkey::PasskeyService;
use crate::realtime::events::EventBus;
use crate::realtime::idle::IdleManager;
use crate::smtp::client::SmtpClient;

/// Parse a comma-separated list of CIDR strings into IpNet values.
///
/// Bare addresses without a prefix length (e.g. "127.0.0.1") are accepted and
/// normalized to host networks (/32 for IPv4, /128 for IPv6). Entries that are
/// neither valid CIDRs nor valid IP addresses are logged and skipped so a typo
/// degrades gracefully (falls back to socket IP) rather than crashing startup.
fn parse_trusted_proxies(raw: &str) -> Vec<IpNet> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| {
            if let Ok(net) = s.parse::<IpNet>() {
                return Some(net);
            }
            if let Ok(addr) = s.parse::<IpAddr>() {
                return Some(IpNet::from(addr));
            }
            tracing::warn!(entry = s, "TRUSTED_PROXIES: unrecognised entry, skipping");
            None
        })
        .collect()
}

/// Per-IP key extractor for rate limiting.
///
/// X-Real-IP and X-Forwarded-For headers are only trusted when the TCP
/// connection came from a known proxy address (configured via TRUSTED_PROXIES).
/// When no trusted proxies are configured (standalone mode), or the connecting
/// IP is not in the trusted set, the raw socket peer address is used.
///
/// This prevents a client from spoofing their IP by setting the header
/// themselves before the request reaches the proxy.
#[derive(Debug, Clone)]
struct RealIpKeyExtractor {
    trusted: Vec<IpNet>,
}

impl KeyExtractor for RealIpKeyExtractor {
    type Key = IpAddr;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        let socket_ip = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci: &ConnectInfo<SocketAddr>| ci.0.ip());

        if let Some(ip) = socket_ip
            && self.trusted.iter().any(|net| net.contains(&ip))
        {
            if let Some(real_ip) = req
                .headers()
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<IpAddr>().ok())
            {
                return Ok(real_ip);
            }
            if let Some(xff_ip) = req
                .headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(',').next())
                .and_then(|s| s.trim().parse::<IpAddr>().ok())
            {
                return Ok(xff_ip);
            }
        }

        Ok(socket_ip.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)))
    }
}

/// All application-level services passed into the router at startup.
///
/// Add new services here rather than adding more parameters to `create_router`.
/// Tests build an `AppServices` with `test_services(...)` and override only
/// the fields they care about using struct update syntax.
pub struct AppServices {
    pub config: Arc<AppConfig>,
    pub transport: Arc<MailTransport>,
    pub store: Arc<SessionStore>,
    pub imap_client: Arc<dyn ImapClient>,
    pub smtp_client: Arc<dyn SmtpClient>,
    pub http_client: Arc<reqwest::Client>,
    pub search_engine: Arc<crate::search::engine::SearchEngine>,
    pub event_bus: Arc<EventBus>,
    pub idle_manager: Arc<IdleManager>,
    pub mfa_crypto: Arc<MfaCrypto>,
    pub passkey_service: Arc<PasskeyService>,
    pub link_proxy_secret: Option<Arc<LinkProxySecret>>,
}

pub fn create_router(svc: AppServices) -> Router {
    let AppServices {
        config,
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
    } = svc;
    let key_extractor = RealIpKeyExtractor {
        trusted: parse_trusted_proxies(&config.trusted_proxies),
    };

    // Rate-limit login routes: replenish 1 token every 12 s, burst of 5.
    let passkey_governor = GovernorConfigBuilder::default()
        .key_extractor(key_extractor.clone())
        .period(Duration::from_secs(12))
        .burst_size(5)
        .finish()
        .expect("valid governor config");

    let login_governor = GovernorConfigBuilder::default()
        .key_extractor(key_extractor)
        .period(Duration::from_secs(12))
        .burst_size(5)
        .finish()
        .expect("valid governor config");

    // Public passkey login routes (rate-limited + CSRF, no auth_guard).
    let public_passkey = Router::new()
        .route("/mfa/passkey/login/begin", post(mfa::passkey_login_begin))
        .route("/mfa/passkey/login/complete", post(mfa::passkey_login_complete))
        .layer(middleware::from_fn(csrf_protection))
        .layer(GovernorLayer::new(passkey_governor));

    // Public auth route: GovernorLayer (outermost) -> CSRF -> handler.
    let public_auth = Router::new()
        .route("/login", post(auth::login))
        .layer(middleware::from_fn(csrf_protection))
        .layer(GovernorLayer::new(login_governor));

    // Browser-bound routes (no auth_guard, no rate limit).
    // These routes only need the browser cookie, not full auth.
    let browser_routes = Router::new()
        .route("/accounts", get(auth::list_accounts))
        .route("/accounts/{id}", delete(auth::remove_account).layer(middleware::from_fn(csrf_protection)));

    // Protected auth routes (auth_guard + CSRF).
    let protected_auth = Router::new()
        .route("/session", get(auth::get_session))
        .route("/logout", post(auth::logout))
        .layer(middleware::from_fn(auth_guard))
        .layer(middleware::from_fn(csrf_protection));

    let auth_router = Router::new()
        .merge(public_passkey)
        .merge(public_auth)
        .merge(browser_routes)
        .merge(protected_auth);

    // Protected data routes (auth_guard + CSRF).
    let protected_data = Router::new()
        .route(
            "/folders",
            get(folders::list_folders).post(folder_mgmt::create_folder),
        )
        .route(
            "/folders/{name}",
            patch(folder_mgmt::rename_folder).delete(folder_mgmt::delete_folder),
        )
        .route(
            "/folders/{name}/subscribe",
            patch(folder_mgmt::subscribe_folder),
        )
        .route("/folders/{folder}/messages", get(messages::list_messages))
        .route("/folders/{folder}/mark-all-read", post(messages::mark_all_read))
        .route("/messages/by-message-id", post(messages::get_message_by_message_id))
        .route("/messages/{folder}/{uid}/report-spam", post(spam::report_spam_handler))
        .route("/messages/{folder}/{uid}/report-ham", post(spam::report_ham_handler))
        .route("/messages/{folder}/{uid}", get(messages::get_message))
        .route(
            "/messages/{folder}/{uid}/flags",
            patch(messages::update_flags),
        )
        .route(
            "/messages/{folder}/{uid}/attachments/{attachment_id}",
            get(messages::download_attachment),
        )
        .route("/messages/move", post(messages::move_message_handler))
        .route("/messages/send", post(send::send_message_handler))
        .route(
            "/messages/{folder}/{uid}",
            delete(messages::delete_message_handler),
        )
        .route("/drafts/reply-for", post(drafts::get_reply_draft_handler))
        .route("/drafts/{uuid}", post(drafts::save_draft_handler))
        .route("/drafts/{uuid}", delete(drafts::delete_draft_handler))
        .route(
            "/drafts/{uuid}/attachments",
            post(attachments::upload_attachment).get(attachments::list_attachments),
        )
        .route(
            "/drafts/{uuid}/attachments/{attachment_id}",
            delete(attachments::delete_attachment),
        )
        .route(
            "/drafts/{uuid}/attachments/{attachment_id}/content",
            get(attachments::get_attachment_content),
        )
        .route("/search", post(search::search_messages))
        .route(
            "/contact-groups",
            get(contact_groups::list_groups_handler).post(contact_groups::create_group_handler),
        )
        .route(
            "/contact-groups/{id}",
            put(contact_groups::update_group_handler).delete(contact_groups::delete_group_handler),
        )
        .route(
            "/contact-groups/{id}/members",
            get(contact_groups::list_members_handler).post(contact_groups::add_member_handler),
        )
        .route(
            "/contact-groups/{id}/members/{contact_id}",
            delete(contact_groups::remove_member_handler),
        )
        .route(
            "/tags",
            get(tags::list_tags_handler).post(tags::create_tag_handler),
        )
        .route(
            "/tags/{id}",
            put(tags::update_tag_handler).delete(tags::delete_tag_handler),
        )
        .route(
            "/tags/{id}/messages",
            post(tags::tag_message_handler).get(tags::list_tag_messages_handler),
        )
        .route(
            "/tags/{id}/messages/bulk",
            post(tags::bulk_tag_handler),
        )
        .route(
            "/tags/{id}/messages/{folder}/{uid}",
            delete(tags::untag_message_handler),
        )
        .route(
            "/messages/{folder}/{uid}/tags",
            get(tags::get_message_tags_handler),
        )
        .route(
            "/contacts",
            get(contacts::list_contacts_handler).post(contacts::create_contact_handler),
        )
        .route("/contacts/export", get(contacts::export_contacts_handler))
        .route("/contacts/import", post(contacts::import_contacts_handler))
        .route(
            "/contacts/autocomplete/all",
            get(contacts::autocomplete_all_handler),
        )
        .route(
            "/contacts/autocomplete",
            get(contacts::autocomplete_handler),
        )
        .route(
            "/contacts/{id}",
            get(contacts::get_contact_handler).delete(contacts::delete_contact_handler),
        )
        .route(
            "/contacts/{id}/export",
            get(contacts::export_single_contact_handler),
        )
        .route(
            "/identities",
            get(identities::list_identities_handler).post(identities::create_identity_handler),
        )
        .route(
            "/identities/{id}",
            get(identities::get_identity_handler)
                .put(identities::update_identity_handler)
                .delete(identities::delete_identity_handler),
        )
        .route(
            "/settings/display",
            get(display_preferences::get_display_preferences)
                .put(display_preferences::update_display_preferences),
        )
        .route(
            "/settings/notifications",
            get(notification_preferences::get_notification_preferences)
                .put(notification_preferences::update_notification_preferences),
        )
        .route(
            "/calendar/events",
            get(calendar::list_events).post(calendar::create_event),
        )
        .route(
            "/calendar/events/import-ics",
            post(calendar::import_ics),
        )
        .route(
            "/calendar/events/{id}",
            get(calendar::get_event)
                .put(calendar::update_event)
                .delete(calendar::delete_event),
        )
        .route(
            "/calendar/settings",
            get(calendar::get_calendar_settings)
                .put(calendar::update_calendar_settings),
        )
        .route(
            "/calendar/meeting-templates",
            get(calendar::list_meeting_templates)
                .post(calendar::create_meeting_template),
        )
        .route(
            "/calendar/meeting-templates/{id}",
            delete(calendar::delete_meeting_template),
        )
        .route("/filters", get(filters::list_filters_handler).post(filters::create_filter_handler))
        .route("/filters/reorder", put(filters::reorder_filters_handler))
        .route("/filters/apply", post(filters::apply_filters_handler))
        .route("/filters/{id}", put(filters::update_filter_handler).delete(filters::delete_filter_handler))
        .route("/settings/vacation", get(vacation::get_vacation_handler).put(vacation::update_vacation_handler))
        .route("/quota", get(quota::get_quota))
        .route("/pgp/keys", get(pgp::list_keys).post(pgp::store_key))
        .route("/pgp/keys/{id}", get(pgp::get_key).delete(pgp::delete_key))
        .route("/pgp/keys/{id}/identity", put(pgp::assign_identity))
        .route("/pgp/wkd", get(pgp::wkd_lookup))
        .route("/mfa/status", get(mfa::status))
        .route("/mfa/totp/setup", post(mfa::totp_setup))
        .route("/mfa/totp/confirm", post(mfa::totp_confirm))
        .route("/mfa/totp", delete(mfa::totp_delete))
        .route("/mfa/passkey/register/begin", post(mfa::passkey_register_begin))
        .route("/mfa/passkey/register/complete", post(mfa::passkey_register_complete))
        .route("/mfa/passkeys", get(mfa::passkey_list))
        .route("/mfa/passkeys/{id}", delete(mfa::passkey_delete))
        .route("/mfa/settings/passkey-only", put(mfa::passkey_only_set))
        .route("/link", get(link_proxy::redirect))
        .layer(middleware::from_fn(auth_guard))
        .layer(middleware::from_fn(csrf_protection));

    // WebSocket route — auth is handled inside the handler via cookie,
    // so it bypasses CSRF and auth_guard middleware.
    let ws_route = Router::new()
        .route("/ws", get(crate::realtime::ws::ws_handler));

    #[cfg(feature = "stickers")]
    let sticker_routes = Router::new()
        .route("/calendar/stickers", get(stickers::list_stickers))
        .route(
            "/calendar/stickers/{date}",
            put(stickers::put_sticker).delete(stickers::delete_sticker),
        )
        .layer(middleware::from_fn(auth_guard))
        .layer(middleware::from_fn(csrf_protection));

    let api_router = Router::new()
        .route("/health", get(health::health_check))
        .nest("/auth", auth_router)
        .merge(ws_route)
        .merge(protected_data);

    #[cfg(feature = "stickers")]
    let api_router = api_router.merge(sticker_routes);

    let index_path = Path::new(&config.static_dir).join("index.html");
    let static_service = ServeDir::new(&config.static_dir).fallback(ServeFile::new(index_path));

    let inner = Router::new()
        .nest("/api", api_router)
        .fallback_service(static_service);

    // If BASE_PATH is set (e.g. "/rav"), nest the entire app under that prefix.
    let router = match config.base_path.as_deref() {
        Some(bp) if !bp.is_empty() => Router::new().nest(bp, inner),
        _ => inner,
    };

    let mut router = router
        .layer(Extension(idle_manager))
        .layer(Extension(event_bus))
        .layer(Extension(smtp_client))
        .layer(Extension(http_client))
        .layer(Extension(search_engine))
        .layer(Extension(imap_client))
        .layer(Extension(store))
        .layer(Extension(transport))
        .layer(Extension(mfa_crypto))
        .layer(Extension(passkey_service))
        .layer(Extension(config.clone()))
        .layer(TraceLayer::new_for_http());

    if let Some(secret) = link_proxy_secret {
        router = router.layer(Extension(secret));
    }

    if config.environment == "development" {
        router.layer(CorsLayer::permissive())
    } else {
        router
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod extractor_tests {
    use super::*;
    use std::net::SocketAddr;

    fn make_req(socket: Option<&str>, headers: &[(&str, &str)]) -> axum::http::Request<()> {
        let mut builder = axum::http::Request::builder();
        for (name, val) in headers {
            builder = builder.header(*name, *val);
        }
        let mut req = builder.body(()).unwrap();
        if let Some(s) = socket {
            let addr: SocketAddr = s.parse().unwrap();
            req.extensions_mut().insert(ConnectInfo(addr));
        }
        req
    }

    fn trusted() -> RealIpKeyExtractor {
        RealIpKeyExtractor {
            trusted: parse_trusted_proxies("10.0.0.1/32,172.28.0.0/24"),
        }
    }

    fn untrusted() -> RealIpKeyExtractor {
        RealIpKeyExtractor { trusted: vec![] }
    }

    // --- extractor behaviour ---

    #[test]
    fn untrusted_socket_ignores_x_real_ip() {
        let ip = trusted().extract(&make_req(Some("1.2.3.4:1234"), &[("x-real-ip", "9.9.9.9")])).unwrap();
        assert_eq!(ip, "1.2.3.4".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn trusted_socket_uses_x_real_ip() {
        let ip = trusted().extract(&make_req(Some("10.0.0.1:1234"), &[("x-real-ip", "203.0.113.5")])).unwrap();
        assert_eq!(ip, "203.0.113.5".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn trusted_socket_falls_back_to_xff_when_no_x_real_ip() {
        let ip = trusted().extract(&make_req(Some("172.28.0.3:1234"), &[("x-forwarded-for", "203.0.113.5, 172.28.0.3")])).unwrap();
        assert_eq!(ip, "203.0.113.5".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn trusted_socket_no_headers_uses_socket_ip() {
        let ip = trusted().extract(&make_req(Some("10.0.0.1:1234"), &[])).unwrap();
        assert_eq!(ip, "10.0.0.1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn no_connect_info_falls_back_to_loopback() {
        let ip = untrusted().extract(&make_req(None, &[("x-real-ip", "9.9.9.9")])).unwrap();
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn empty_trusted_list_always_uses_socket_ip() {
        let ip = untrusted().extract(&make_req(Some("5.6.7.8:1234"), &[("x-real-ip", "9.9.9.9")])).unwrap();
        assert_eq!(ip, "5.6.7.8".parse::<IpAddr>().unwrap());
    }

    // --- parse_trusted_proxies ---

    #[test]
    fn parse_valid_cidrs() {
        let nets = parse_trusted_proxies("127.0.0.1/32,172.28.0.0/24");
        assert_eq!(nets.len(), 2);
    }

    #[test]
    fn parse_bare_ipv4_normalises_to_host() {
        let nets = parse_trusted_proxies("127.0.0.1");
        assert_eq!(nets.len(), 1);
        assert!(nets[0].contains(&"127.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(!nets[0].contains(&"127.0.0.2".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn parse_bare_ipv6_normalises_to_host() {
        let nets = parse_trusted_proxies("::1");
        assert_eq!(nets.len(), 1);
        assert!(nets[0].contains(&"::1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn parse_invalid_entry_skipped_valid_kept() {
        let nets = parse_trusted_proxies("not-an-ip,127.0.0.1/32");
        assert_eq!(nets.len(), 1);
    }

    #[test]
    fn parse_empty_string_returns_empty() {
        assert!(parse_trusted_proxies("").is_empty());
    }

    #[test]
    fn parse_whitespace_entries_ignored() {
        assert!(parse_trusted_proxies("  ,  ,  ").is_empty());
    }
}
