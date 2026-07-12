use std::collections::HashMap;

use serde::Serialize;
use tokio::sync::{broadcast, RwLock};

/// Events that can be pushed to connected WebSocket clients.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
#[allow(dead_code)]
pub enum MailEvent {
    /// New messages were written to the cache for a folder. Cache-only
    /// signal (fired by `run_sync` as soon as it fetches+stores new
    /// headers) - not filtered/vacation-processed yet, so not suitable
    /// for user-facing notifications. Frontend uses this purely to
    /// invalidate its query cache.
    FolderStateChanged {
        folder: String,
        count: u32,
        latest_sender: Option<String>,
        latest_subject: Option<String>,
    },
    /// New mail actually landed in INBOX for the user to see, after
    /// filter rules and the vacation responder have run. Only covers
    /// messages that survived filtering (weren't moved/deleted by a
    /// rule) - this is what drives desktop/toast notifications.
    NewMail {
        folder: String,
        count: u32,
        latest_sender: Option<String>,
        latest_subject: Option<String>,
    },
    /// Flags changed on messages in a folder.
    FlagsChanged { folder: String },
    /// Folder list or counts changed. `folder` is set when a specific folder was synced.
    FolderUpdated { folder: Option<String> },
    /// An outbox entry moved to a new state (scheduled -> sending -> sent | failed).
    /// `sent` entries are deleted right after this fires, so the frontend
    /// treats `sent` as "remove from the outbox list", not a fetchable state.
    OutboxStateChanged {
        id: String,
        state: String,
        fail_reason: Option<String>,
    },
}

/// Fan-out event bus: per-user broadcast channels.
///
/// Each user (keyed by `user_hash`) gets a `broadcast::Sender` that can have
/// multiple receivers (one per WebSocket connection). When all receivers are
/// dropped the channel is cleaned up.
pub struct EventBus {
    channels: RwLock<HashMap<String, broadcast::Sender<MailEvent>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
        }
    }

    /// Subscribe to events for a user. Creates the channel if it doesn't exist.
    /// Returns a receiver that will receive all future events for this user.
    pub async fn subscribe(&self, user_hash: &str) -> broadcast::Receiver<MailEvent> {
        // Fast path: read lock.
        {
            let channels = self.channels.read().await;
            if let Some(tx) = channels.get(user_hash) {
                return tx.subscribe();
            }
        }

        // Slow path: write lock to insert.
        let mut channels = self.channels.write().await;
        let tx = channels
            .entry(user_hash.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(64);
                tx
            });
        tx.subscribe()
    }

    /// Publish an event to all connected clients for a user.
    /// No-op if no one is listening.
    pub async fn publish(&self, user_hash: &str, event: MailEvent) {
        let channels = self.channels.read().await;
        if let Some(tx) = channels.get(user_hash) {
            // Ignore error (no receivers connected).
            let _ = tx.send(event);
        }
    }

    /// Remove a user's channel when no more connections exist.
    /// Called when the last WebSocket for a user disconnects.
    pub async fn cleanup(&self, user_hash: &str) {
        let mut channels = self.channels.write().await;
        if let Some(tx) = channels.get(user_hash) {
            // Only remove if no active receivers remain.
            if tx.receiver_count() == 0 {
                channels.remove(user_hash);
            }
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn subscribe_and_publish() {
        let bus = Arc::new(EventBus::new());
        let mut rx: tokio::sync::broadcast::Receiver<MailEvent> = bus.subscribe("user1").await;

        bus.publish("user1", MailEvent::FolderStateChanged {
            folder: "INBOX".to_string(),
            count: 1,
            latest_sender: Some("alice@example.com".to_string()),
            latest_subject: Some("Hello".to_string()),
        }).await;

        let event: MailEvent = rx.recv().await.unwrap();
        match event {
            MailEvent::FolderStateChanged { folder, count, .. } => {
                assert_eq!(folder, "INBOX");
                assert_eq!(count, 1);
            }
            _ => panic!("unexpected event type"),
        }
    }

    #[tokio::test]
    async fn publish_to_nonexistent_user_is_noop() {
        let bus = EventBus::new();
        // Should not panic.
        bus.publish("ghost", MailEvent::FolderUpdated { folder: None }).await;
    }

    #[tokio::test]
    async fn multiple_subscribers_receive_same_event() {
        let bus = Arc::new(EventBus::new());
        let mut rx1: tokio::sync::broadcast::Receiver<MailEvent> = bus.subscribe("user1").await;
        let mut rx2: tokio::sync::broadcast::Receiver<MailEvent> = bus.subscribe("user1").await;

        bus.publish("user1", MailEvent::FolderUpdated { folder: None }).await;

        let e1: MailEvent = rx1.recv().await.unwrap();
        assert!(matches!(e1, MailEvent::FolderUpdated { .. }));
        let e2: MailEvent = rx2.recv().await.unwrap();
        assert!(matches!(e2, MailEvent::FolderUpdated { .. }));
    }

    #[tokio::test]
    async fn cleanup_removes_empty_channel() {
        let bus = EventBus::new();
        let rx: tokio::sync::broadcast::Receiver<MailEvent> = bus.subscribe("user1").await;
        drop(rx);

        bus.cleanup("user1").await;

        let channels = bus.channels.read().await;
        assert!(!channels.contains_key("user1"));
    }

    #[tokio::test]
    async fn cleanup_keeps_channel_with_active_subscribers() {
        let bus = EventBus::new();
        let _rx: tokio::sync::broadcast::Receiver<MailEvent> = bus.subscribe("user1").await;

        bus.cleanup("user1").await;

        let channels = bus.channels.read().await;
        assert!(channels.contains_key("user1"));
    }
}
