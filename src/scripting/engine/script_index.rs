//! Script Index for Fast Lookup
//!
//! This module provides an in-memory index of scripts for O(1) lookup
//! during request routing, replacing the O(n) collection scan.

use dashmap::DashMap;
use std::sync::Arc;

use crate::scripting::types::Script;
use crate::storage::StorageEngine;

/// Index key for script lookup
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct IndexKey {
    database: String,
    service: String,
    path: String,
    method: String,
}

/// In-memory index for fast script lookup.
///
/// The original implementation scans the entire _scripts collection
/// for every request, which is O(n) where n is the number of scripts.
/// This index provides O(1) lookup by (database, service, path, method).
pub struct ScriptIndex {
    /// Map from (database, service, path, method) to Script
    /// We store exact paths and use a separate structure for patterns
    exact_paths: DashMap<IndexKey, Script>,
    /// Scripts with path parameters (e.g., "users/:id")
    /// Stored per (database, service) for pattern matching
    pattern_paths: DashMap<(String, String), Vec<Script>>,
}

impl Default for ScriptIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        Self {
            exact_paths: DashMap::new(),
            pattern_paths: DashMap::new(),
        }
    }

    /// Find a script matching the given request.
    ///
    /// First checks exact path matches, then falls back to pattern matching.
    pub fn find(&self, database: &str, service: &str, path: &str, method: &str) -> Option<Script> {
        // Normalize method to uppercase
        let method_upper = method.to_uppercase();

        // Try exact match first (most common case)
        let key = IndexKey {
            database: database.to_string(),
            service: service.to_string(),
            path: path.to_string(),
            method: method_upper.clone(),
        };

        if let Some(script) = self.exact_paths.get(&key) {
            return Some(script.clone());
        }

        // Try WebSocket method for WS upgrades
        if method_upper == "GET" {
            let ws_key = IndexKey {
                database: database.to_string(),
                service: service.to_string(),
                path: path.to_string(),
                method: "WS".to_string(),
            };
            if let Some(script) = self.exact_paths.get(&ws_key) {
                return Some(script.clone());
            }
        }

        // Fall back to pattern matching
        let pattern_key = (database.to_string(), service.to_string());
        if let Some(patterns) = self.pattern_paths.get(&pattern_key) {
            for script in patterns.iter() {
                if script
                    .methods
                    .iter()
                    .any(|m| m.eq_ignore_ascii_case(&method_upper) || m.eq_ignore_ascii_case("WS"))
                    && Self::path_matches(&script.path, path)
                {
                    return Some(script.clone());
                }
            }
        }

        None
    }

    /// Check if a script path pattern matches the actual path.
    fn path_matches(pattern: &str, path: &str) -> bool {
        let pattern_parts: Vec<&str> = pattern.split('/').collect();
        let path_parts: Vec<&str> = path.split('/').collect();

        if pattern_parts.len() != path_parts.len() {
            return false;
        }

        for (p, actual) in pattern_parts.iter().zip(path_parts.iter()) {
            if p.starts_with(':') {
                // Parameter - matches anything
                continue;
            }
            if *p != *actual {
                return false;
            }
        }

        true
    }

    /// Check if a path contains parameters.
    fn has_parameters(path: &str) -> bool {
        path.contains(':')
    }

    /// Add a script to the index.
    pub fn insert(&self, script: Script) {
        let database = script.database.clone();
        let service = script.service.clone();
        let path = script.path.clone();

        if Self::has_parameters(&path) {
            // Pattern path - add to pattern list
            let pattern_key = (database, service);
            self.pattern_paths
                .entry(pattern_key)
                .or_default()
                .push(script);
        } else {
            // Exact path - add for each method
            for method in &script.methods {
                let key = IndexKey {
                    database: script.database.clone(),
                    service: script.service.clone(),
                    path: path.clone(),
                    method: method.to_uppercase(),
                };
                self.exact_paths.insert(key, script.clone());
            }
        }
    }

    /// Remove a script from the index.
    pub fn remove(&self, script_key: &str, database: &str, service: &str) {
        // Remove from exact paths
        self.exact_paths.retain(|_, v| v.key != script_key);

        // Remove from pattern paths
        let pattern_key = (database.to_string(), service.to_string());
        if let Some(mut patterns) = self.pattern_paths.get_mut(&pattern_key) {
            patterns.retain(|s| s.key != script_key);
        }
    }

    /// Clear the entire index.
    pub fn clear(&self) {
        self.exact_paths.clear();
        self.pattern_paths.clear();
    }

    /// Rebuild the index from storage.
    ///
    /// This should be called:
    /// - On server startup
    /// - When scripts are created, updated, or deleted
    pub fn rebuild(&self, storage: &Arc<StorageEngine>) {
        self.clear();

        // Iterate all databases
        for db_name in storage.list_databases() {
            if let Ok(db) = storage.get_database(&db_name) {
                // Try to get _scripts collection
                if let Ok(collection) = db.get_collection("_scripts") {
                    for doc in collection.scan(None) {
                        if let Ok(script) = serde_json::from_value::<Script>(doc.to_value()) {
                            self.insert(script);
                        }
                    }
                }
            }
        }
    }

    /// Rebuild the index for a specific database.
    pub fn rebuild_database(&self, storage: &Arc<StorageEngine>, database: &str) {
        // Remove existing entries for this database
        self.exact_paths.retain(|k, _| k.database != database);
        self.pattern_paths.retain(|k, _| k.0 != database);

        // Re-index from storage
        if let Ok(db) = storage.get_database(database) {
            if let Ok(collection) = db.get_collection("_scripts") {
                for doc in collection.scan(None) {
                    if let Ok(script) = serde_json::from_value::<Script>(doc.to_value()) {
                        if script.database == database {
                            self.insert(script);
                        }
                    }
                }
            }
        }
    }

    /// Get index statistics.
    pub fn stats(&self) -> IndexStats {
        let mut pattern_count = 0;
        let mut databases = std::collections::HashSet::new();
        for entry in self.pattern_paths.iter() {
            pattern_count += entry.value().len();
            databases.insert(entry.key().0.clone());
        }
        for entry in self.exact_paths.iter() {
            databases.insert(entry.key().database.clone());
        }

        IndexStats {
            exact_entries: self.exact_paths.len(),
            pattern_entries: pattern_count,
            databases: databases.len(),
        }
    }
}

