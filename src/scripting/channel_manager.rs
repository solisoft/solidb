//! Channel Manager for WebSocket Pub/Sub and Presence Tracking
//!
//! This module provides shared state for WebSocket channel subscriptions
//! and presence tracking across all Lua scripts.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::RwLock;
use tokio::sync::{broadcast, mpsc};

/// Unique identifier for a WebSocket connection
pub type ConnectionId = String;

/// Message broadcast to channel subscribers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    pub channel: String,
    pub data: JsonValue,
    pub sender_id: Option<ConnectionId>,
    pub timestamp: i64,
}

/// Presence change event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceEvent {
    pub event_type: PresenceEventType,
    pub channel: String,
    pub user_info: JsonValue,
    pub connection_id: ConnectionId,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceEventType {
    Join,
    Leave,
    Update,
}

impl std::fmt::Display for PresenceEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PresenceEventType::Join => write!(f, "join"),
            PresenceEventType::Leave => write!(f, "leave"),
            PresenceEventType::Update => write!(f, "update"),
        }
    }
}

/// Events delivered to individual connections
#[derive(Debug, Clone)]
pub enum ChannelEvent {
    Message(ChannelMessage),
    Presence(PresenceEvent),
}

/// Information about a user's presence in a channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceInfo {
    pub connection_id: ConnectionId,
    pub user_info: JsonValue,
    pub joined_at: i64,
}

/// State for a single channel
struct ChannelState {
    /// Broadcast sender for channel messages
    message_tx: broadcast::Sender<ChannelMessage>,
    /// Presence information for users in this channel
    presence: HashMap<ConnectionId, PresenceInfo>,
    /// Connections subscribed to this channel. A set, not a counter:
    /// subscribing twice used to count twice while unregistering
    /// decremented once, so such a channel (and its 1000-slot ring) was
    /// never freed (audit M6).
    subscribers: HashSet<ConnectionId>,
    /// Created timestamp
    #[allow(dead_code)]
    created_at: i64,
}

impl ChannelState {
    fn new(stats: &ChannelStats) -> Self {
        let (tx, _) = broadcast::channel(1000);
        stats.total_channels_created.fetch_add(1, Ordering::SeqCst);
        stats.active_channels.fetch_add(1, Ordering::SeqCst);
        Self {
            message_tx: tx,
            presence: HashMap::new(),
            subscribers: HashSet::new(),
            created_at: chrono::Utc::now().timestamp_millis(),
        }
    }

    fn is_unused(&self) -> bool {
        self.subscribers.is_empty() && self.presence.is_empty()
    }
}

/// Channels are keyed by `(database, name)`: the manager is shared by every
/// database on the instance, and a bare name let one tenant's script listen
/// to or inject into another tenant's `"chat"` (audit H6). A connection's
/// database is fixed when it registers, and every operation on a
/// connection resolves channel names inside it.
type ChannelKey = (String, String);

fn channel_key(database: &str, channel: &str) -> ChannelKey {
    (database.to_string(), channel.to_string())
}

/// Information about a single WebSocket connection
struct ConnectionInfo {
    /// Channels this connection is subscribed to (names within `database`)
    subscribed_channels: HashSet<String>,
    /// Channels where this connection has presence
    presence_channels: HashSet<String>,
    /// Channel to send events to this specific connection
    event_tx: mpsc::Sender<ChannelEvent>,
    /// Database the connection's script belongs to: the channel namespace
    database: String,
    /// Connection created timestamp
    #[allow(dead_code)]
    connected_at: i64,
}

/// Statistics for channel manager
pub struct ChannelStats {
    pub total_channels_created: AtomicUsize,
    pub total_messages_broadcast: AtomicUsize,
    pub total_presence_joins: AtomicUsize,
    pub active_channels: AtomicUsize,
    pub active_connections: AtomicUsize,
}

