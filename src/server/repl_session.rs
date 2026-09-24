//! REPL session management for interactive Lua script execution
//!
//! Provides stateful sessions that persist variables between REPL evaluations.

use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::{Arc, Once, RwLock, Weak};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A REPL session that maintains state between evaluations
#[derive(Debug, Clone)]
pub struct ReplSession {
    /// Unique session identifier
    pub id: String,
    /// Database context for this session
    pub db_name: String,
    /// The user who opened it. A session is only ever handed back to them,
    /// and the per-user cap counts by it.
    pub owner: String,
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
        Self::new_for(db_name, String::new())
    }

    /// Create a REPL session owned by `owner`.
    pub fn new_for(db_name: String, owner: String) -> Self {
        let now = Instant::now();
        Self {
            id: Uuid::now_v7().to_string(),
            db_name,
            owner,
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
        // Keep only the last 100 commands, and no more than
        // MAX_HISTORY_BYTES of source in total: every entry is held for the
        // session's lifetime and scanned on each eval.
        if self.history.len() > 100 {
            self.history.remove(0);
        }
        let mut total: usize = self.history.iter().map(|c| c.len()).sum();
        while total > MAX_HISTORY_BYTES && self.history.len() > 1 {
            total -= self.history.remove(0).len();
        }
    }

    /// Check if the session has expired
    pub fn is_expired(&self, timeout: Duration) -> bool {
        self.last_accessed.elapsed() > timeout
    }
}

/// Upper bound on the source kept in one session's history.
const MAX_HISTORY_BYTES: usize = 1024 * 1024;

/// How often the background sweep drops expired sessions.
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

/// Why a session could not be created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplSessionError {
    /// The instance-wide session cap is reached (after dropping expired
    /// sessions).
    TooManySessions(usize),
}

impl std::fmt::Display for ReplSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplSessionError::TooManySessions(max) => write!(
                f,
                "Too many open REPL sessions (limit {}); retry later or reuse a session_id",
                max
            ),
        }
    }
}

/// Store for managing REPL sessions
#[derive(Clone)]
pub struct ReplSessionStore {
    sessions: Arc<RwLock<HashMap<String, ReplSession>>>,
    /// Session timeout duration (default 30 minutes)
    timeout: Duration,
    /// Sessions one user may hold; the least recently used is dropped to
    /// make room (`SOLIDB_REPL_MAX_SESSIONS_PER_USER`, default 8).
    max_per_user: usize,
    /// Sessions the instance holds in total; creation fails beyond it
    /// (`SOLIDB_REPL_MAX_SESSIONS`, default 1000).
    max_total: usize,
    /// Starts the periodic sweep the first time a session is created.
    cleanup_started: Arc<Once>,
}

impl Default for ReplSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplSessionStore {
    /// Create a new session store with default 30-minute timeout
    pub fn new() -> Self {
        Self::with_timeout(30 * 60) // 30 minutes
    }

