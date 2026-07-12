use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::Semaphore;

use super::connection::{connect, ImapStream};
use super::error::ImapError;
use super::types::ImapCredentials;

/// An authenticated IMAP session over our stream wrapper.
pub type ImapSession = async_imap::Session<ImapStream>;

/// Max number of brand-new IMAP connections (TCP+TLS+LOGIN) allowed to be
/// opened concurrently for a single account. Only one cached slot exists per
/// account, so any request that doesn't win it falls through to `connect()`;
/// without a cap, a burst of concurrent requests (e.g. an unbatched bulk
/// action over hundreds of messages) opens one connection per request all at
/// once, which is what actually OOM-killed the process.
const MAX_CONCURRENT_CONNECTS_PER_ACCOUNT: usize = 4;

/// One reusable session per account (email@host).
///
/// Acquiring takes the session out of the slot; releasing puts it back.
/// On error paths, callers simply drop the session rather than calling
/// `release`, ensuring a broken connection is never reused.
pub struct SessionCache {
    slots: Mutex<HashMap<String, ImapSession>>,
    connect_limits: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl SessionCache {
    pub fn new() -> Self {
        SessionCache {
            slots: Mutex::new(HashMap::new()),
            connect_limits: Mutex::new(HashMap::new()),
        }
    }

    fn key(creds: &ImapCredentials) -> String {
        format!("{}@{}", creds.email, creds.host)
    }

    fn connect_limit(&self, key: &str) -> Arc<Semaphore> {
        let mut limits = self.connect_limits.lock().unwrap();
        limits
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTS_PER_ACCOUNT)))
            .clone()
    }

    /// Take an existing session from the cache, or create a new one.
    ///
    /// The lock is released before the async connect so we never hold a
    /// `std::sync::MutexGuard` across an `.await`.
    pub async fn acquire(
        &self,
        creds: &ImapCredentials,
        connect_host: &str,
        tls_connector: &async_native_tls::TlsConnector,
    ) -> Result<ImapSession, ImapError> {
        let key = Self::key(creds);
        {
            let mut slots = self.slots.lock().unwrap();
            if let Some(session) = slots.remove(&key) {
                return Ok(session);
            }
        }
        // Bound how many callers can open a fresh connection for this
        // account at once; excess callers queue here instead of all
        // connecting simultaneously.
        let limiter = self.connect_limit(&key);
        let _permit = limiter
            .acquire_owned()
            .await
            .expect("connect semaphore is never closed");
        connect(creds, connect_host, tls_connector).await
    }

    /// Return a healthy session to the cache after a successful operation.
    pub fn release(&self, creds: &ImapCredentials, session: ImapSession) {
        let key = Self::key(creds);
        let mut slots = self.slots.lock().unwrap();
        slots.insert(key, session);
    }
}
