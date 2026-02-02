//! Offline Sync Local Storage
//!
//! Provides SQLite-based local storage for offline-first synchronization.
//! Stores documents, version vectors, and pending changes.

use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// Configuration for the offline queue bounds
///
/// When the queue is full (either by count or bytes), new changes will be rejected
/// to prevent unbounded memory growth while offline.
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// Maximum number of pending changes (default: 10,000)
    pub max_count: usize,
    /// Maximum total size in bytes of pending change data (default: 100MB)
    pub max_bytes: usize,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_count: 10_000,
            max_bytes: 100 * 1024 * 1024, // 100MB
        }
    }
}

impl QueueConfig {
    /// Create a new queue config with custom limits
    pub fn new(max_count: usize, max_bytes: usize) -> Self {
        Self {
            max_count,
            max_bytes,
        }
    }

    /// Create a config for limited mobile devices
    pub fn mobile() -> Self {
        Self {
            max_count: 1_000,
            max_bytes: 10 * 1024 * 1024, // 10MB
        }
    }

    /// Create a config for desktop applications
    pub fn desktop() -> Self {
        Self {
            max_count: 50_000,
            max_bytes: 500 * 1024 * 1024, // 500MB
        }
    }
}

/// Result of attempting to add a change to a bounded queue
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueResult {
    /// Change was added successfully
    Added,
    /// Queue is full by count limit
    RejectedCountLimit,
    /// Queue is full by bytes limit
    RejectedBytesLimit,
}

/// Queue statistics for monitoring
#[derive(Debug, Clone, Default)]
pub struct QueueStats {
    /// Number of pending changes
    pub count: usize,
    /// Total size in bytes of pending change data
    pub bytes: usize,
}

/// Local storage backend using SQLite
pub struct LocalStore {
    conn: Connection,
    device_id: String,
}

impl LocalStore {
    /// Open or create a local store at the given path
    pub fn open<P: AsRef<Path>>(path: P, device_id: String) -> SqliteResult<Self> {
        let conn = Connection::open(path)?;
        let mut store = Self { conn, device_id };
        store.init_schema()?;
        Ok(store)
    }