impl Default for ChannelStats {
    fn default() -> Self {
        Self {
            total_channels_created: AtomicUsize::new(0),
            total_messages_broadcast: AtomicUsize::new(0),
            total_presence_joins: AtomicUsize::new(0),
            active_channels: AtomicUsize::new(0),
            active_connections: AtomicUsize::new(0),
        }
    }
}

/// Error types for channel operations
#[derive(Debug, Clone)]
pub enum ChannelError {
    ChannelNotFound(String),
    NoSubscribers,
    ConnectionNotFound(String),
    SendError(String),
}

impl std::fmt::Display for ChannelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelError::ChannelNotFound(name) => write!(f, "Channel not found: {}", name),
            ChannelError::NoSubscribers => write!(f, "No subscribers to receive message"),
            ChannelError::ConnectionNotFound(id) => write!(f, "Connection not found: {}", id),
            ChannelError::SendError(msg) => write!(f, "Send error: {}", msg),
        }
    }
}

impl std::error::Error for ChannelError {}

/// Central manager for WebSocket channels and presence
pub struct ChannelManager {
    /// All active channels
    channels: Arc<RwLock<HashMap<ChannelKey, ChannelState>>>,
    /// All active connections
    connections: Arc<RwLock<HashMap<ConnectionId, ConnectionInfo>>>,
    /// Stats
    pub stats: Arc<ChannelStats>,
}

