//! Query result cache for caching frequently executed query results.
//!
//! This module provides caching for query results to improve read performance.
//!
//! Concurrency model:
//! - One `parking_lot::RwLock` guards both the entry map and the
//!   per-collection invalidation index, so they can never diverge (a put
//!   racing an invalidate is fully serialized) and reads stay cheap and
//!   non-async on the request hot path (no `.await` suspension).
//! - Each entry remembers the collections it references, so eviction and
//!   invalidation only touch the index sets they belong to — O(matching
//!   entries), not O(total cache).

use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Configuration for query caching
/// Largest result set worth caching, in rows. The cache evicts by entry
/// count and TTL, never by bytes, so without this one client varying the
/// query text could park a thousand multi-hundred-megabyte result sets.
pub const MAX_CACHED_ROWS: usize = 10_000;

#[derive(Debug, Clone)]
pub struct QueryCacheConfig {
    /// Maximum number of queries to cache
    pub max_entries: usize,
    /// Time-to-live for cached query results
    pub ttl_secs: u64,
}

impl Default for QueryCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 1_000,
            ttl_secs: 60, // 1 minute default TTL
        }
    }
}

/// A cached query result with metadata
pub struct CachedQueryResult {
    pub result: Arc<Vec<serde_json::Value>>,
    pub cached_at: Instant,
    /// Collections this entry references (parsed from the cache key at
    /// `put` time) so removal can prune only the matching index sets.
    collections: Vec<String>,
}

/// Entry map + per-collection invalidation index, guarded by a single lock.
#[derive(Default)]
struct Inner {
    entries: HashMap<String, CachedQueryResult>,
    /// collection_name -> set of cache keys referencing that collection
    by_collection: HashMap<String, HashSet<String>>,
}

impl Inner {
    /// Remove an entry and prune it from its collections' index sets,
    /// dropping index sets that become empty.
    fn remove_entry(&mut self, key: &str) {
        if let Some(entry) = self.entries.remove(key) {
            for coll in &entry.collections {
                let emptied = match self.by_collection.get_mut(coll) {
                    Some(set) => {
                        set.remove(key);
                        set.is_empty()
                    }
                    None => false,
                };
                if emptied {
                    self.by_collection.remove(coll);
                }
            }
        }
    }
}

/// Query cache with TTL support and per-collection invalidation index.
pub struct QueryCache {
    inner: RwLock<Inner>,
    max_entries: usize,
    ttl: Duration,
}

