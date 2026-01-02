//! REPL session management for interactive Lua script execution
//!
//! Provides stateful sessions that persist variables between REPL evaluations.

use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A REPL session that maintains state between evaluations
#[derive(Debug, Clone)]
pub struct ReplSession {
    /// Unique session identifier
    pub id: String,
    /// Database context for this session
    pub db_name: String,
    /// Variables persisted across evaluations (stored as JSON)
    pub variables: HashMap<String, JsonValue>,
    /// Command history for this session
    pub history: Vec<String>,
    /// When the session was created
    pub created_at: Instant,
    /// Last time the session was accessed
    pub last_accessed: Instant,
}

impl ReplSession {
    /// Create a new REPL session
    pub fn new(db_name: String) -> Self {
        let now = Instant::now();
        Self {
            id: Uuid::now_v7().to_string(),
            db_name,
            variables: HashMap::new(),
            history: Vec::new(),
            created_at: now,
            last_accessed: now,
        }
    }

    /// Update the last accessed time
    pub fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }

    /// Add a command to history
    pub fn add_to_history(&mut self, code: String) {
        self.history.push(code);
        // Keep only the last 100 commands
        if self.history.len() > 100 {
            self.history.remove(0);
        }
    }

    /// Check if the session has expired
    pub fn is_expired(&self, timeout: Duration) -> bool {
        self.last_accessed.elapsed() > timeout
    }
}

/// Store for managing REPL sessions
#[derive(Clone)]
pub struct ReplSessionStore {
    sessions: Arc<RwLock<HashMap<String, ReplSession>>>,
    /// Session timeout duration (default 30 minutes)
    timeout: Duration,
}

impl Default for ReplSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplSessionStore {
    /// Create a new session store with default 30-minute timeout
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            timeout: Duration::from_secs(30 * 60), // 30 minutes
        }
    }

    /// Create a new session store with custom timeout
    pub fn with_timeout(timeout_secs: u64) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            timeout: Duration::from_secs(timeout_secs),
        }
    }

    /// Get an existing session or create a new one
    pub fn get_or_create(&self, session_id: Option<&str>, db_name: &str) -> ReplSession {
        let mut sessions = self.sessions.write().unwrap();

        // Try to get existing session
        if let Some(id) = session_id {
            if let Some(session) = sessions.get_mut(id) {
                // Check if session is for the same database
                if session.db_name == db_name && !session.is_expired(self.timeout) {
                    session.touch();
                    return session.clone();
                } else {
                    // Remove expired or mismatched session
                    sessions.remove(id);
                }
            }
        }

        // Create new session
        let session = ReplSession::new(db_name.to_string());
        sessions.insert(session.id.clone(), session.clone());
        session
    }

    /// Get a session by ID (returns None if expired or not found)
    pub fn get(&self, session_id: &str) -> Option<ReplSession> {
        let sessions = self.sessions.read().unwrap();
        sessions.get(session_id).and_then(|s| {
            if s.is_expired(self.timeout) {
                None
            } else {
                Some(s.clone())
            }
        })
    }

    /// Update a session's variables and history
    pub fn update(&self, session: ReplSession) {
        let mut sessions = self.sessions.write().unwrap();
        sessions.insert(session.id.clone(), session);
    }

    /// Update only the variables for a session
    pub fn update_variables(&self, session_id: &str, variables: HashMap<String, JsonValue>) {
        let mut sessions = self.sessions.write().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            session.variables = variables;
            session.touch();
        }
    }

    /// Remove expired sessions
    pub fn cleanup_expired(&self) -> usize {
        let mut sessions = self.sessions.write().unwrap();
        let before_count = sessions.len();
        sessions.retain(|_, session| !session.is_expired(self.timeout));
        before_count - sessions.len()
    }

    /// Get the number of active sessions
    pub fn active_count(&self) -> usize {
        let sessions = self.sessions.read().unwrap();
        sessions.values().filter(|s| !s.is_expired(self.timeout)).count()
    }

    /// Delete a specific session
    pub fn delete(&self, session_id: &str) -> bool {
        let mut sessions = self.sessions.write().unwrap();
        sessions.remove(session_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_creation() {
        let session = ReplSession::new("test_db".to_string());
        assert!(!session.id.is_empty());
        assert_eq!(session.db_name, "test_db");
        assert!(session.variables.is_empty());
        assert!(session.history.is_empty());
    }

    #[test]
    fn test_session_store_get_or_create() {
        let store = ReplSessionStore::new();

        // Create new session
        let session1 = store.get_or_create(None, "db1");
        assert_eq!(session1.db_name, "db1");

        // Get same session by ID
        let session2 = store.get_or_create(Some(&session1.id), "db1");
        assert_eq!(session1.id, session2.id);

        // Different DB should create new session even with same ID
        let session3 = store.get_or_create(Some(&session1.id), "db2");
        assert_ne!(session1.id, session3.id);
    }

    #[test]
    fn test_session_expiration() {
        let store = ReplSessionStore::with_timeout(0); // Immediate timeout

        let session = store.get_or_create(None, "test_db");
        std::thread::sleep(Duration::from_millis(10));

        // Session should be expired
        assert!(store.get(&session.id).is_none());
    }

    #[test]
    fn test_cleanup_expired() {
        let store = ReplSessionStore::with_timeout(0);

        // Create a few sessions
        store.get_or_create(None, "db1");
        store.get_or_create(None, "db2");

        std::thread::sleep(Duration::from_millis(10));

        let cleaned = store.cleanup_expired();
        assert_eq!(cleaned, 2);
        assert_eq!(store.active_count(), 0);
    }

    #[test]
    fn test_history_limit() {
        let mut session = ReplSession::new("test".to_string());

        for i in 0..150 {
            session.add_to_history(format!("command {}", i));
        }

        assert_eq!(session.history.len(), 100);
        assert_eq!(session.history[0], "command 50");
    }
}
