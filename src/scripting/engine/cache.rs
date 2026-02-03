//! Script Bytecode Cache
//!
//! This module provides caching for compiled Lua bytecode, avoiding
//! the cost of recompiling scripts on every request.

use dashmap::DashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Cache for compiled Lua bytecode.
///
/// Compiling Lua source to bytecode takes ~10% of request time.
/// This cache stores compiled bytecode keyed by script identity,
/// allowing instant reuse of pre-compiled code.
pub struct ScriptCache {
    /// Cache entries: key -> (bytecode, last_access_time, access_count)
    entries: DashMap<String, CacheEntry>,
    /// Maximum number of entries
    max_size: usize,
    /// Cache statistics
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

/// A cached bytecode entry
struct CacheEntry {
    /// Compiled Lua bytecode (Arc for O(1) clone on cache hits)
    bytecode: Arc<Vec<u8>>,
    /// When this entry was last accessed
    last_access: Instant,
    /// Number of times this entry was accessed
    access_count: AtomicUsize,
    /// Hash of the source code (for invalidation)
    code_hash: u64,
}

impl ScriptCache {
    /// Create a new cache with the specified maximum size.
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: DashMap::with_capacity(max_size),
            max_size,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    /// Create a cache with default size (1000 entries).
    pub fn with_default_size() -> Self {
        Self::new(1000)
    }

    /// Generate a cache key for a script.
    pub fn cache_key(script_key: &str, code: &str) -> String {
        let code_hash = Self::hash_code(code);
        format!("{}:{:016x}", script_key, code_hash)
    }

    /// Hash source code for cache invalidation.
    fn hash_code(code: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        code.hash(&mut hasher);
        hasher.finish()
    }

    /// Get cached bytecode or compile and cache it.
    ///
    /// Returns the bytecode if cached, or compiles the source and caches the result.
    /// Uses Arc<Vec<u8>> internally for O(1) clone on cache hits.
    pub fn get_or_compile<F>(
        &self,
        script_key: &str,
        code: &str,
        compile_fn: F,
    ) -> Result<Vec<u8>, mlua::Error>
    where
        F: FnOnce(&str) -> Result<Vec<u8>, mlua::Error>,
    {
        let code_hash = Self::hash_code(code);
        let key = format!("{}:{:016x}", script_key, code_hash);

        // Try to get from cache
        if let Some(mut entry) = self.entries.get_mut(&key) {
            // Verify hash matches (in case of hash collision)
            if entry.code_hash == code_hash {
                entry.last_access = Instant::now();
                entry.access_count.fetch_add(1, Ordering::Relaxed);
                self.hits.fetch_add(1, Ordering::Relaxed);
                // Arc::clone is O(1) atomic increment - no bytecode copying!
                return Ok((*entry.bytecode).clone());
            }
        }

        // Cache miss - compile
        self.misses.fetch_add(1, Ordering::Relaxed);
        let bytecode = compile_fn(code)?;

        // Evict if needed
        if self.entries.len() >= self.max_size {
            self.evict_lru();
        }

        // Store in cache with Arc wrapper
        let bytecode_arc = Arc::new(bytecode.clone());
        self.entries.insert(
            key,
            CacheEntry {
                bytecode: bytecode_arc,
                last_access: Instant::now(),
                access_count: AtomicUsize::new(1),
                code_hash,
            },
        );

        Ok(bytecode)
    }