impl Default for ChannelManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelManager {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            connections: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(ChannelStats::default()),
        }
    }

    /// Register a new WebSocket connection
    pub fn register_connection(
        &self,
        database: &str,
    ) -> (ConnectionId, mpsc::Receiver<ChannelEvent>) {
        let conn_id = uuid::Uuid::new_v4().to_string();
        let (event_tx, event_rx) = mpsc::channel(100);

        let info = ConnectionInfo {
            subscribed_channels: HashSet::new(),
            presence_channels: HashSet::new(),
            event_tx,
            database: database.to_string(),
            connected_at: chrono::Utc::now().timestamp_millis(),
        };

        self.connections
            .write()
            .unwrap()
            .insert(conn_id.clone(), info);
        self.stats.active_connections.fetch_add(1, Ordering::SeqCst);

        (conn_id, event_rx)
    }

    /// The database a registered connection belongs to.
    fn database_of(&self, conn_id: &ConnectionId) -> Result<String, ChannelError> {
        self.connections
            .read()
            .unwrap()
            .get(conn_id)
            .map(|c| c.database.clone())
            .ok_or_else(|| ChannelError::ConnectionNotFound(conn_id.clone()))
    }

    /// Unregister a connection and cleanup all subscriptions/presence
    pub fn unregister_connection(&self, conn_id: &ConnectionId) {
        let conn_info = self.connections.write().unwrap().remove(conn_id);

        if let Some(info) = conn_info {
            // Leave all presence channels
            for channel in info.presence_channels.iter() {
                self.presence_leave_internal(conn_id, &info.database, channel);
            }

            // Unsubscribe from all channels
            for channel in info.subscribed_channels.iter() {
                self.unsubscribe_internal(conn_id, &info.database, channel);
            }

            self.stats.active_connections.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Subscribe to a channel in the connection's database. Idempotent per
    /// connection.
    pub fn subscribe(
        &self,
        conn_id: &ConnectionId,
        channel: &str,
    ) -> Result<broadcast::Receiver<ChannelMessage>, ChannelError> {
        let database = self.database_of(conn_id)?;

        // Get or create channel
        let mut channels = self.channels.write().unwrap();
        let channel_state = channels
            .entry(channel_key(&database, channel))
            .or_insert_with(|| ChannelState::new(&self.stats));
        channel_state.subscribers.insert(conn_id.clone());
        let rx = channel_state.message_tx.subscribe();
        drop(channels);

        // Track subscription in connection
        if let Some(conn) = self.connections.write().unwrap().get_mut(conn_id) {
            conn.subscribed_channels.insert(channel.to_string());
        }

        Ok(rx)
    }

    /// Unsubscribe from a channel
    pub fn unsubscribe(&self, conn_id: &ConnectionId, channel: &str) {
        let Ok(database) = self.database_of(conn_id) else {
            return;
        };
        self.unsubscribe_internal(conn_id, &database, channel);

        if let Some(conn) = self.connections.write().unwrap().get_mut(conn_id) {
            conn.subscribed_channels.remove(channel);
        }
    }

    fn unsubscribe_internal(&self, conn_id: &ConnectionId, database: &str, channel: &str) {
        let mut channels = self.channels.write().unwrap();
        let key = channel_key(database, channel);
        if let Some(state) = channels.get_mut(&key) {
            state.subscribers.remove(conn_id);

            // Cleanup empty channels
            if state.is_unused() {
                channels.remove(&key);
                self.stats.active_channels.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    /// Broadcast a message to all subscribers of `channel` in `database`.
    ///
    /// Delivered to every subscribed connection's event queue (what
    /// `ws.recv_any` reads) and to any `broadcast::Receiver` handed out by
    /// [`Self::subscribe`]. Returns the number of connections reached.
    pub fn broadcast(
        &self,
        database: &str,
        channel: &str,
        data: JsonValue,
        sender_id: Option<&ConnectionId>,
    ) -> Result<usize, ChannelError> {
        let channels = self.channels.read().unwrap();

        let Some(state) = channels.get(&channel_key(database, channel)) else {
            return Err(ChannelError::ChannelNotFound(channel.to_string()));
        };
        let msg = ChannelMessage {
            channel: channel.to_string(),
            data,
            sender_id: sender_id.cloned(),
            timestamp: chrono::Utc::now().timestamp_millis(),
        };
        let subscribers: Vec<ConnectionId> = state.subscribers.iter().cloned().collect();
        let _ = state.message_tx.send(msg.clone());
        drop(channels);

        let connections = self.connections.read().unwrap();
        let mut delivered = 0;
        for id in &subscribers {
            if let Some(conn) = connections.get(id) {
                if conn.database == database
                    && conn
                        .event_tx
                        .try_send(ChannelEvent::Message(msg.clone()))
                        .is_ok()
                {
                    delivered += 1;
                }
            }
        }

        self.stats
            .total_messages_broadcast
            .fetch_add(1, Ordering::SeqCst);
        Ok(delivered)
    }

    /// Broadcast from a connection, in that connection's database.
    pub fn broadcast_from(
        &self,
        conn_id: &ConnectionId,
        channel: &str,
        data: JsonValue,
    ) -> Result<usize, ChannelError> {
        let database = self.database_of(conn_id)?;
        self.broadcast(&database, channel, data, Some(conn_id))
    }

    /// Join presence in a channel
    pub fn presence_join(
        &self,
        conn_id: &ConnectionId,
        channel: &str,
        user_info: JsonValue,
    ) -> Result<(), ChannelError> {
        let database = self.database_of(conn_id)?;
        let mut channels = self.channels.write().unwrap();

        // Ensure channel exists
        let channel_state = channels
            .entry(channel_key(&database, channel))
            .or_insert_with(|| ChannelState::new(&self.stats));

        let presence_info = PresenceInfo {
            connection_id: conn_id.clone(),
            user_info: user_info.clone(),
            joined_at: chrono::Utc::now().timestamp_millis(),
        };

        channel_state
            .presence
            .insert(conn_id.clone(), presence_info);
        self.stats
            .total_presence_joins
            .fetch_add(1, Ordering::SeqCst);

        // Broadcast presence event to all subscribers
        let event = PresenceEvent {
            event_type: PresenceEventType::Join,
            channel: channel.to_string(),
            user_info,
            connection_id: conn_id.clone(),
            timestamp: chrono::Utc::now().timestamp_millis(),
        };

        drop(channels);

        // Track in connection
        if let Some(conn) = self.connections.write().unwrap().get_mut(conn_id) {
            conn.presence_channels.insert(channel.to_string());
        }

        self.broadcast_presence_event(&database, channel, event);

        Ok(())
    }

    /// Leave presence in a channel
    pub fn presence_leave(&self, conn_id: &ConnectionId, channel: &str) {
        let Ok(database) = self.database_of(conn_id) else {
            return;
        };
        self.presence_leave_internal(conn_id, &database, channel);

        if let Some(conn) = self.connections.write().unwrap().get_mut(conn_id) {
            conn.presence_channels.remove(channel);
        }
    }

    fn presence_leave_internal(&self, conn_id: &ConnectionId, database: &str, channel: &str) {
        let mut channels = self.channels.write().unwrap();
        let key = channel_key(database, channel);

        if let Some(state) = channels.get_mut(&key) {
            if let Some(info) = state.presence.remove(conn_id) {
                let event = PresenceEvent {
                    event_type: PresenceEventType::Leave,
                    channel: channel.to_string(),
                    user_info: info.user_info,
                    connection_id: conn_id.clone(),
                    timestamp: chrono::Utc::now().timestamp_millis(),
                };

                // Cleanup empty channels
                if state.is_unused() {
                    channels.remove(&key);
                    self.stats.active_channels.fetch_sub(1, Ordering::SeqCst);
                }

                drop(channels);
                self.broadcast_presence_event(database, channel, event);
            }
        }
    }

    /// List all users present in `channel` of `database`
    pub fn presence_list(&self, database: &str, channel: &str) -> Vec<PresenceInfo> {
        let channels = self.channels.read().unwrap();

        channels
            .get(&channel_key(database, channel))
            .map(|state| state.presence.values().cloned().collect())
            .unwrap_or_default()
    }

    /// List channels a connection is subscribed to
    pub fn list_subscriptions(&self, conn_id: &ConnectionId) -> Vec<String> {
        self.connections
            .read()
            .unwrap()
            .get(conn_id)
            .map(|c| c.subscribed_channels.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Broadcast presence event to the connections of `database` listening
    /// on `channel`
    fn broadcast_presence_event(&self, database: &str, channel: &str, event: PresenceEvent) {
        let connections = self.connections.read().unwrap();

        for conn in connections.values() {
            if conn.database == database
                && (conn.subscribed_channels.contains(channel)
                    || conn.presence_channels.contains(channel))
            {
                let _ = conn
                    .event_tx
                    .try_send(ChannelEvent::Presence(event.clone()));
            }
        }
    }

    /// Get connection's event sender for receiving channel messages
    pub fn get_event_sender(&self, conn_id: &ConnectionId) -> Option<mpsc::Sender<ChannelEvent>> {
        self.connections
            .read()
            .unwrap()
            .get(conn_id)
            .map(|c| c.event_tx.clone())
    }

    #[cfg(test)]
    fn channel_count(&self) -> usize {
        self.channels.read().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_unregister() {
        let manager = ChannelManager::new();

        let (conn_id, _rx) = manager.register_connection("test_db");
        assert_eq!(manager.stats.active_connections.load(Ordering::SeqCst), 1);

        manager.unregister_connection(&conn_id);
        assert_eq!(manager.stats.active_connections.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_subscribe_broadcast() {
        let manager = ChannelManager::new();

        let (conn_id, _event_rx) = manager.register_connection("test_db");
        let mut msg_rx = manager.subscribe(&conn_id, "test-channel").unwrap();

        // Broadcast a message
        let result = manager.broadcast(
            "test_db",
            "test-channel",
            serde_json::json!({"hello": "world"}),
            Some(&conn_id),
        );
        assert!(result.is_ok());

        // Receive the message
        let msg = msg_rx.recv().await.unwrap();
        assert_eq!(msg.channel, "test-channel");
        assert_eq!(msg.data, serde_json::json!({"hello": "world"}));

        manager.unregister_connection(&conn_id);
    }

    #[tokio::test]
    async fn test_presence_join_leave() {
        let manager = ChannelManager::new();

        let (conn_id, mut event_rx) = manager.register_connection("test_db");

        // Also subscribe so we get presence events
        let _msg_rx = manager.subscribe(&conn_id, "room-1").unwrap();

        let (conn_id2, mut event_rx2) = manager.register_connection("test_db");
        let _msg_rx2 = manager.subscribe(&conn_id2, "room-1").unwrap();

        // Join presence
        manager
            .presence_join(
                &conn_id,
                "room-1",
                serde_json::json!({"user_id": "alice", "name": "Alice"}),
            )
            .unwrap();

        // conn_id2 should receive presence event
        if let Ok(Some(ChannelEvent::Presence(event))) =
            tokio::time::timeout(std::time::Duration::from_millis(100), event_rx2.recv()).await
        {
            assert!(matches!(event.event_type, PresenceEventType::Join));
            assert_eq!(event.channel, "room-1");
        }

        // conn_id should ALSO receive presence event (self-presence)
        if let Ok(Some(ChannelEvent::Presence(event))) =
            tokio::time::timeout(std::time::Duration::from_millis(100), event_rx.recv()).await
        {
            assert!(matches!(event.event_type, PresenceEventType::Join));
            assert_eq!(event.channel, "room-1");
        }

        // Check presence list
        let users = manager.presence_list("test_db", "room-1");
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].connection_id, conn_id);

        // Leave presence
        manager.presence_leave(&conn_id, "room-1");

        // conn_id should receive leave event
        if let Ok(Some(ChannelEvent::Presence(event))) =
            tokio::time::timeout(std::time::Duration::from_millis(100), event_rx.recv()).await
        {
            assert!(matches!(event.event_type, PresenceEventType::Leave));
        }

        let users = manager.presence_list("test_db", "room-1");
        assert_eq!(users.len(), 0);

        manager.unregister_connection(&conn_id);
        manager.unregister_connection(&conn_id2);
    }

    /// Audit H6: channels and presence are scoped to the connection's
    /// database.
    #[tokio::test]
    async fn channels_are_namespaced_per_database() {
        let manager = ChannelManager::new();
        let (a, mut a_rx) = manager.register_connection("tenant_a");
        let (b, mut b_rx) = manager.register_connection("tenant_b");
        manager.subscribe(&a, "chat").unwrap();
        manager.subscribe(&b, "chat").unwrap();
        manager
            .presence_join(&a, "chat", serde_json::json!({"user": "alice"}))
            .unwrap();

        // A's own presence event.
        let _ = a_rx.try_recv();
        // B sees neither A's presence nor A's message.
        assert!(b_rx.try_recv().is_err());
        assert!(manager.presence_list("tenant_b", "chat").is_empty());
        assert_eq!(manager.presence_list("tenant_a", "chat").len(), 1);

        let delivered = manager
            .broadcast_from(&a, "chat", serde_json::json!("hi"))
            .unwrap();
        assert_eq!(delivered, 1);
        assert!(matches!(a_rx.try_recv(), Ok(ChannelEvent::Message(_))));
        assert!(b_rx.try_recv().is_err());

        manager.unregister_connection(&a);
        manager.unregister_connection(&b);
        assert_eq!(manager.channel_count(), 0);
    }

    /// Audit M6: resubscribing must not leave a channel alive after the
    /// connection goes away.
    #[tokio::test]
    async fn resubscribe_is_idempotent() {
        let manager = ChannelManager::new();
        let (conn, _rx) = manager.register_connection("db");
        for _ in 0..5 {
            manager.subscribe(&conn, "room").unwrap();
        }
        manager.unregister_connection(&conn);
        assert_eq!(manager.channel_count(), 0);
        assert_eq!(manager.stats.active_channels.load(Ordering::SeqCst), 0);
    }
}