/// Index statistics
#[derive(Debug, Clone)]
pub struct IndexStats {
    /// Number of exact path entries
    pub exact_entries: usize,
    /// Number of pattern path entries
    pub pattern_entries: usize,
    /// Number of databases with indexed scripts
    pub databases: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_script(key: &str, database: &str, path: &str, methods: Vec<&str>) -> Script {
        make_script_with_service(key, database, "default", path, methods)
    }

    fn make_script_with_service(
        key: &str,
        database: &str,
        service: &str,
        path: &str,
        methods: Vec<&str>,
    ) -> Script {
        Script {
            key: key.to_string(),
            name: key.to_string(),
            methods: methods.into_iter().map(|s| s.to_string()).collect(),
            path: path.to_string(),
            database: database.to_string(),
            service: service.to_string(),
            collection: None,
            code: "return {}".to_string(),
            description: None,
            created_at: "2024-01-01".to_string(),
            updated_at: "2024-01-01".to_string(),
        }
    }

    #[test]
    fn test_exact_path_lookup() {
        let index = ScriptIndex::new();

        let script = make_script("s1", "testdb", "hello", vec!["GET"]);
        index.insert(script.clone());

        let found = index.find("testdb", "default", "hello", "GET");
        assert!(found.is_some());
        assert_eq!(found.unwrap().key, "s1");

        // Wrong method
        let found = index.find("testdb", "default", "hello", "POST");
        assert!(found.is_none());

        // Wrong database
        let found = index.find("otherdb", "default", "hello", "GET");
        assert!(found.is_none());

        // Wrong service
        let found = index.find("testdb", "other", "hello", "GET");
        assert!(found.is_none());
    }