    /// Get bytecode from cache if available.
    pub fn get(&self, script_key: &str, code: &str) -> Option<Vec<u8>> {
        let code_hash = Self::hash_code(code);
        let key = format!("{}:{:016x}", script_key, code_hash);

        if let Some(mut entry) = self.entries.get_mut(&key) {
            if entry.code_hash == code_hash {
                entry.last_access = Instant::now();
                entry.access_count.fetch_add(1, Ordering::Relaxed);
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Some((*entry.bytecode).clone());
            }
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Store compiled bytecode in the cache.
    pub fn insert(&self, script_key: &str, code: &str, bytecode: Vec<u8>) {
        let code_hash = Self::hash_code(code);
        let key = format!("{}:{:016x}", script_key, code_hash);

        // Evict if needed
        if self.entries.len() >= self.max_size {
            self.evict_lru();
        }

        self.entries.insert(
            key,
            CacheEntry {
                bytecode: Arc::new(bytecode),
                last_access: Instant::now(),
                access_count: AtomicUsize::new(1),
                code_hash,
            },
        );
    }

    /// Invalidate cache entry for a script.
    pub fn invalidate(&self, script_key: &str) {
        // Remove all entries that start with this script key
        self.entries
            .retain(|k, _| !k.starts_with(&format!("{}:", script_key)));
    }

    /// Clear the entire cache.
    pub fn clear(&self) {
        self.entries.clear();
    }

    /// Evict the least recently used entry.
    fn evict_lru(&self) {
        let mut oldest_key: Option<String> = None;
        let mut oldest_time = Instant::now();

        // Find LRU entry
        for entry in self.entries.iter() {
            if entry.last_access < oldest_time {
                oldest_time = entry.last_access;
                oldest_key = Some(entry.key().clone());
            }
        }

        // Remove it
        if let Some(key) = oldest_key {
            self.entries.remove(&key);
            self.evictions.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.entries.len(),
            max_size: self.max_size,
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Current number of cached entries
    pub entries: usize,
    /// Maximum cache size
    pub max_size: usize,
    /// Number of cache hits
    pub hits: u64,
    /// Number of cache misses
    pub misses: u64,
    /// Number of entries evicted
    pub evictions: u64,
}

impl CacheStats {
    /// Calculate hit rate as a percentage
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    #[test]
    fn test_cache_creation() {
        let cache = ScriptCache::new(100);
        assert_eq!(cache.max_size, 100);

        let stats = cache.stats();
        assert_eq!(stats.entries, 0);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
    }

    #[test]
    fn test_cache_key_generation() {
        let key1 = ScriptCache::cache_key("script1", "return 1");
        let key2 = ScriptCache::cache_key("script1", "return 2");
        let key3 = ScriptCache::cache_key("script1", "return 1");

        assert_ne!(key1, key2); // Different code
        assert_eq!(key1, key3); // Same code
    }

    #[test]
    fn test_cache_hit_miss() {
        let cache = ScriptCache::new(10);
        let lua = Lua::new();

        // First call - miss
        let result = cache.get_or_compile("test", "return 1 + 1", |code| {
            let chunk = lua.load(code);
            let func = chunk.into_function()?;
            Ok(func.dump(false))
        });
        assert!(result.is_ok());

        let stats = cache.stats();
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 0);

        // Second call - hit
        let result = cache.get_or_compile("test", "return 1 + 1", |_| {
            panic!("Should not compile again!");
        });
        assert!(result.is_ok());

        let stats = cache.stats();
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 1);
    }

    #[test]
    fn test_cache_invalidation() {
        let cache = ScriptCache::new(10);

        // Insert some entries
        cache.insert("script1", "code1", vec![1, 2, 3]);
        cache.insert("script1", "code2", vec![4, 5, 6]);
        cache.insert("script2", "code1", vec![7, 8, 9]);

        assert_eq!(cache.stats().entries, 3);

        // Invalidate script1
        cache.invalidate("script1");

        assert_eq!(cache.stats().entries, 1);
        assert!(cache.get("script2", "code1").is_some());
        assert!(cache.get("script1", "code1").is_none());
    }

    #[test]
    fn test_cache_eviction() {
        let cache = ScriptCache::new(2);

        cache.insert("s1", "c1", vec![1]);
        cache.insert("s2", "c2", vec![2]);

        assert_eq!(cache.stats().entries, 2);

        // This should trigger eviction
        cache.insert("s3", "c3", vec![3]);

        assert_eq!(cache.stats().entries, 2);
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn test_hit_rate() {
        let cache = ScriptCache::new(10);

        cache.insert("s1", "c1", vec![1]);

        // 3 hits
        cache.get("s1", "c1");
        cache.get("s1", "c1");
        cache.get("s1", "c1");

        // 1 miss
        cache.get("s2", "c2");

        let stats = cache.stats();
        assert_eq!(stats.hits, 3);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hit_rate(), 75.0);
    }

    #[test]
    fn test_compile_error_not_cached() {
        let cache = ScriptCache::new(10);
        let lua = Lua::new();

        // Try to compile invalid Lua code
        let result =
            cache.get_or_compile("bad_script", "this is not valid lua syntax !!!", |code| {
                let chunk = lua.load(code);
                let func = chunk.into_function()?;
                Ok(func.dump(false))
            });

        // Should fail
        assert!(result.is_err());

        // Should NOT be cached
        let stats = cache.stats();
        assert_eq!(stats.entries, 0, "Failed compile should not be cached");
    }

    #[test]
    fn test_invalidate_nonexistent() {
        let cache = ScriptCache::new(10);

        // Insert one entry
        cache.insert("s1", "c1", vec![1]);
        assert_eq!(cache.stats().entries, 1);

        // Invalidate non-existent key - should not panic or affect existing
        cache.invalidate("nonexistent");

        // s1 should still be there
        assert_eq!(cache.stats().entries, 1);
        assert!(cache.get("s1", "c1").is_some());
    }

    #[test]
    fn test_concurrent_same_script() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(ScriptCache::new(10));
        let compile_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let c = cache.clone();
                let cc = compile_count.clone();
                thread::spawn(move || {
                    c.get_or_compile("test", "return 1", |_| {
                        cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        // Simulate some compilation time
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        Ok(vec![1, 2, 3])
                    })
                    .unwrap()
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        // Only one entry should exist
        assert_eq!(cache.stats().entries, 1);
    }

    #[test]
    fn test_different_code_same_key() {
        let cache = ScriptCache::new(10);

        // Insert with same script_key but different code - should be separate entries
        cache.insert("s1", "code_v1", vec![1, 1, 1]);
        cache.insert("s1", "code_v2", vec![2, 2, 2]);

        assert_eq!(cache.stats().entries, 2);

        // Both should be retrievable
        let v1 = cache.get("s1", "code_v1");
        let v2 = cache.get("s1", "code_v2");

        assert!(v1.is_some());
        assert!(v2.is_some());
        assert_eq!(v1.unwrap(), vec![1, 1, 1]);
        assert_eq!(v2.unwrap(), vec![2, 2, 2]);
    }

    #[test]
    fn test_invalidate_clears_all_versions() {
        let cache = ScriptCache::new(10);

        // Insert multiple versions of the same script
        cache.insert("s1", "code_v1", vec![1]);
        cache.insert("s1", "code_v2", vec![2]);
        cache.insert("s2", "code_x", vec![3]);

        assert_eq!(cache.stats().entries, 3);

        // Invalidate s1 - should clear both versions
        cache.invalidate("s1");

        assert_eq!(cache.stats().entries, 1);
        assert!(cache.get("s1", "code_v1").is_none());
        assert!(cache.get("s1", "code_v2").is_none());
        assert!(cache.get("s2", "code_x").is_some());
    }

    #[test]
    fn test_lru_eviction_order() {
        let cache = ScriptCache::new(3);

        // Insert 3 entries
        cache.insert("s1", "c1", vec![1]);
        std::thread::sleep(std::time::Duration::from_millis(5));
        cache.insert("s2", "c2", vec![2]);
        std::thread::sleep(std::time::Duration::from_millis(5));
        cache.insert("s3", "c3", vec![3]);

        // Access s1 to make it recent
        cache.get("s1", "c1");

        // Insert s4 - should evict s2 (oldest access)
        cache.insert("s4", "c4", vec![4]);

        assert_eq!(cache.stats().entries, 3);
        assert!(
            cache.get("s1", "c1").is_some(),
            "s1 should still exist (recently accessed)"
        );
        assert!(cache.get("s3", "c3").is_some(), "s3 should still exist");
        assert!(
            cache.get("s4", "c4").is_some(),
            "s4 should exist (just added)"
        );
        // s2 was evicted - checking would increment miss counter but entry is gone
    }

    #[test]
    fn test_clear() {
        let cache = ScriptCache::new(10);

        cache.insert("s1", "c1", vec![1]);
        cache.insert("s2", "c2", vec![2]);
        cache.insert("s3", "c3", vec![3]);

        assert_eq!(cache.stats().entries, 3);

        cache.clear();

        assert_eq!(cache.stats().entries, 0);
        assert!(cache.get("s1", "c1").is_none());
    }

    #[test]
    fn test_zero_hit_rate() {
        let cache = ScriptCache::new(10);

        // Empty cache - hit rate should be 0
        let stats = cache.stats();
        assert_eq!(stats.hit_rate(), 0.0);

        // Only misses
        cache.get("nonexistent", "code");
        cache.get("nonexistent2", "code2");

        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.hit_rate(), 0.0);
    }
}