    /// Create a new session store with custom timeout
    pub fn with_timeout(timeout_secs: u64) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            timeout: Duration::from_secs(timeout_secs),
            max_per_user: env_usize("SOLIDB_REPL_MAX_SESSIONS_PER_USER", 8),
            max_total: env_usize("SOLIDB_REPL_MAX_SESSIONS", 1000),
            cleanup_started: Arc::new(Once::new()),
        }
    }

    /// Override the caps (tests).
    pub fn with_limits(mut self, max_per_user: usize, max_total: usize) -> Self {
        self.max_per_user = max_per_user.max(1);
        self.max_total = max_total.max(1);
        self
    }

    /// Get an existing session or create a new one
    pub fn get_or_create(&self, session_id: Option<&str>, db_name: &str) -> ReplSession {
        match self.get_or_create_for(session_id, db_name, "") {
            Ok(session) => session,
            // Unowned sessions are a test/compat path; never refuse it.
            Err(_) => ReplSession::new(db_name.to_string()),
        }
    }

    /// Get `owner`'s session `session_id` on `db_name`, or create one.
    ///
    /// Audit M3: every call without a `session_id` used to create a session
    /// that was only ever removed if its id came back, so sessions (each
    /// holding up to 100 history entries) accumulated without bound. They
    /// are now capped per user and in total, and swept periodically.
    ///
    /// A session belonging to another user is never returned: its id alone
    /// does not grant it.
    pub fn get_or_create_for(
        &self,
        session_id: Option<&str>,
        db_name: &str,
        owner: &str,
    ) -> Result<ReplSession, ReplSessionError> {
        self.ensure_cleanup_task();
        let mut sessions = self.sessions.write().unwrap();

        // Try to get existing session
        if let Some(id) = session_id {
            if let Some(session) = sessions.get_mut(id) {
                if session.owner == owner
                    && session.db_name == db_name
                    && !session.is_expired(self.timeout)
                {
                    session.touch();
                    return Ok(session.clone());
                } else if session.owner == owner || session.is_expired(self.timeout) {
                    // Remove expired or mismatched session of this owner.
                    // Someone else's live session is left alone.
                    sessions.remove(id);
                }
            }
        }

        // Make room under the per-user cap by dropping this user's least
        // recently used sessions.
        let mut own: Vec<(String, Instant)> = sessions
            .values()
            .filter(|s| s.owner == owner)
            .map(|s| (s.id.clone(), s.last_accessed))
            .collect();
        if own.len() >= self.max_per_user {
            own.sort_by_key(|(_, at)| *at);
            let excess = own.len() + 1 - self.max_per_user;
            for (id, _) in own.into_iter().take(excess) {
                sessions.remove(&id);
            }
        }

        if sessions.len() >= self.max_total {
            let timeout = self.timeout;
            sessions.retain(|_, s| !s.is_expired(timeout));
            if sessions.len() >= self.max_total {
                return Err(ReplSessionError::TooManySessions(self.max_total));
            }
        }

        // Create new session
        let session = ReplSession::new_for(db_name.to_string(), owner.to_string());
        sessions.insert(session.id.clone(), session.clone());
        Ok(session)
    }

    /// Spawn the periodic expiry sweep, once, on the current Tokio runtime.
    ///
    /// Started lazily from here rather than from server setup so the store
    /// is self-contained. The task holds only a weak reference and stops
    /// once the store is gone. Outside a runtime (unit tests) nothing is
    /// spawned, and a later call inside one will start it.
    fn ensure_cleanup_task(&self) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let weak: Weak<RwLock<HashMap<String, ReplSession>>> = Arc::downgrade(&self.sessions);
        let timeout = self.timeout;
        self.cleanup_started.call_once(|| {
            handle.spawn(async move {
                let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
                interval.tick().await;
                loop {
                    interval.tick().await;
                    let Some(sessions) = weak.upgrade() else {
                        break;
                    };
                    if let Ok(mut sessions) = sessions.write() {
                        sessions.retain(|_, s| !s.is_expired(timeout));
                    };
                }
            });
        });
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
        sessions
            .values()
            .filter(|s| !s.is_expired(self.timeout))
            .count()
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
    fn sessions_are_owned_and_capped_per_user() {
        let store = ReplSessionStore::new().with_limits(2, 100);
        let a1 = store.get_or_create_for(None, "db", "alice").unwrap();
        // Bob cannot pick up Alice's session by id.
        let b = store.get_or_create_for(Some(&a1.id), "db", "bob").unwrap();
        assert_ne!(a1.id, b.id);
        assert!(store.get(&a1.id).is_some(), "alice's session survives");

        let _a2 = store.get_or_create_for(None, "db", "alice").unwrap();
        let _a3 = store.get_or_create_for(None, "db", "alice").unwrap();
        // The oldest of alice's sessions was dropped to stay under the cap.
        assert!(store.get(&a1.id).is_none());
        assert_eq!(store.active_count(), 3); // two for alice, one for bob
    }

    #[test]
    fn global_cap_refuses_new_sessions() {
        let store = ReplSessionStore::new().with_limits(10, 2);
        store.get_or_create_for(None, "db", "a").unwrap();
        store.get_or_create_for(None, "db", "b").unwrap();
        assert_eq!(
            store.get_or_create_for(None, "db", "c").unwrap_err(),
            ReplSessionError::TooManySessions(2)
        );
    }

    #[test]
    fn history_is_bounded_by_bytes() {
        let mut session = ReplSession::new("test".to_string());
        for _ in 0..10 {
            session.add_to_history("x".repeat(300 * 1024));
        }
        let total: usize = session.history.iter().map(|c| c.len()).sum();
        assert!(total <= MAX_HISTORY_BYTES);
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
