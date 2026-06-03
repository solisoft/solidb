use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::sdbql::ast::{BodyClause, Query};
use crate::sdbql::parser;

#[derive(Clone)]
pub struct PreparedStatement {
    pub query: Arc<Query>,
    pub hash: String,
    pub created_at: Instant,
    pub use_count: u64,
}

/// A cached statement plus the collections it references (computed once at
/// `put` time so removal can prune only the matching index sets).
struct CacheEntry {
    stmt: Arc<PreparedStatement>,
    collections: HashSet<String>,
}

/// Entry map + per-collection invalidation index, guarded by a single lock.
#[derive(Default)]
struct Inner {
    entries: HashMap<String, CacheEntry>,
    /// collection_name -> set of statement hashes that reference it
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

/// Prepared-statement cache.
///
/// Concurrency: one `parking_lot::RwLock` guards both the entry map and the
/// per-collection invalidation index, so the read path doesn't suspend the
/// async runtime, the two structures can never diverge (a put racing an
/// invalidate is fully serialized), and `invalidate_collection` is
/// O(matches) instead of the previous O(total cache) scan that also walked
/// every `Arc<Query>`.
pub struct PreparedStatementCache {
    inner: RwLock<Inner>,
    max_entries: usize,
    ttl: Duration,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl PreparedStatementCache {
    pub fn new(max_entries: usize, ttl_secs: u64) -> Self {
        Self {
            inner: RwLock::new(Inner::default()),
            max_entries,
            ttl: Duration::from_secs(ttl_secs),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    pub fn get(&self, query_text: &str) -> Option<Arc<PreparedStatement>> {
        let hash = Self::hash_query(query_text);
        let inner = self.inner.read();
        if let Some(entry) = inner.entries.get(&hash) {
            if entry.stmt.created_at.elapsed() < self.ttl {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Some(entry.stmt.clone());
            }
        }
        None
    }

    pub fn put(&self, query_text: &str, query: Query) -> Arc<PreparedStatement> {
        let hash = Self::hash_query(query_text);
        let collections = collect_collection_names(&query);
        let stmt = Arc::new(PreparedStatement {
            query: Arc::new(query),
            hash: hash.clone(),
            created_at: Instant::now(),
            use_count: 0,
        });

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
                .insert(hash.clone());
        }
        inner.entries.insert(
            hash,
            CacheEntry {
                stmt: stmt.clone(),
                collections,
            },
        );

        stmt
    }

    pub fn parse_if_needed(
        &self,
        query_text: &str,
    ) -> crate::error::DbResult<Arc<PreparedStatement>> {
        if let Some(stmt) = self.get(query_text) {
            Ok(stmt)
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            let query = parser::parse(query_text)?;
            Ok(self.put(query_text, query))
        }
    }

    pub fn hash_query(query: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        query.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    pub fn stats(&self) -> (u64, u64, usize) {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let size = self.inner.read().entries.len();
        (hits, misses, size)
    }

    pub fn invalidate_all(&self) {
        let mut inner = self.inner.write();
        inner.entries.clear();
        inner.by_collection.clear();
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
    }

    pub fn invalidate_collection(&self, collection_name: &str) {
        let mut inner = self.inner.write();
        let Some(to_remove) = inner.by_collection.remove(collection_name) else {
            return;
        };
        for key in &to_remove {
            // Also prunes cross-references from other collections' sets.
            inner.remove_entry(key);
        }
    }

    /// Number of collections currently tracked by the invalidation index
    /// (test/observability helper).
    #[cfg(test)]
    fn index_len(&self) -> usize {
        self.inner.read().by_collection.len()
    }
}

impl Default for PreparedStatementCache {
    fn default() -> Self {
        Self::new(1000, 300)
    }
}

/// Extract the collection names referenced by a query's body clauses.
/// Used to populate the per-collection invalidation index at `put` time,
/// so `invalidate_collection` doesn't have to re-walk every `Arc<Query>`.
fn collect_collection_names(query: &Query) -> HashSet<String> {
    let mut out = HashSet::new();
    for clause in &query.body_clauses {
        if let BodyClause::For(for_clause) = clause {
            out.insert(for_clause.collection.clone());
        }
    }
    out
}

use std::sync::OnceLock;
static PREPARED_STATEMENT_CACHE: OnceLock<PreparedStatementCache> = OnceLock::new();

pub fn get_prepared_statement_cache() -> &'static PreparedStatementCache {
    PREPARED_STATEMENT_CACHE.get_or_init(PreparedStatementCache::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic() {
        let cache = PreparedStatementCache::default();

        let query = "FOR doc IN users RETURN doc";

        let stmt1 = cache.parse_if_needed(query).unwrap();
        let stmt2 = cache.parse_if_needed(query).unwrap();

        assert_eq!(stmt1.hash, stmt2.hash);

        let (hits, misses, _) = cache.stats();
        assert_eq!(hits, 1);
        assert_eq!(misses, 1);
    }

    #[test]
    fn test_cache_miss() {
        let cache = PreparedStatementCache::default();

        let stmt1 = cache
            .parse_if_needed("FOR doc IN users RETURN doc")
            .unwrap();
        let stmt2 = cache
            .parse_if_needed("FOR doc IN orders RETURN doc")
            .unwrap();

        assert_ne!(stmt1.hash, stmt2.hash);

        let (hits, misses, _) = cache.stats();
        assert_eq!(hits, 0);
        assert_eq!(misses, 2);
    }

    #[test]
    fn test_invalidate_collection_uses_index() {
        let cache = PreparedStatementCache::default();
        let _ = cache
            .parse_if_needed("FOR doc IN users RETURN doc")
            .unwrap();
        let _ = cache
            .parse_if_needed("FOR doc IN orders RETURN doc")
            .unwrap();
        let (_, _, size_before) = cache.stats();
        assert_eq!(size_before, 2);

        cache.invalidate_collection("users");
        let (_, _, size_after) = cache.stats();
        assert_eq!(size_after, 1);

        // The users index entry is gone entirely (no empty-set leak).
        assert_eq!(cache.index_len(), 1);

        // The remaining statement (orders) should still be there.
        let stmt = cache
            .parse_if_needed("FOR doc IN orders RETURN doc")
            .unwrap();
        // It's a cache hit on the still-present statement.
        let (hits, _, _) = cache.stats();
        assert!(hits >= 1);
        assert_eq!(stmt.query.body_clauses.len(), 1);
    }

    #[test]
    fn test_eviction_prunes_index() {
        let cache = PreparedStatementCache::new(2, 300);
        let _ = cache.parse_if_needed("FOR doc IN a RETURN doc").unwrap();
        let _ = cache.parse_if_needed("FOR doc IN b RETURN doc").unwrap();
        // Triggers eviction of half the entries.
        let _ = cache.parse_if_needed("FOR doc IN c RETURN doc").unwrap();

        let (_, _, size) = cache.stats();
        assert!(size <= 2);
        // Index never tracks more collections than live entries reference.
        assert!(cache.index_len() <= size);
    }
}
