//! In-memory semantic cache for LLM responses.
//!
//! Opt-in via `SEMANTIC_CACHE_ENABLED`. A query is embedded and compared by cosine
//! similarity against recently cached queries; if one is similar enough
//! (`>= SEMANTIC_CACHE_THRESHOLD`) and not expired (`SEMANTIC_CACHE_TTL`), its
//! stored response is returned without calling the LLM again.
//!
//! Purely in-memory (like the exact-match [`crate::storage::query_cache`]): it is
//! lost on restart and never serves a response across a restart. The caller owns
//! embedding the query and passes the vector in, so this module has no LLM
//! dependency — it is just a cosine-nearest lookup with TTL + size bounds.
//!
//! Configuration (process environment, read once at first use):
//! - `SEMANTIC_CACHE_ENABLED` — `1`/`true`/`yes`/`on` to enable (default: off)
//! - `SEMANTIC_CACHE_THRESHOLD` — cosine similarity cutoff (default: `0.95`)
//! - `SEMANTIC_CACHE_TTL` — entry lifetime in seconds (default: `3600`)
//! - `SEMANTIC_CACHE_MAX` — max cached queries per database (default: `256`)

use crate::storage::vector::cosine_similarity;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

struct Entry {
    embedding: Vec<f32>,
    response: Value,
    cached_at: Instant,
}

/// Cosine-nearest response cache, partitioned per database.
pub struct SemanticCache {
    inner: RwLock<HashMap<String, Vec<Entry>>>,
    enabled: bool,
    threshold: f32,
    ttl: Duration,
    max_per_db: usize,
}

impl SemanticCache {
    pub(crate) fn new(enabled: bool, threshold: f32, ttl: Duration, max_per_db: usize) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            enabled,
            threshold,
            ttl,
            max_per_db: max_per_db.max(1),
        }
    }

    fn from_env() -> Self {
        let enabled = std::env::var("SEMANTIC_CACHE_ENABLED")
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        let threshold = std::env::var("SEMANTIC_CACHE_THRESHOLD")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0.95_f32);
        let ttl_secs = std::env::var("SEMANTIC_CACHE_TTL")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(3600_u64);
        let max_per_db = std::env::var("SEMANTIC_CACHE_MAX")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(256_usize);
        Self::new(
            enabled,
            threshold,
            Duration::from_secs(ttl_secs),
            max_per_db,
        )
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Return a cached response whose query embedding is within `threshold` cosine
    /// similarity of `query_emb` (best match wins), or `None`.
    pub fn get(&self, db: &str, query_emb: &[f32]) -> Option<Value> {
        if !self.enabled || query_emb.is_empty() {
            return None;
        }
        let map = self.inner.read().ok()?;
        let bucket = map.get(db)?;
        let mut best: Option<(f32, &Value)> = None;
        for e in bucket.iter() {
            if e.cached_at.elapsed() > self.ttl {
                continue;
            }
            let sim = cosine_similarity(query_emb, &e.embedding);
            let better = match best {
                Some((b, _)) => sim > b,
                None => true,
            };
            if sim >= self.threshold && better {
                best = Some((sim, &e.response));
            }
        }
        best.map(|(_, v)| v.clone())
    }

    /// Store `response` keyed by its query embedding.
    pub fn put(&self, db: &str, query_emb: Vec<f32>, response: Value) {
        if !self.enabled || query_emb.is_empty() {
            return;
        }
        let mut map = match self.inner.write() {
            Ok(m) => m,
            Err(_) => return,
        };
        let ttl = self.ttl;
        let bucket = map.entry(db.to_string()).or_default();
        // Opportunistically drop expired entries.
        bucket.retain(|e| e.cached_at.elapsed() <= ttl);
        bucket.push(Entry {
            embedding: query_emb,
            response,
            cached_at: Instant::now(),
        });
        // Bound size: drop the oldest entries on overflow (appended in time order).
        if bucket.len() > self.max_per_db {
            let overflow = bucket.len() - self.max_per_db;
            bucket.drain(0..overflow);
        }
    }

    /// Remove all cached entries.
    pub fn clear(&self) {
        if let Ok(mut m) = self.inner.write() {
            m.clear();
        }
    }
}

static CACHE: OnceLock<SemanticCache> = OnceLock::new();

/// Process-wide semantic cache, configured from the environment on first use.
pub fn semantic_cache() -> &'static SemanticCache {
    CACHE.get_or_init(SemanticCache::from_env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cache() -> SemanticCache {
        SemanticCache::new(true, 0.9, Duration::from_secs(3600), 256)
    }

    #[test]
    fn test_hit_on_similar_and_miss_on_dissimilar() {
        let c = cache();
        c.put("db", vec![1.0, 0.0, 0.0], json!("answer-A"));

        // Near-identical query → hit.
        assert_eq!(c.get("db", &[0.99, 0.01, 0.0]), Some(json!("answer-A")));
        // Exact query → hit.
        assert_eq!(c.get("db", &[1.0, 0.0, 0.0]), Some(json!("answer-A")));
        // Orthogonal query (cosine 0) → miss.
        assert_eq!(c.get("db", &[0.0, 1.0, 0.0]), None);
        // Different database bucket → miss.
        assert_eq!(c.get("other", &[1.0, 0.0, 0.0]), None);
    }

    #[test]
    fn test_disabled_is_noop() {
        let c = SemanticCache::new(false, 0.9, Duration::from_secs(3600), 256);
        c.put("db", vec![1.0, 0.0, 0.0], json!("x"));
        assert_eq!(c.get("db", &[1.0, 0.0, 0.0]), None);
    }

    #[test]
    fn test_eviction_bounds_bucket() {
        let c = SemanticCache::new(true, 0.9, Duration::from_secs(3600), 2);
        c.put("db", vec![1.0, 0.0, 0.0], json!("a"));
        c.put("db", vec![0.0, 1.0, 0.0], json!("b"));
        c.put("db", vec![0.0, 0.0, 1.0], json!("c")); // evicts oldest ("a")

        assert_eq!(c.get("db", &[1.0, 0.0, 0.0]), None); // "a" evicted
        assert_eq!(c.get("db", &[0.0, 1.0, 0.0]), Some(json!("b")));
        assert_eq!(c.get("db", &[0.0, 0.0, 1.0]), Some(json!("c")));
    }

    #[test]
    fn test_ttl_expiry() {
        let c = SemanticCache::new(true, 0.9, Duration::from_millis(5), 256);
        c.put("db", vec![1.0, 0.0, 0.0], json!("stale"));
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(c.get("db", &[1.0, 0.0, 0.0]), None);
    }
}