    #[test]
    fn test_pattern_path_lookup() {
        let index = ScriptIndex::new();

        let script = make_script("s1", "testdb", "users/:id", vec!["GET"]);
        index.insert(script);

        let found = index.find("testdb", "default", "users/123", "GET");
        assert!(found.is_some());
        assert_eq!(found.unwrap().key, "s1");

        let found = index.find("testdb", "default", "users/456", "GET");
        assert!(found.is_some());

        // Wrong path structure
        let found = index.find("testdb", "default", "users/123/posts", "GET");
        assert!(found.is_none());
    }

    #[test]
    fn test_multiple_methods() {
        let index = ScriptIndex::new();

        let script = make_script("s1", "testdb", "api", vec!["GET", "POST"]);
        index.insert(script);

        assert!(index.find("testdb", "default", "api", "GET").is_some());
        assert!(index.find("testdb", "default", "api", "POST").is_some());
        assert!(index.find("testdb", "default", "api", "DELETE").is_none());
    }

    #[test]
    fn test_remove() {
        let index = ScriptIndex::new();

        let script1 = make_script("s1", "testdb", "api1", vec!["GET"]);
        let script2 = make_script("s2", "testdb", "api2", vec!["GET"]);
        index.insert(script1);
        index.insert(script2);

        assert!(index.find("testdb", "default", "api1", "GET").is_some());
        assert!(index.find("testdb", "default", "api2", "GET").is_some());

        index.remove("s1", "testdb", "default");

        assert!(index.find("testdb", "default", "api1", "GET").is_none());
        assert!(index.find("testdb", "default", "api2", "GET").is_some());
    }

    #[test]
    fn test_stats() {
        let index = ScriptIndex::new();

        // Add exact path
        index.insert(make_script("s1", "db1", "api", vec!["GET"]));
        // Add pattern path
        index.insert(make_script("s2", "db1", "users/:id", vec!["GET"]));
        // Add another pattern in different db
        index.insert(make_script("s3", "db2", "items/:id", vec!["GET", "POST"]));

        let stats = index.stats();
        assert_eq!(stats.exact_entries, 1);
        assert_eq!(stats.pattern_entries, 2);
        assert_eq!(stats.databases, 2);
    }

    #[test]
    fn test_path_matches() {
        assert!(ScriptIndex::path_matches("hello", "hello"));
        assert!(ScriptIndex::path_matches("users/:id", "users/123"));
        assert!(ScriptIndex::path_matches(
            "posts/:id/comments/:cid",
            "posts/1/comments/2"
        ));

        assert!(!ScriptIndex::path_matches("hello", "world"));
        assert!(!ScriptIndex::path_matches("users/:id", "users/123/extra"));
        assert!(!ScriptIndex::path_matches("a/b", "a/b/c"));
    }

    #[test]
    fn test_ws_method() {
        let index = ScriptIndex::new();

        let script = make_script("s1", "testdb", "chat", vec!["WS"]);
        index.insert(script);

        // GET request should match WS script (for upgrade)
        let found = index.find("testdb", "default", "chat", "GET");
        assert!(found.is_some());
    }

    #[test]
    fn test_method_case_insensitivity() {
        let index = ScriptIndex::new();

        let script = make_script("s1", "db", "api", vec!["GET"]);
        index.insert(script);

        // All case variants should match
        assert!(index.find("db", "default", "api", "GET").is_some());
        assert!(index.find("db", "default", "api", "get").is_some());
        assert!(index.find("db", "default", "api", "Get").is_some());
        assert!(index.find("db", "default", "api", "gEt").is_some());
    }

