mod client;
mod generator;

pub use generator::is_sieve_capable;

use std::sync::Arc;

use crate::config::AppConfig;
use crate::db::filters::FilterRule;

/// Push all rules to ManageSieve if sieve_host is configured. Best-effort: logs on failure.
pub async fn push_filters(config: &Arc<AppConfig>, email: &str, password: &str, rules: &[FilterRule]) {
    let Some(ref host) = config.sieve_host else { return; };
    let script = generator::generate_sieve_script(rules);
    if let Err(e) = client::push_script(host, config.sieve_port, email, password, "rav-filters", &script).await {
        tracing::warn!(error = %e, "ManageSieve push failed - filters will apply via IDLE only");
    }
}
