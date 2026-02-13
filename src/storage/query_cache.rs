//! Query result cache for caching frequently executed query results.
//!
//! This module provides caching for query results to improve read performance.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Configuration for query caching
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
    pub result: Vec<serde_json::Value>,
    pub cached_at: Instant,
}

/// Query cache with TTL support
pub struct QueryCache {
    cache: RwLock<HashMap<String, CachedQueryResult>>,
    max_entries: usize,
    ttl: Duration,
}

impl QueryCache {
    pub fn new(max_entries: usize, ttl_secs: u64) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            max_entries,
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub fn with_config(config: &QueryCacheConfig) -> Self {
        Self::new(config.max_entries, config.ttl_secs)
    }

    /// Get a cached query result
    pub async fn get(&self, query_hash: &str) -> Option<Vec<serde_json::Value>> {
        let cache = self.cache.read().await;
        if let Some(cached) = cache.get(query_hash) {
            // Check if expired
            if cached.cached_at.elapsed() < self.ttl {
                return Some(cached.result.clone());
            }
        }
        None
    }

    /// Store a query result in cache
    pub async fn put(&self, query_hash: String, result: Vec<serde_json::Value>) {
        let mut cache = self.cache.write().await;

        // Simple eviction: if over capacity, clear half the cache
        if cache.len() >= self.max_entries {
            let keys_to_remove: Vec<String> =
                cache.keys().take(self.max_entries / 2).cloned().collect();
            for key in keys_to_remove {
                cache.remove(&key);
            }
        }

        cache.insert(
            query_hash,
            CachedQueryResult {
                result,
                cached_at: Instant::now(),
            },
        );
    }

    /// Invalidate all cached results (e.g., after a write operation)
    pub async fn invalidate_all(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }

    /// Invalidate queries related to a specific collection
    pub async fn invalidate_collection(&self, collection_name: &str) {
        let mut cache = self.cache.write().await;
        let keys_to_remove: Vec<String> = cache
            .keys()
            .filter(|k| k.contains(collection_name))
            .cloned()
            .collect();
        for key in keys_to_remove {
            cache.remove(&key);
        }
    }

    /// Get cache statistics
    pub async fn stats(&self) -> QueryCacheStats {
        let cache = self.cache.read().await;
        QueryCacheStats {
            entries: cache.len(),
            max_entries: self.max_entries,
            ttl_secs: self.ttl.as_secs(),
        }
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

/// Generate a simple hash for a query to use as cache key
pub fn hash_query(
    query: &str,
    bind_vars: &std::collections::HashMap<String, serde_json::Value>,
) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    query.hash(&mut hasher);

    // Include bind vars in hash
    let mut sorted_vars: Vec<_> = bind_vars.iter().collect();
    sorted_vars.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in sorted_vars {
        k.hash(&mut hasher);
        v.hash(&mut hasher);
    }

    format!("{:x}", hasher.finish())
}