    #[test]
    fn test_complex_path_pattern() {
        let index = ScriptIndex::new();

        // Three-level parameter path
        let script = make_script(
            "s1",
            "db",
            "posts/:pid/comments/:cid/replies/:rid",
            vec!["GET"],
        );
        index.insert(script);

        // Should match various ID combinations
        assert!(index
            .find("db", "default", "posts/123/comments/456/replies/789", "GET")
            .is_some());
        assert!(index
            .find("db", "default", "posts/abc/comments/xyz/replies/def", "GET")
            .is_some());
        assert!(index
            .find("db", "default", "posts/1/comments/2/replies/3", "GET")
            .is_some());

        // Should NOT match wrong structure
        assert!(index
            .find("db", "default", "posts/123/comments/456", "GET")
            .is_none());
        assert!(index
            .find("db", "default", "posts/123/comments/456/replies", "GET")
            .is_none());
        assert!(index
            .find(
                "db",
                "default",
                "posts/123/comments/456/replies/789/extra",
                "GET"
            )
            .is_none());
    }

    #[test]
    fn test_insert_duplicate() {
        let index = ScriptIndex::new();

        let script = make_script("s1", "db", "api", vec!["GET"]);

        // Insert same script twice
        index.insert(script.clone());
        index.insert(script.clone());

        // Should still find it
        assert!(index.find("db", "default", "api", "GET").is_some());

        // Stats might show duplicate entries (implementation detail)
        let stats = index.stats();
        assert!(stats.exact_entries >= 1);
    }

    #[test]
    fn test_remove_nonexistent() {
        let index = ScriptIndex::new();

        let script = make_script("s1", "db", "api", vec!["GET"]);
        index.insert(script);

        // Remove non-existent key - should not panic
        index.remove("nonexistent", "db", "default");
        index.remove("nonexistent", "other_db", "default");

        // Original should still be there
        assert!(index.find("db", "default", "api", "GET").is_some());
    }

    #[test]
    fn test_concurrent_find_insert_remove() {
        use std::sync::Arc;
        use std::thread;

        let index = Arc::new(ScriptIndex::new());

        // Pre-populate with some scripts
        for i in 0..10 {
            let script = make_script(&format!("s{}", i), "db", &format!("api{}", i), vec!["GET"]);
            index.insert(script);
        }

        let handles: Vec<_> = (0..4)
            .map(|thread_id| {
                let idx = index.clone();
                thread::spawn(move || {
                    for i in 0i32..100 {
                        match (thread_id + i) % 3 {
                            0 => {
                                // Find operations
                                let _ = idx.find("db", "default", &format!("api{}", i % 10), "GET");
                            }
                            1 => {
                                // Insert operations
                                let script = make_script(
                                    &format!("new_s{}_{}", thread_id, i),
                                    "db",
                                    &format!("new_api{}_{}", thread_id, i),
                                    vec!["POST"],
                                );
                                idx.insert(script);
                            }
                            2 => {
                                // Remove operations (on newly added scripts)
                                idx.remove(
                                    &format!("new_s{}_{}", thread_id, i.saturating_sub(1)),
                                    "db",
                                    "default",
                                );
                            }
                            _ => unreachable!(),
                        }
                    }
                })
            })
            .collect();

        for h in handles {
            h.join()
                .expect("Thread panicked during concurrent operations");
        }

        // Original scripts should still be findable (we didn't remove them)
        for i in 0..10 {
            assert!(
                index
                    .find("db", "default", &format!("api{}", i), "GET")
                    .is_some(),
                "Original script api{} should still exist",
                i
            );
        }
    }

    #[test]
    fn test_pattern_with_single_segment() {
        let index = ScriptIndex::new();

        // Single segment pattern
        let script = make_script("s1", "db", ":id", vec!["GET"]);
        index.insert(script);

        assert!(index.find("db", "default", "123", "GET").is_some());
        assert!(index.find("db", "default", "abc", "GET").is_some());

        // Multiple segments should not match
        assert!(index.find("db", "default", "123/456", "GET").is_none());
    }

