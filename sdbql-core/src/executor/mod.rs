//! Executor module for SDBQL queries.
//!
//! This module provides a trait-based executor that can run SDBQL queries
//! against any data source implementing the DataSource trait.

mod builtins;
mod helpers;
mod local;

pub use builtins::BuiltinFunctions;
pub use helpers::*;
pub use local::LocalExecutor;

use serde_json::Value;
use std::collections::HashMap;

/// Trait for data sources that can be queried by the executor.
///
/// Implement this trait to enable SDBQL queries against your data store.
/// Note: `Send` is required for cross-thread use; `Sync` is not required
/// since queries are typically executed within a mutex guard.
pub trait DataSource {
    /// Scan all documents in a collection.
    ///
    /// # Arguments
    /// * `collection` - Name of the collection to scan
    /// * `limit` - Optional limit on number of documents to return
    ///
    /// # Returns
    /// Vec of document values
    fn scan(&self, collection: &str, limit: Option<usize>) -> Vec<Value>;

    /// Get a single document by key.
    ///
    /// # Arguments
    /// * `collection` - Name of the collection
    /// * `key` - Document key
    ///
    /// # Returns
    /// The document if found, None otherwise
    fn get(&self, collection: &str, key: &str) -> Option<Value>;

    /// Check if a collection exists.
    ///
    /// # Arguments
    /// * `name` - Collection name
    ///
    /// # Returns
    /// true if the collection exists
    fn collection_exists(&self, name: &str) -> bool;

    /// List all collection names.
    ///
    /// # Returns
    /// Vec of collection names
    fn list_collections(&self) -> Vec<String> {
        vec![]
    }
}

/// Bind variables for parameterized queries
pub type BindVars = HashMap<String, Value>;

/// Configuration for query execution limits
#[derive(Debug, Clone)]
pub struct QueryLimits {
    /// Maximum number of documents to scan (default: 10,000)
    pub max_scan_docs: usize,
    /// Maximum result size in bytes (default: 1MB)
    pub max_result_size: usize,
    /// Maximum execution time in milliseconds (default: 5000)
    pub max_execution_time_ms: u64,
}

impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            max_scan_docs: 10_000,
            max_result_size: 1024 * 1024, // 1MB
            max_execution_time_ms: 5000,
        }
    }
}

impl QueryLimits {
    /// Create limits suitable for mobile devices
    pub fn mobile() -> Self {
        Self {
            max_scan_docs: 1_000,
            max_result_size: 256 * 1024, // 256KB
            max_execution_time_ms: 2000,
        }
    }

    /// Create limits suitable for desktop applications
    pub fn desktop() -> Self {
        Self {
            max_scan_docs: 50_000,
            max_result_size: 10 * 1024 * 1024, // 10MB
            max_execution_time_ms: 30000,
        }
    }
}

/// In-memory data source for testing
#[derive(Default)]
pub struct InMemoryDataSource {
    collections: HashMap<String, HashMap<String, Value>>,
}

impl InMemoryDataSource {
    /// Create a new empty in-memory data source
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a collection with documents
    pub fn add_collection(&mut self, name: &str, docs: Vec<Value>) {
        let mut collection = HashMap::new();
        for (i, doc) in docs.into_iter().enumerate() {
            let key = doc
                .get("_key")
                .and_then(|k| k.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("doc{}", i));
            collection.insert(key, doc);
        }
        self.collections.insert(name.to_string(), collection);
    }

    /// Insert a single document
    pub fn insert(&mut self, collection: &str, key: &str, doc: Value) {
        self.collections
            .entry(collection.to_string())
            .or_default()
            .insert(key.to_string(), doc);
    }
}

impl DataSource for InMemoryDataSource {
    fn scan(&self, collection: &str, limit: Option<usize>) -> Vec<Value> {
        if let Some(coll) = self.collections.get(collection) {
            let docs: Vec<Value> = coll.values().cloned().collect();
            if let Some(limit) = limit {
                docs.into_iter().take(limit).collect()
            } else {
                docs
            }
        } else {
            vec![]
        }
    }

    fn get(&self, collection: &str, key: &str) -> Option<Value> {
        self.collections
            .get(collection)
            .and_then(|c| c.get(key).cloned())
    }

    fn collection_exists(&self, name: &str) -> bool {
        self.collections.contains_key(name)
    }

    fn list_collections(&self) -> Vec<String> {
        self.collections.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_in_memory_data_source() {
        let mut ds = InMemoryDataSource::new();
        ds.add_collection(
            "users",
            vec![
                json!({"_key": "1", "name": "Alice", "age": 30}),
                json!({"_key": "2", "name": "Bob", "age": 25}),
            ],
        );

        assert!(ds.collection_exists("users"));
        assert!(!ds.collection_exists("orders"));

        let docs = ds.scan("users", None);
        assert_eq!(docs.len(), 2);

        let doc = ds.get("users", "1");
        assert!(doc.is_some());
        assert_eq!(doc.unwrap()["name"], "Alice");
    }

    #[test]
    fn test_scan_with_limit() {
        let mut ds = InMemoryDataSource::new();
        ds.add_collection(
            "items",
            vec![json!({"x": 1}), json!({"x": 2}), json!({"x": 3})],
        );

        let docs = ds.scan("items", Some(2));
        assert_eq!(docs.len(), 2);
    }
}
