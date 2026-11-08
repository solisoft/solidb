//! The parse cache: a query's syntax tree, kept by its exact text.
//!
//! A client sends the same SDBQL text over and over with only its bind
//! variables changing, and parsing it again each time cost more than an
//! index read (1.2 µs for a one-line FILTER, 5 µs for a join). Parsing is a
//! pure function of the text — bind variables stay `@name` nodes in the tree
//! and are read from the executor at run time, and nothing rewrites a tree
//! once it is parsed (constant folding, for EXPLAIN, works on a copy) — so
//! one tree per text serves every request that sends it, whatever its bind
//! variables, its database or its caller.
//!
//! Entries are keyed by the text itself, not a hash of it: two texts never
//! share a tree. The cache is bounded by entries and by the bytes of text it
//! holds; a text longer than [`MAX_CACHED_TEXT_BYTES`] is parsed every time
//! and never stored, and when either bound is reached the least recently
//! used quarter is evicted. A text that does not parse is not stored either.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::sdbql::ast::Query;
use crate::sdbql::parser;

/// Most entries the shared cache holds.
pub const MAX_CACHED_STATEMENTS: usize = 1000;
/// Most bytes of query text the shared cache holds, all entries together.
pub const MAX_CACHED_BYTES: usize = 4 << 20;
/// Longest query text the cache stores; a longer one is parsed every time.
pub const MAX_CACHED_TEXT_BYTES: usize = 16 << 10;

/// A parsed query, shared by every request that sends its text.
pub struct PreparedStatement {
    pub query: Arc<Query>,
}

struct Entry {
    stmt: Arc<PreparedStatement>,
    /// [`PreparedStatementCache::clock`] at the last lookup that found it.
    last_used: AtomicU64,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<Box<str>, Entry>,
    /// Bytes of text held, the keys' lengths summed.
    bytes: usize,
}

/// The parse cache (see the module documentation).
///
/// One `parking_lot::RwLock` guards the map: a lookup takes the read side
/// and marks its entry used with an atomic store, so concurrent requests for
/// cached texts never wait on each other; only a miss that stores takes the
/// write side.
pub struct PreparedStatementCache {
    inner: RwLock<Inner>,
    max_entries: usize,
    max_bytes: usize,
    clock: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
}

/// What the cache holds and how it has done, for `/metrics`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
    pub bytes: usize,
}

