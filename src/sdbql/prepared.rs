use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::sdbql::ast::Query;
use crate::sdbql::parser;

#[derive(Clone)]
pub struct PreparedStatement {
    pub query: Arc<Query>,
    pub hash: String,
    pub created_at: Instant,
    pub use_count: u64,
}

pub struct PreparedStatementCache {
    cache: RwLock<HashMap<String, Arc<PreparedStatement>>>,
    max_entries: usize,
    ttl: Duration,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl PreparedStatementCache {
    pub fn new(max_entries: usize, ttl_secs: u64) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            max_entries,
            ttl: Duration::from_secs(ttl_secs),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    pub async fn get(&self, query_text: &str) -> Option<Arc<PreparedStatement>> {
        let hash = Self::hash_query(query_text);
        let cache = self.cache.read().await;

        if let Some(stmt) = cache.get(&hash) {
            if stmt.created_at.elapsed() < self.ttl {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Some(stmt.clone());
            }
        }
        None
    }

    pub async fn put(&self, query_text: &str, query: Query) -> Arc<PreparedStatement> {
        let hash = Self::hash_query(query_text);
        let stmt = Arc::new(PreparedStatement {
            query: Arc::new(query),
            hash: hash.clone(),
            created_at: Instant::now(),
            use_count: 0,
        });

        let mut cache = self.cache.write().await;

        if cache.len() >= self.max_entries {
            let keys_to_remove: Vec<String> =
                cache.keys().take(self.max_entries / 2).cloned().collect();
            for key in keys_to_remove {
                cache.remove(&key);
            }
        }

        cache.insert(hash.clone(), stmt.clone());
        stmt
    }

    pub async fn parse_if_needed(
        &self,
        query_text: &str,
    ) -> crate::error::DbResult<Arc<PreparedStatement>> {
        if let Some(stmt) = self.get(query_text).await {
            Ok(stmt)
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            let query = parser::parse(query_text)?;
            Ok(self.put(query_text, query).await)
        }
    }

    pub fn hash_query(query: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        query.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    pub async fn stats(&self) -> (u64, u64, usize) {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let size = self.cache.read().await.len();
        (hits, misses, size)
    }

    pub async fn invalidate_all(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
    }

    pub async fn invalidate_collection(&self, collection_name: &str) {
        let mut cache = self.cache.write().await;
        let all_keys: Vec<String> = cache.keys().cloned().collect();
        let keys_to_remove: Vec<String> = all_keys
            .into_iter()
            .filter(|k| {
                if let Some(stmt) = cache.get(k) {
                    stmt.query.body_clauses.iter().any(|clause| {
                        matches!(clause, crate::sdbql::ast::BodyClause::For(for_clause)
                            if for_clause.collection == collection_name)
                    })
                } else {
                    false
                }
            })
            .collect();

        for key in keys_to_remove {
            cache.remove(&key);
        }
    }
}

impl Default for PreparedStatementCache {
    fn default() -> Self {
        Self::new(1000, 300)
    }
}

use std::sync::OnceLock;
static PREPARED_STATEMENT_CACHE: OnceLock<PreparedStatementCache> = OnceLock::new();

pub fn get_prepared_statement_cache() -> &'static PreparedStatementCache {
    PREPARED_STATEMENT_CACHE.get_or_init(PreparedStatementCache::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_basic() {
        let cache = PreparedStatementCache::default();

        let query = "FOR doc IN users RETURN doc";

        let stmt1 = cache.parse_if_needed(query).await.unwrap();
        let stmt2 = cache.parse_if_needed(query).await.unwrap();

        assert_eq!(stmt1.hash, stmt2.hash);

        let (hits, misses, _) = cache.stats().await;
        assert_eq!(hits, 1);
        assert_eq!(misses, 1);
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let cache = PreparedStatementCache::default();

        let stmt1 = cache
            .parse_if_needed("FOR doc IN users RETURN doc")
            .await
            .unwrap();
        let stmt2 = cache
            .parse_if_needed("FOR doc IN orders RETURN doc")
            .await
            .unwrap();

        assert_ne!(stmt1.hash, stmt2.hash);

        let (hits, misses, _) = cache.stats().await;
        assert_eq!(hits, 0);
        assert_eq!(misses, 2);
    }
}
