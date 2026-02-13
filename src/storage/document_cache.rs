//! Document cache for fast read access to frequently accessed documents.
//!
//! This module provides an LRU cache for caching document reads.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for document caching
#[derive(Debug, Clone)]
pub struct DocumentCacheConfig {
    /// Maximum number of documents to cache
    pub max_entries: usize,
}

impl Default for DocumentCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
        }
    }
}

/// Simple document cache with manual LRU tracking using HashMap
pub struct DocumentCache {
    cache: RwLock<HashMap<String, Arc<serde_json::Value>>>,
    access_order: RwLock<Vec<String>>,
    max_entries: usize,
}

impl DocumentCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            access_order: RwLock::new(Vec::new()),
            max_entries,
        }
    }

    pub fn with_config(config: &DocumentCacheConfig) -> Self {
        Self::new(config.max_entries)
    }

    /// Get a document from cache
    pub async fn get(&self, key: &str) -> Option<Arc<serde_json::Value>> {
        // Check if key exists and move to front of access order
        let mut access_order = self.access_order.write().await;
        if let Some(pos) = access_order.iter().position(|k| k == key) {
            access_order.remove(pos);
            access_order.push(key.to_string());
        }

        let cache = self.cache.read().await;
        cache.get(key).cloned()
    }

    /// Put a document into the cache
    pub async fn put(&self, key: String, value: serde_json::Value) {
        let mut cache = self.cache.write().await;
        let mut access_order = self.access_order.write().await;

        // If key already exists, remove old position from access order
        if cache.contains_key(&key) {
            if let Some(pos) = access_order.iter().position(|k| k == &key) {
                access_order.remove(pos);
            }
        }

        // Add to cache and access order
        cache.insert(key.clone(), Arc::new(value));
        access_order.push(key);

        // Evict oldest entries if over capacity
        while cache.len() > self.max_entries {
            if let Some(oldest) = access_order.first() {
                cache.remove(oldest);
                access_order.remove(0);
            } else {
                break;
            }
        }
    }

    /// Invalidate (remove) a specific document from cache
    pub async fn invalidate(&self, key: &str) {
        let mut cache = self.cache.write().await;
        let mut access_order = self.access_order.write().await;

        cache.remove(key);
        if let Some(pos) = access_order.iter().position(|k| k == key) {
            access_order.remove(pos);
        }
    }

    /// Invalidate all documents in a collection
    pub async fn invalidate_collection(&self, collection_prefix: &str) {
        let mut cache = self.cache.write().await;
        let mut access_order = self.access_order.write().await;

        let keys: Vec<String> = cache
            .keys()
            .filter(|k| k.starts_with(collection_prefix))
            .cloned()
            .collect();

        for key in keys {
            cache.remove(&key);
            if let Some(pos) = access_order.iter().position(|k| k == &key) {
                access_order.remove(pos);
            }
        }
    }

    /// Clear the entire cache
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        let mut access_order = self.access_order.write().await;
        cache.clear();
        access_order.clear();
    }

    /// Get cache statistics
    pub async fn stats(&self) -> DocumentCacheStats {
        let cache = self.cache.read().await;
        DocumentCacheStats {
            entries: cache.len(),
            max_entries: self.max_entries,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DocumentCacheStats {
    pub entries: usize,
    pub max_entries: usize,
}

/// Global document cache instance
static DOCUMENT_CACHE: std::sync::OnceLock<DocumentCache> = std::sync::OnceLock::new();

pub fn init_document_cache(config: &DocumentCacheConfig) {
    let _ = DOCUMENT_CACHE.set(DocumentCache::with_config(config));
}

pub fn get_document_cache() -> &'static DocumentCache {
    DOCUMENT_CACHE.get_or_init(|| DocumentCache::new(10_000))
}
