//! SQLite data source implementation for local SDBQL query execution.
//!
//! This module provides a `DataSource` implementation that reads from the
//! local SQLite store used for offline synchronization.

use sdbql_core::DataSource;
use serde_json::Value;

use super::store::LocalStore;

/// SQLite-based data source that implements the `DataSource` trait.
///
/// This allows executing SDBQL queries against the local offline store.
pub struct SqliteDataSource<'a> {
    store: &'a LocalStore,
}

impl<'a> SqliteDataSource<'a> {
    /// Create a new SQLite data source backed by a LocalStore.
    pub fn new(store: &'a LocalStore) -> Self {
        Self { store }
    }
}

impl<'a> DataSource for SqliteDataSource<'a> {
    /// Scan all documents in a collection.
    ///
    /// # Arguments
    /// * `collection` - Collection name to scan
    /// * `limit` - Optional limit on number of documents to return
    fn scan(&self, collection: &str, limit: Option<usize>) -> Vec<Value> {
        match self.store.list_documents(collection) {
            Ok(docs) => {
                let mut values: Vec<Value> = docs
                    .into_iter()
                    .map(|(key, mut data, _version)| {
                        // Add _key field to document if it's an object
                        if let Value::Object(ref mut obj) = data {
                            obj.insert("_key".to_string(), Value::String(key.clone()));
                            // Also add _id for compatibility
                            obj.insert(
                                "_id".to_string(),
                                Value::String(format!("{}/{}", collection, key)),
                            );
                        }
                        data
                    })
                    .collect();

                if let Some(limit) = limit {
                    values.truncate(limit);
                }

                values
            }
            Err(e) => {
                tracing::warn!("Failed to scan collection {}: {}", collection, e);
                Vec::new()
            }
        }
    }

    /// Get a single document by key.
    ///
    /// # Arguments
    /// * `collection` - Collection name
    /// * `key` - Document key
    fn get(&self, collection: &str, key: &str) -> Option<Value> {
        match self.store.get_document(collection, key) {
            Ok(Some((mut data, _version))) => {
                // Add _key field to document if it's an object
                if let Value::Object(ref mut obj) = data {
                    obj.insert("_key".to_string(), Value::String(key.to_string()));
                    obj.insert(
                        "_id".to_string(),
                        Value::String(format!("{}/{}", collection, key)),
                    );
                }
                Some(data)
            }
            Ok(None) => None,
            Err(e) => {
                tracing::warn!("Failed to get document {}/{}: {}", collection, key, e);
                None
            }
        }
    }

    /// Check if a collection exists.
    ///
    /// For SQLite, we consider a collection to exist if there are any
    /// documents in it (including deleted ones tracked for sync).
    fn collection_exists(&self, name: &str) -> bool {
        // Check if there are any documents (including deleted) in the collection
        match self.store.list_documents(name) {
            Ok(docs) => !docs.is_empty(),
            Err(_) => false,
        }
    }

    /// List all collections that have documents.
    fn list_collections(&self) -> Vec<String> {
        match self.store.list_collections() {
            Ok(collections) => collections,
            Err(e) => {
                tracing::warn!("Failed to list collections: {}", e);
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn create_test_store() -> (tempfile::TempDir, LocalStore) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let store = LocalStore::open(&db_path, "test-device".to_string()).unwrap();
        (dir, store)
    }

    #[test]
    fn test_scan_empty_collection() {
        let (_dir, store) = create_test_store();
        let ds = SqliteDataSource::new(&store);

        let results = ds.scan("users", None);
        assert!(results.is_empty());
    }

    #[test]
    fn test_scan_with_documents() {
        let (_dir, mut store) = create_test_store();

        store
            .put_document("users", "1", &json!({"name": "Alice"}), "{}")
            .unwrap();
        store
            .put_document("users", "2", &json!({"name": "Bob"}), "{}")
            .unwrap();

        let ds = SqliteDataSource::new(&store);

        let results = ds.scan("users", None);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_scan_with_limit() {
        let (_dir, mut store) = create_test_store();

        store
            .put_document("users", "1", &json!({"name": "Alice"}), "{}")
            .unwrap();
        store
            .put_document("users", "2", &json!({"name": "Bob"}), "{}")
            .unwrap();
        store
            .put_document("users", "3", &json!({"name": "Charlie"}), "{}")
            .unwrap();

        let ds = SqliteDataSource::new(&store);

        let results = ds.scan("users", Some(2));
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_get_document() {
        let (_dir, mut store) = create_test_store();

        store
            .put_document("users", "1", &json!({"name": "Alice"}), "{}")
            .unwrap();

        let ds = SqliteDataSource::new(&store);

        let doc = ds.get("users", "1");
        assert!(doc.is_some());
        let doc = doc.unwrap();
        assert_eq!(doc.get("name"), Some(&json!("Alice")));
        assert_eq!(doc.get("_key"), Some(&json!("1")));
        assert_eq!(doc.get("_id"), Some(&json!("users/1")));
    }

    #[test]
    fn test_get_missing_document() {
        let (_dir, store) = create_test_store();
        let ds = SqliteDataSource::new(&store);

        let doc = ds.get("users", "nonexistent");
        assert!(doc.is_none());
    }

    #[test]
    fn test_collection_exists() {
        let (_dir, mut store) = create_test_store();

        // Check before adding documents
        {
            let ds = SqliteDataSource::new(&store);
            assert!(!ds.collection_exists("users"));
        }

        // Add a document
        store
            .put_document("users", "1", &json!({"name": "Alice"}), "{}")
            .unwrap();

        // Check after adding documents
        {
            let ds = SqliteDataSource::new(&store);
            assert!(ds.collection_exists("users"));
        }
    }
}