    /// Open a local store in the default location (user data directory)
    pub fn open_default(app_name: &str, device_id: String) -> SqliteResult<Self> {
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf())
            })
            .join(app_name);

        std::fs::create_dir_all(&data_dir).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;

        let db_path = data_dir.join("sync.db");
        Self::open(db_path, device_id)
    }

    /// Initialize the database schema
    fn init_schema(&mut self) -> SqliteResult<()> {
        // Documents table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS documents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                collection TEXT NOT NULL,
                key TEXT NOT NULL,
                data TEXT NOT NULL,
                version_vector TEXT NOT NULL,
                modified_at INTEGER NOT NULL,
                is_deleted BOOLEAN NOT NULL DEFAULT 0,
                UNIQUE(collection, key)
            )",
            [],
        )?;

        // Pending changes queue
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS pending_changes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                collection TEXT NOT NULL,
                document_key TEXT NOT NULL,
                operation TEXT NOT NULL,
                data TEXT,
                version_vector TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                retry_count INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

        // Sync metadata (last sync vector, etc.)
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS sync_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            )",
            [],
        )?;

        // Subscriptions
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS subscriptions (
                collection TEXT PRIMARY KEY,
                filter_query TEXT,
                subscribed_at INTEGER NOT NULL
            )",
            [],
        )?;

        // Indexes
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_docs_collection ON documents(collection)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pending_collection ON pending_changes(collection, document_key)",
            [],
        )?;

        // Insert device ID
        let device_id = self.device_id.clone();
        self.set_metadata("device_id", &device_id)?;

        Ok(())
    }

    // === Document Operations ===

    /// Store a document locally
    pub fn put_document(
        &mut self,
        collection: &str,
        key: &str,
        data: &Value,
        version_vector: &str,
    ) -> SqliteResult<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let data_str = serde_json::to_string(data).unwrap_or_default();

        self.conn.execute(
            "INSERT INTO documents (collection, key, data, version_vector, modified_at, is_deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)
             ON CONFLICT(collection, key) DO UPDATE SET
             data = excluded.data,
             version_vector = excluded.version_vector,
             modified_at = excluded.modified_at,
             is_deleted = 0",
            params![collection, key, data_str, version_vector, now],
        )?;

        Ok(())
    }

    /// Get a document by key
    pub fn get_document(
        &self,
        collection: &str,
        key: &str,
    ) -> SqliteResult<Option<(Value, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT data, version_vector FROM documents
             WHERE collection = ?1 AND key = ?2 AND is_deleted = 0",
        )?;

        let result = stmt
            .query_row(params![collection, key], |row| {
                let data_str: String = row.get(0)?;
                let vector: String = row.get(1)?;
                let data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);
                Ok((data, vector))
            })
            .optional()?;

        Ok(result)
    }

    /// Delete a document locally
    pub fn delete_document(
        &mut self,
        collection: &str,
        key: &str,
        version_vector: &str,
    ) -> SqliteResult<()> {
        let now = chrono::Utc::now().timestamp_millis();

        self.conn.execute(
            "UPDATE documents SET
             is_deleted = 1,
             version_vector = ?1,
             modified_at = ?2
             WHERE collection = ?3 AND key = ?4",
            params![version_vector, now, collection, key],
        )?;

        Ok(())
    }

    /// List all documents in a collection
    pub fn list_documents(&self, collection: &str) -> SqliteResult<Vec<(String, Value, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT key, data, version_vector FROM documents
             WHERE collection = ?1 AND is_deleted = 0
             ORDER BY key",
        )?;

        let rows = stmt.query_map(params![collection], |row| {
            let key: String = row.get(0)?;
            let data_str: String = row.get(1)?;
            let vector: String = row.get(2)?;
            let data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);
            Ok((key, data, vector))
        })?;

        rows.collect()
    }

    /// Get all document keys and their version vectors (for sync)
    pub fn get_all_versions(&self, collection: &str) -> SqliteResult<HashMap<String, String>> {
        let mut stmt = self.conn.prepare(
            "SELECT key, version_vector FROM documents
             WHERE collection = ?1",
        )?;

        let rows = stmt.query_map(params![collection], |row| {
            let key: String = row.get(0)?;
            let vector: String = row.get(1)?;
            Ok((key, vector))
        })?;

        rows.collect()
    }

    /// List all collections that have documents
    pub fn list_collections(&self) -> SqliteResult<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT collection FROM documents WHERE is_deleted = 0")?;

        let rows = stmt.query_map([], |row| {
            let collection: String = row.get(0)?;
            Ok(collection)
        })?;

        rows.collect()
    }

    // === Pending Changes ===

    /// Add a pending change to the queue
    pub fn add_pending_change(
        &mut self,
        collection: &str,
        document_key: &str,
        operation: &str,
        data: Option<&Value>,
        version_vector: &str,
    ) -> SqliteResult<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let data_str = data.map(|d| serde_json::to_string(d).unwrap_or_default());

        self.conn.execute(
            "INSERT INTO pending_changes (collection, document_key, operation, data, version_vector, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![collection, document_key, operation, data_str, version_vector, now],
        )?;

        Ok(())
    }

    /// Add a pending change to the queue with bounds checking
    ///
    /// Returns `QueueResult::Added` if the change was successfully added,
    /// or a rejection reason if the queue is full.
    ///
    /// This uses a "reject new" strategy - when the queue is full, new changes
    /// are rejected rather than dropping old ones. This is the safest approach
    /// as it never loses acknowledged changes.
    pub fn add_pending_change_bounded(
        &mut self,
        collection: &str,
        document_key: &str,
        operation: &str,
        data: Option<&Value>,
        version_vector: &str,
        config: &QueueConfig,
    ) -> SqliteResult<QueueResult> {
        // Get current queue stats
        let stats = self.get_queue_stats()?;

        // Check count limit
        if stats.count >= config.max_count {
            return Ok(QueueResult::RejectedCountLimit);
        }

        // Calculate size of new change data
        let new_size = data
            .map(|d| serde_json::to_string(d).map(|s| s.len()).unwrap_or(0))
            .unwrap_or(0);

        // Check bytes limit
        if stats.bytes + new_size > config.max_bytes {
            return Ok(QueueResult::RejectedBytesLimit);
        }

        // Add the change
        self.add_pending_change(collection, document_key, operation, data, version_vector)?;

        Ok(QueueResult::Added)
    }

    /// Get current queue statistics
    pub fn get_queue_stats(&self) -> SqliteResult<QueueStats> {
        let mut stmt = self
            .conn
            .prepare("SELECT COUNT(*), COALESCE(SUM(LENGTH(data)), 0) FROM pending_changes")?;

        stmt.query_row([], |row| {
            let count: i64 = row.get(0)?;
            let bytes: i64 = row.get(1)?;
            Ok(QueueStats {
                count: count as usize,
                bytes: bytes as usize,
            })
        })
    }

    /// Check if the queue can accept a change of the given size
    pub fn can_accept_change(&self, data_size: usize, config: &QueueConfig) -> SqliteResult<bool> {
        let stats = self.get_queue_stats()?;
        Ok(stats.count < config.max_count && stats.bytes + data_size <= config.max_bytes)
    }

    /// Get the number of pending changes that can still be added
    pub fn remaining_capacity(&self, config: &QueueConfig) -> SqliteResult<usize> {
        let stats = self.get_queue_stats()?;
        Ok(config.max_count.saturating_sub(stats.count))
    }

    /// Get all pending changes
    pub fn get_pending_changes(&self) -> SqliteResult<Vec<PendingChange>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, collection, document_key, operation, data, version_vector, retry_count
             FROM pending_changes
             ORDER BY created_at ASC
             LIMIT 1000",
        )?;

        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let collection: String = row.get(1)?;
            let document_key: String = row.get(2)?;
            let operation: String = row.get(3)?;
            let data_str: Option<String> = row.get(4)?;
            let version_vector: String = row.get(5)?;
            let retry_count: i64 = row.get(6)?;

            let data = data_str.and_then(|s| serde_json::from_str(&s).ok());

            Ok(PendingChange {
                id,
                collection,
                document_key,
                operation,
                data,
                version_vector,
                retry_count: retry_count as u32,
            })
        })?;

        rows.collect()
    }

    /// Remove a pending change after successful sync
    pub fn remove_pending_change(&mut self, id: i64) -> SqliteResult<()> {
        self.conn
            .execute("DELETE FROM pending_changes WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Increment retry count for a pending change
    pub fn increment_retry(&mut self, id: i64) -> SqliteResult<()> {
        self.conn.execute(
            "UPDATE pending_changes SET retry_count = retry_count + 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// Clear all pending changes (e.g., after full sync)
    pub fn clear_pending_changes(&mut self) -> SqliteResult<()> {
        self.conn.execute("DELETE FROM pending_changes", [])?;
        Ok(())
    }

    // === Metadata ===

    /// Set a metadata value
    pub fn set_metadata(&mut self, key: &str, value: &str) -> SqliteResult<()> {
        let now = chrono::Utc::now().timestamp_millis();

        self.conn.execute(
            "INSERT INTO sync_metadata (key, value, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET
             value = excluded.value,
             updated_at = excluded.updated_at",
            params![key, value, now],
        )?;

        Ok(())
    }

    /// Get a metadata value
    pub fn get_metadata(&self, key: &str) -> SqliteResult<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM sync_metadata WHERE key = ?1")?;

        stmt.query_row(params![key], |row| row.get(0)).optional()
    }

    /// Get the last sync vector
    pub fn get_last_sync_vector(&self) -> SqliteResult<Option<String>> {
        self.get_metadata("last_sync_vector")
    }

    /// Set the last sync vector
    pub fn set_last_sync_vector(&mut self, vector: &str) -> SqliteResult<()> {
        self.set_metadata("last_sync_vector", vector)
    }

    // === Subscriptions ===

    /// Subscribe to a collection
    pub fn subscribe_collection(
        &mut self,
        collection: &str,
        filter_query: Option<&str>,
    ) -> SqliteResult<()> {
        let now = chrono::Utc::now().timestamp_millis();

        self.conn.execute(
            "INSERT INTO subscriptions (collection, filter_query, subscribed_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(collection) DO UPDATE SET
             filter_query = excluded.filter_query",
            params![collection, filter_query, now],
        )?;

        Ok(())
    }

    /// Unsubscribe from a collection
    pub fn unsubscribe_collection(&mut self, collection: &str) -> SqliteResult<()> {
        self.conn.execute(
            "DELETE FROM subscriptions WHERE collection = ?1",
            params![collection],
        )?;
        Ok(())
    }

    /// Get all subscriptions
    pub fn get_subscriptions(&self) -> SqliteResult<Vec<(String, Option<String>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT collection, filter_query FROM subscriptions ORDER BY collection")?;

        let rows = stmt.query_map([], |row| {
            let collection: String = row.get(0)?;
            let filter: Option<String> = row.get(1)?;
            Ok((collection, filter))
        })?;

        rows.collect()
    }

    // === Utility ===

    /// Get the device ID
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Close the database connection
    pub fn close(self) -> SqliteResult<()> {
        self.conn.close().map_err(|e| e.1)
    }
}

/// A pending change waiting to be synced
#[derive(Debug, Clone)]
pub struct PendingChange {
    pub id: i64,
    pub collection: String,
    pub document_key: String,
    pub operation: String,
    pub data: Option<Value>,
    pub version_vector: String,
    pub retry_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn create_test_store() -> LocalStore {
        let temp_path = PathBuf::from(format!("/tmp/test_sync_{}.db", uuid::Uuid::new_v4()));
        LocalStore::open(&temp_path, "test-device".to_string()).unwrap()
    }

    #[test]
    fn test_document_storage() {
        let mut store = create_test_store();
        let data = serde_json::json!({"name": "test", "value": 42});
        let vector = "{\"node1\": 1}";

        // Put document
        store
            .put_document("orders", "order-1", &data, vector)
            .unwrap();

        // Get document
        let (retrieved, retrieved_vector) =
            store.get_document("orders", "order-1").unwrap().unwrap();
        assert_eq!(retrieved["name"], "test");
        assert_eq!(retrieved_vector, vector);

        // List documents
        let docs = store.list_documents("orders").unwrap();
        assert_eq!(docs.len(), 1);

        // Delete document
        store.delete_document("orders", "order-1", vector).unwrap();
        let result = store.get_document("orders", "order-1").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_pending_changes() {
        let mut store = create_test_store();
        let data = serde_json::json!({"name": "test"});

        // Add pending change
        store
            .add_pending_change("orders", "order-1", "INSERT", Some(&data), "{\"node1\": 1}")
            .unwrap();

        // Get pending changes
        let changes = store.get_pending_changes().unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].collection, "orders");

        // Remove pending change
        store.remove_pending_change(changes[0].id).unwrap();
        let changes = store.get_pending_changes().unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn test_metadata() {
        let mut store = create_test_store();

        store.set_metadata("test_key", "test_value").unwrap();
        let value = store.get_metadata("test_key").unwrap().unwrap();
        assert_eq!(value, "test_value");
    }

    #[test]
    fn test_subscriptions() {
        let mut store = create_test_store();

        store
            .subscribe_collection("orders", Some("FILTER doc.user = 'alice'"))
            .unwrap();
        store.subscribe_collection("products", None).unwrap();

        let subs = store.get_subscriptions().unwrap();
        assert_eq!(subs.len(), 2);

        store.unsubscribe_collection("orders").unwrap();
        let subs = store.get_subscriptions().unwrap();
        assert_eq!(subs.len(), 1);
    }

    #[test]
    fn test_queue_stats() {
        let mut store = create_test_store();
        let data = serde_json::json!({"name": "test"});

        // Initially empty
        let stats = store.get_queue_stats().unwrap();
        assert_eq!(stats.count, 0);
        assert_eq!(stats.bytes, 0);

        // Add some changes
        store
            .add_pending_change("orders", "order-1", "INSERT", Some(&data), "{\"node1\": 1}")
            .unwrap();
        store
            .add_pending_change("orders", "order-2", "INSERT", Some(&data), "{\"node1\": 2}")
            .unwrap();

        let stats = store.get_queue_stats().unwrap();
        assert_eq!(stats.count, 2);
        assert!(stats.bytes > 0);
    }

    #[test]
    fn test_bounded_queue_count_limit() {
        let mut store = create_test_store();
        let data = serde_json::json!({"name": "test"});

        // Very small limit for testing
        let config = QueueConfig::new(2, 1024 * 1024);

        // Add up to the limit
        assert_eq!(
            store
                .add_pending_change_bounded(
                    "orders",
                    "order-1",
                    "INSERT",
                    Some(&data),
                    "{}",
                    &config
                )
                .unwrap(),
            QueueResult::Added
        );
        assert_eq!(
            store
                .add_pending_change_bounded(
                    "orders",
                    "order-2",
                    "INSERT",
                    Some(&data),
                    "{}",
                    &config
                )
                .unwrap(),
            QueueResult::Added
        );

        // Next one should be rejected
        assert_eq!(
            store
                .add_pending_change_bounded(
                    "orders",
                    "order-3",
                    "INSERT",
                    Some(&data),
                    "{}",
                    &config
                )
                .unwrap(),
            QueueResult::RejectedCountLimit
        );

        // Verify stats
        let stats = store.get_queue_stats().unwrap();
        assert_eq!(stats.count, 2);
    }

    #[test]
    fn test_bounded_queue_bytes_limit() {
        let mut store = create_test_store();

        // Data that will fit one at a time
        let medium_data = serde_json::json!({
            "content": "x".repeat(200)
        });

        // Bytes limit is 500, so one entry (~200 bytes) fits but two won't
        let config = QueueConfig::new(100, 500);

        // First one should succeed
        assert_eq!(
            store
                .add_pending_change_bounded(
                    "orders",
                    "order-1",
                    "INSERT",
                    Some(&medium_data),
                    "{}",
                    &config
                )
                .unwrap(),
            QueueResult::Added
        );

        // Second one should succeed (200 + 200 = 400 < 500)
        assert_eq!(
            store
                .add_pending_change_bounded(
                    "orders",
                    "order-2",
                    "INSERT",
                    Some(&medium_data),
                    "{}",
                    &config
                )
                .unwrap(),
            QueueResult::Added
        );

        // Third one should fail due to bytes limit (400 + 200 = 600 > 500)
        let result = store
            .add_pending_change_bounded(
                "orders",
                "order-3",
                "INSERT",
                Some(&medium_data),
                "{}",
                &config,
            )
            .unwrap();
        assert_eq!(result, QueueResult::RejectedBytesLimit);
    }

    #[test]
    fn test_remaining_capacity() {
        let mut store = create_test_store();
        let data = serde_json::json!({"name": "test"});

        let config = QueueConfig::new(10, 1024 * 1024);

        assert_eq!(store.remaining_capacity(&config).unwrap(), 10);

        store
            .add_pending_change("orders", "order-1", "INSERT", Some(&data), "{}")
            .unwrap();
        store
            .add_pending_change("orders", "order-2", "INSERT", Some(&data), "{}")
            .unwrap();

        assert_eq!(store.remaining_capacity(&config).unwrap(), 8);
    }

    #[test]
    fn test_can_accept_change() {
        let mut store = create_test_store();
        let data = serde_json::json!({"name": "test"});

        let config = QueueConfig::new(2, 1024 * 1024);

        assert!(store.can_accept_change(100, &config).unwrap());

        store
            .add_pending_change("orders", "order-1", "INSERT", Some(&data), "{}")
            .unwrap();
        store
            .add_pending_change("orders", "order-2", "INSERT", Some(&data), "{}")
            .unwrap();

        // Queue is now full by count
        assert!(!store.can_accept_change(100, &config).unwrap());
    }

    #[test]
    fn test_queue_config_presets() {
        let default = QueueConfig::default();
        assert_eq!(default.max_count, 10_000);
        assert_eq!(default.max_bytes, 100 * 1024 * 1024);

        let mobile = QueueConfig::mobile();
        assert_eq!(mobile.max_count, 1_000);
        assert_eq!(mobile.max_bytes, 10 * 1024 * 1024);

        let desktop = QueueConfig::desktop();
        assert_eq!(desktop.max_count, 50_000);
        assert_eq!(desktop.max_bytes, 500 * 1024 * 1024);
    }
}