impl PreparedStatementCache {
    /// A cache of at most `max_entries` texts and `max_bytes` bytes of text.
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            inner: RwLock::new(Inner::default()),
            max_entries: max_entries.max(1),
            max_bytes,
            clock: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// The tree cached for exactly `query_text`, if any.
    pub fn get(&self, query_text: &str) -> Option<Arc<PreparedStatement>> {
        let inner = self.inner.read();
        let entry = inner.entries.get(query_text)?;
        entry.last_used.store(
            self.clock.fetch_add(1, Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.hits.fetch_add(1, Ordering::Relaxed);
        Some(entry.stmt.clone())
    }

    /// The tree of `query_text`: the cached one, or parsed now and stored
    /// (unless the text is too long to keep).
    pub fn parse_if_needed(
        &self,
        query_text: &str,
    ) -> crate::error::DbResult<Arc<PreparedStatement>> {
        if let Some(stmt) = self.get(query_text) {
            return Ok(stmt);
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        let stmt = Arc::new(PreparedStatement {
            query: Arc::new(parser::parse(query_text)?),
        });
        if query_text.len() <= MAX_CACHED_TEXT_BYTES && query_text.len() <= self.max_bytes {
            self.put(query_text, &stmt);
        }
        Ok(stmt)
    }

    fn put(&self, query_text: &str, stmt: &Arc<PreparedStatement>) {
        let mut inner = self.inner.write();
        // Another request may have stored it meanwhile: same text, same tree.
        if inner.entries.contains_key(query_text) {
            return;
        }
        if inner.entries.len() >= self.max_entries
            || inner.bytes + query_text.len() > self.max_bytes
        {
            self.evict(&mut inner, query_text.len());
        }
        inner.bytes += query_text.len();
        inner.entries.insert(
            query_text.into(),
            Entry {
                stmt: stmt.clone(),
                last_used: AtomicU64::new(self.clock.fetch_add(1, Ordering::Relaxed)),
            },
        );
    }

    /// Drop the least recently used quarter of the entries, or more if that
    /// does not leave room for `incoming` bytes. Runs once per that many
    /// misses past the bound, so its sort is not on the path of a hit.
    fn evict(&self, inner: &mut Inner, incoming: usize) {
        let mut by_age: Vec<(u64, Box<str>)> = inner
            .entries
            .iter()
            .map(|(text, e)| (e.last_used.load(Ordering::Relaxed), text.clone()))
            .collect();
        by_age.sort_unstable_by_key(|(used, _)| *used);
        let quarter = (inner.entries.len() / 4).max(1);
        for (n, (_, text)) in by_age.into_iter().enumerate() {
            let roomy =
                inner.entries.len() < self.max_entries && inner.bytes + incoming <= self.max_bytes;
            if n >= quarter && roomy {
                break;
            }
            if inner.entries.remove(&text).is_some() {
                inner.bytes -= text.len();
            }
        }
    }

    pub fn stats(&self) -> ParseCacheStats {
        let inner = self.inner.read();
        ParseCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            entries: inner.entries.len(),
            bytes: inner.bytes,
        }
    }

    pub fn invalidate_all(&self) {
        let mut inner = self.inner.write();
        inner.entries.clear();
        inner.bytes = 0;
    }
}

impl Default for PreparedStatementCache {
    fn default() -> Self {
        Self::new(MAX_CACHED_STATEMENTS, MAX_CACHED_BYTES)
    }
}

use std::sync::OnceLock;
static PREPARED_STATEMENT_CACHE: OnceLock<PreparedStatementCache> = OnceLock::new();

/// The cache shared by the HTTP `/cursor` handler and the driver.
pub fn get_prepared_statement_cache() -> &'static PreparedStatementCache {
    PREPARED_STATEMENT_CACHE.get_or_init(PreparedStatementCache::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdbql::{BodyClause, Expression};

    #[test]
    fn a_repeated_text_is_parsed_once() {
        let cache = PreparedStatementCache::default();
        let query = "FOR doc IN users FILTER doc.age > @min RETURN doc";
        let first = cache.parse_if_needed(query).unwrap();
        let second = cache.parse_if_needed(query).unwrap();
        assert!(Arc::ptr_eq(&first.query, &second.query));
        let stats = cache.stats();
        assert_eq!((stats.hits, stats.misses, stats.entries), (1, 1, 1));
        assert_eq!(stats.bytes, query.len());
    }

    #[test]
    fn different_texts_get_their_own_trees() {
        let cache = PreparedStatementCache::default();
        let users = cache
            .parse_if_needed("FOR doc IN users RETURN doc")
            .unwrap();
        let orders = cache
            .parse_if_needed("FOR doc IN orders RETURN doc")
            .unwrap();
        // Texts differing only in whitespace are different keys too.
        let spaced = cache
            .parse_if_needed("FOR doc IN users  RETURN doc")
            .unwrap();
        let collection = |q: &Query| match &q.body_clauses[0] {
            BodyClause::For(f) => f.collection.clone(),
            _ => unreachable!(),
        };
        assert_eq!(collection(&users.query), "users");
        assert_eq!(collection(&orders.query), "orders");
        assert!(!Arc::ptr_eq(&users.query, &spaced.query));
        assert_eq!(cache.stats().misses, 3);
    }

    #[test]
    fn bind_variables_stay_in_the_tree() {
        // What makes one tree right for every request: the values are not
        // in it.
        let cache = PreparedStatementCache::default();
        let q = cache.parse_if_needed("RETURN @x").unwrap();
        let ret = q.query.return_clause.as_ref().unwrap();
        assert!(matches!(&ret.expression, Expression::BindVariable(n) if n == "x"));
    }

    #[test]
    fn errors_are_not_cached() {
        let cache = PreparedStatementCache::default();
        assert!(cache.parse_if_needed("FOR doc IN").is_err());
        assert!(cache.parse_if_needed("FOR doc IN").is_err());
        let stats = cache.stats();
        assert_eq!((stats.misses, stats.entries), (2, 0));
    }

    #[test]
    fn entries_are_bounded_and_the_recently_used_kept() {
        let cache = PreparedStatementCache::new(8, 1 << 20);
        let hot = "FOR d IN hot RETURN d";
        cache.parse_if_needed(hot).unwrap();
        for i in 0..100 {
            cache
                .parse_if_needed(&format!("FOR d IN c{i} RETURN d"))
                .unwrap();
            // Used between every insert: never the least recently used.
            cache.parse_if_needed(hot).unwrap();
            assert!(cache.stats().entries <= 8);
        }
        assert!(cache.get(hot).is_some());
        assert!(cache.get("FOR d IN c0 RETURN d").is_none());
    }

    #[test]
    fn bytes_are_bounded() {
        let cache = PreparedStatementCache::new(1000, 200);
        for i in 0..50 {
            cache
                .parse_if_needed(&format!("FOR d IN collection_{i:03} RETURN d"))
                .unwrap();
            assert!(cache.stats().bytes <= 200, "{:?}", cache.stats());
        }
        assert!(cache.stats().entries >= 1);
    }

    #[test]
    fn long_texts_are_parsed_but_not_kept() {
        let cache = PreparedStatementCache::default();
        let long = format!("RETURN \"{}\"", "x".repeat(MAX_CACHED_TEXT_BYTES));
        cache.parse_if_needed(&long).unwrap();
        cache.parse_if_needed(&long).unwrap();
        let stats = cache.stats();
        assert_eq!((stats.misses, stats.entries, stats.bytes), (2, 0, 0));
    }

    #[test]
    fn concurrent_use_shares_one_tree_per_text() {
        let cache = Arc::new(PreparedStatementCache::new(16, 1 << 20));
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let cache = cache.clone();
                std::thread::spawn(move || {
                    for i in 0..2_000 {
                        let n = (i * 7 + t) % 24;
                        let text = format!("FOR d IN c{n} FILTER d.v == @v RETURN d");
                        let stmt = cache.parse_if_needed(&text).unwrap();
                        match &stmt.query.body_clauses[0] {
                            BodyClause::For(f) => assert_eq!(f.collection, format!("c{n}")),
                            _ => unreachable!(),
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let stats = cache.stats();
        assert!(stats.entries <= 16);
        assert_eq!(stats.hits + stats.misses, 16_000);
    }
}