impl QueryCache {
    pub fn new(max_entries: usize, ttl_secs: u64) -> Self {
        Self {
            inner: RwLock::new(Inner::default()),
            max_entries,
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub fn with_config(config: &QueryCacheConfig) -> Self {
        Self::new(config.max_entries, config.ttl_secs)
    }

    /// Get a cached query result. Synchronous and non-blocking; safe to call
    /// from request hot paths.
    pub fn get(&self, query_hash: &str) -> Option<Arc<Vec<serde_json::Value>>> {
        let inner = self.inner.read();
        if let Some(cached) = inner.entries.get(query_hash) {
            if cached.cached_at.elapsed() < self.ttl {
                return Some(cached.result.clone());
            }
        }
        None
    }

    /// Store a query result in cache. Parses the cache key to maintain the
    /// per-collection invalidation index.
    pub fn put(&self, query_hash: String, result: Vec<serde_json::Value>) {
        let collections = extract_collections_from_key(&query_hash);

        let mut inner = self.inner.write();

        // Simple eviction: if over capacity, clear half the cache. Each
        // removal prunes only the index sets the entry belongs to.
        if inner.entries.len() >= self.max_entries {
            let keys_to_remove: Vec<String> = inner
                .entries
                .keys()
                .take(self.max_entries / 2)
                .cloned()
                .collect();
            for key in &keys_to_remove {
                inner.remove_entry(key);
            }
        }

        for coll in &collections {
            inner
                .by_collection
                .entry(coll.clone())
                .or_default()
                .insert(query_hash.clone());
        }
        inner.entries.insert(
            query_hash,
            CachedQueryResult {
                result: Arc::new(result),
                cached_at: Instant::now(),
                collections,
            },
        );
    }

    /// Invalidate all cached results.
    pub fn invalidate_all(&self) {
        let mut inner = self.inner.write();
        inner.entries.clear();
        inner.by_collection.clear();
    }

    /// Invalidate queries related to a specific collection. O(matches)
    /// instead of the previous O(total cache) scan.
    pub fn invalidate_collection(&self, collection_name: &str) {
        let mut inner = self.inner.write();
        let Some(keys_to_remove) = inner.by_collection.remove(collection_name) else {
            return;
        };
        for key in &keys_to_remove {
            // Also prunes cross-references from other collections' sets.
            inner.remove_entry(key);
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> QueryCacheStats {
        QueryCacheStats {
            entries: self.inner.read().entries.len(),
            max_entries: self.max_entries,
            ttl_secs: self.ttl.as_secs(),
        }
    }

    /// Number of collections currently tracked by the invalidation index
    /// (test/observability helper).
    #[cfg(test)]
    fn index_len(&self) -> usize {
        self.inner.read().by_collection.len()
    }
}

impl Default for QueryCache {
    fn default() -> Self {
        Self::new(1_000, 60)
    }
}

#[derive(Debug, Clone)]
pub struct QueryCacheStats {
    pub entries: usize,
    pub max_entries: usize,
    pub ttl_secs: u64,
}

/// Global query cache instance
static QUERY_CACHE: std::sync::OnceLock<QueryCache> = std::sync::OnceLock::new();

pub fn init_query_cache(config: &QueryCacheConfig) {
    let _ = QUERY_CACHE.set(QueryCache::with_config(config));
}

pub fn get_query_cache() -> &'static QueryCache {
    QUERY_CACHE.get_or_init(QueryCache::default)
}

/// Extract the collection names from a cache key built by `hash_query`.
/// Key format: `"db/coll1,coll2:hash"` (collections part may be empty).
fn extract_collections_from_key(key: &str) -> Vec<String> {
    let Some(slash_pos) = key.find('/') else {
        return vec![];
    };
    let after_slash = &key[slash_pos + 1..];
    let Some(colon_pos) = after_slash.rfind(':') else {
        return vec![];
    };
    let colls_str = &after_slash[..colon_pos];
    if colls_str.is_empty() {
        return vec![];
    }
    colls_str.split(',').map(|s| s.to_string()).collect()
}

/// Generate a cache key for a query that includes the database name and
/// collection names for targeted invalidation.
///
/// The key format is `"db/coll1,coll2:hash"` so the cache is partitioned per
/// database (same query text on different databases must not collide) and
/// `invalidate_collection` can still match by collection name.
pub fn hash_query(
    db_name: &str,
    query: &str,
    bind_vars: &std::collections::HashMap<String, serde_json::Value>,
) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    db_name.hash(&mut hasher);
    query.hash(&mut hasher);

    // Include bind vars in hash
    let mut sorted_vars: Vec<_> = bind_vars.iter().collect();
    sorted_vars.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in sorted_vars {
        k.hash(&mut hasher);
        v.hash(&mut hasher);
    }

    // Extract collection names from query (appears after IN keyword in SDBQL)
    let mut collections = Vec::new();
    let upper = query.to_uppercase();
    for (i, _) in upper.match_indices(" IN ") {
        // Get the word after " IN "
        let after = &query[i + 4..];
        if let Some(name) = after.split_whitespace().next() {
            // Skip bind variables (@var) and subqueries (starting with '(')
            if !name.starts_with('@') && !name.starts_with('(') {
                collections.push(name.to_string());
            }
        }
    }
    collections.sort();
    collections.dedup();

    if collections.is_empty() {
        format!("{}/:{:x}", db_name, hasher.finish())
    } else {
        format!(
            "{}/{}:{:x}",
            db_name,
            collections.join(","),
            hasher.finish()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_put_and_get() {
        let cache = QueryCache::new(10, 60);
        cache.put("db/coll:abc".to_string(), vec![json!({"a": 1})]);
        let got = cache.get("db/coll:abc");
        assert!(got.is_some());
        assert_eq!(got.unwrap().len(), 1);
    }

    #[test]
    fn test_get_missing() {
        let cache = QueryCache::new(10, 60);
        assert!(cache.get("db/coll:missing").is_none());
    }

    #[test]
    fn test_invalidate_collection() {
        let cache = QueryCache::new(10, 60);
        cache.put("db/users:1".to_string(), vec![json!({"a": 1})]);
        cache.put("db/orders:2".to_string(), vec![json!({"b": 2})]);
        cache.put("db/users,orders:3".to_string(), vec![json!({"c": 3})]);

        cache.invalidate_collection("users");
        // Key referencing only users: gone
        assert!(cache.get("db/users:1").is_none());
        // Key referencing only orders: still there
        assert!(cache.get("db/orders:2").is_some());
        // Key referencing both: also gone (it referenced users)
        assert!(cache.get("db/users,orders:3").is_none());
    }

    #[test]
    fn test_invalidate_all() {
        let cache = QueryCache::new(10, 60);
        cache.put("db/users:1".to_string(), vec![json!({"a": 1})]);
        cache.put("db/orders:2".to_string(), vec![json!({"b": 2})]);
        cache.invalidate_all();
        assert!(cache.get("db/users:1").is_none());
        assert!(cache.get("db/orders:2").is_none());
    }

    #[test]
    fn test_extract_collections() {
        let mut got = extract_collections_from_key("db/users,orders:abc");
        got.sort();
        assert_eq!(got, vec!["orders".to_string(), "users".to_string()]);
        assert_eq!(
            extract_collections_from_key("db/:abc"),
            Vec::<String>::new()
        );
        assert_eq!(
            extract_collections_from_key("no_slash"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn test_eviction() {
        let cache = QueryCache::new(2, 60);
        cache.put("db/a:1".to_string(), vec![json!({"a": 1})]);
        cache.put("db/b:2".to_string(), vec![json!({"b": 2})]);
        // This triggers eviction of half the entries.
        cache.put("db/c:3".to_string(), vec![json!({"c": 3})]);
        // We don't assert which key was evicted, only that the cache still works.
        let stats = cache.stats();
        assert!(stats.entries <= 2);
        // The invalidation index never holds more collections than live
        // entries reference (evicted entries are pruned, empty sets dropped).
        assert!(cache.index_len() <= stats.entries);
    }

    #[test]
    fn test_invalidate_prunes_index() {
        let cache = QueryCache::new(10, 60);
        cache.put("db/users:1".to_string(), vec![json!({"a": 1})]);
        cache.put("db/users,orders:2".to_string(), vec![json!({"b": 2})]);
        assert_eq!(cache.index_len(), 2); // users + orders

        cache.invalidate_collection("users");
        // Both entries referenced users, so the orders set emptied out and
        // its index entry must be gone too (no leak of empty sets).
        assert_eq!(cache.index_len(), 0);
        assert_eq!(cache.stats().entries, 0);
    }

    #[test]
    fn test_hash_query_format() {
        let key = hash_query(
            "mydb",
            "FOR doc IN users RETURN doc",
            &std::collections::HashMap::new(),
        );
        assert!(key.starts_with("mydb/users:"));
    }
}
