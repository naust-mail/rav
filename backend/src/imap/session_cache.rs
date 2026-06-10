use std::collections::HashMap;
use std::sync::Mutex;

use super::connection::{connect, ImapStream};
use super::error::ImapError;
use super::types::ImapCredentials;

/// An authenticated IMAP session over our stream wrapper.
pub type ImapSession = async_imap::Session<ImapStream>;

/// One reusable session per account (email@host).
///
/// Acquiring takes the session out of the slot; releasing puts it back.
/// On error paths, callers simply drop the session rather than calling
/// `release`, ensuring a broken connection is never reused.
pub struct SessionCache {
    slots: Mutex<HashMap<String, ImapSession>>,
}

impl SessionCache {
    pub fn new() -> Self {
        SessionCache {
            slots: Mutex::new(HashMap::new()),
        }
    }

    fn key(creds: &ImapCredentials) -> String {
        format!("{}@{}", creds.email, creds.host)
    }

    /// Take an existing session from the cache, or create a new one.
    ///
    /// The lock is released before the async connect so we never hold a
    /// `std::sync::MutexGuard` across an `.await`.
    pub async fn acquire(&self, creds: &ImapCredentials) -> Result<ImapSession, ImapError> {
        let key = Self::key(creds);
        {
            let mut slots = self.slots.lock().unwrap();
            if let Some(session) = slots.remove(&key) {
                return Ok(session);
            }
        }
        connect(creds).await
    }

    /// Return a healthy session to the cache after a successful operation.
    pub fn release(&self, creds: &ImapCredentials, session: ImapSession) {
        let key = Self::key(creds);
        let mut slots = self.slots.lock().unwrap();
        slots.insert(key, session);
    }
}