    #[test]
    fn test_empty_path() {
        let index = ScriptIndex::new();

        let script = make_script("s1", "db", "", vec!["GET"]);
        index.insert(script);

        assert!(index.find("db", "default", "", "GET").is_some());
        assert!(index.find("db", "default", "something", "GET").is_none());
    }

    #[test]
    fn test_multiple_databases() {
        let index = ScriptIndex::new();

        // Same path in different databases
        index.insert(make_script("s1", "db1", "api", vec!["GET"]));
        index.insert(make_script("s2", "db2", "api", vec!["GET"]));
        index.insert(make_script("s3", "db3", "users/:id", vec!["GET"]));
        index.insert(make_script("s4", "db3", "users/:id", vec!["POST"]));

        // Each database should have its own script
        let found1 = index.find("db1", "default", "api", "GET");
        let found2 = index.find("db2", "default", "api", "GET");
        let found3 = index.find("db3", "default", "users/123", "GET");
        let found4 = index.find("db3", "default", "users/123", "POST");

        assert!(found1.is_some());
        assert!(found2.is_some());
        assert!(found3.is_some());
        assert!(found4.is_some());

        assert_eq!(found1.unwrap().key, "s1");
        assert_eq!(found2.unwrap().key, "s2");
        assert_eq!(found3.unwrap().key, "s3");
        assert_eq!(found4.unwrap().key, "s4");
    }

    #[test]
    fn test_clear() {
        let index = ScriptIndex::new();

        index.insert(make_script("s1", "db", "api1", vec!["GET"]));
        index.insert(make_script("s2", "db", "users/:id", vec!["GET"]));

        let stats = index.stats();
        assert!(stats.exact_entries > 0 || stats.pattern_entries > 0);

        index.clear();

        let stats = index.stats();
        assert_eq!(stats.exact_entries, 0);
        assert_eq!(stats.pattern_entries, 0);

        assert!(index.find("db", "default", "api1", "GET").is_none());
        assert!(index.find("db", "default", "users/123", "GET").is_none());
    }

    #[test]
    fn test_service_isolation() {
        let index = ScriptIndex::new();

        // Same path in different services
        index.insert(make_script_with_service(
            "s1",
            "db",
            "users",
            "hello",
            vec!["GET"],
        ));
        index.insert(make_script_with_service(
            "s2",
            "db",
            "admin",
            "hello",
            vec!["GET"],
        ));

        // Each service should have its own script
        let found1 = index.find("db", "users", "hello", "GET");
        let found2 = index.find("db", "admin", "hello", "GET");
        let found3 = index.find("db", "other", "hello", "GET");

        assert!(found1.is_some());
        assert!(found2.is_some());
        assert!(found3.is_none());

        assert_eq!(found1.unwrap().key, "s1");
        assert_eq!(found2.unwrap().key, "s2");
    }

    #[test]
    fn test_service_pattern_isolation() {
        let index = ScriptIndex::new();

        // Pattern paths in different services
        index.insert(make_script_with_service(
            "s1",
            "db",
            "users",
            "items/:id",
            vec!["GET"],
        ));
        index.insert(make_script_with_service(
            "s2",
            "db",
            "admin",
            "items/:id",
            vec!["GET"],
        ));

        let found1 = index.find("db", "users", "items/123", "GET");
        let found2 = index.find("db", "admin", "items/123", "GET");
        let found3 = index.find("db", "other", "items/123", "GET");

        assert!(found1.is_some());
        assert!(found2.is_some());
        assert!(found3.is_none());

        assert_eq!(found1.unwrap().key, "s1");
        assert_eq!(found2.unwrap().key, "s2");
    }
}
