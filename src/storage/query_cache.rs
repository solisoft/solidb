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
        self.put_checked(query_hash, result, None);
    }

    fn put_checked(
        &self,
        query_hash: String,
        result: Vec<serde_json::Value>,
        generation: Option<u64>,
    ) {
        let collections = extract_collections_from_key(&query_hash);

        let mut inner = self.inner.write();

        // Checked under the write lock: an invalidation bumps the generation
        // before taking the lock, so either this sees the bump, or the
        // invalidation runs after this insert and removes it.
        if generation.is_some_and(|g| g != current_generation()) {
            return;
        }

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
        bump_generation();
        let mut inner = self.inner.write();
        inner.entries.clear();
        inner.by_collection.clear();
    }

    /// Invalidate queries related to a specific collection. O(matches)
    /// instead of the previous O(total cache) scan.
    pub fn invalidate_collection(&self, collection_name: &str) {
        bump_generation();
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

/// Generation counter bumped by every invalidation. A reader snapshots it
/// before executing and stores its result only if it is unchanged, so a write
/// that lands while the query runs cannot be papered over by a `put` of the
/// pre-write rows after the invalidation already happened (audit P2).
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn bump_generation() {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}

/// Snapshot of the invalidation generation, for [`QueryCache::put_if_current`].
pub fn current_generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::Acquire)
}

impl QueryCache {
    /// Store `result` only if no invalidation happened since `generation`
    /// was taken (see [`current_generation`]).
    pub fn put_if_current(
        &self,
        query_hash: String,
        result: Vec<serde_json::Value>,
        generation: u64,
    ) {
        self.put_checked(query_hash, result, Some(generation));
    }
}

/// Normalise a collection name for the invalidation index: storage-level
/// callers hold the qualified `db:coll` column-family name, handlers the short
/// one. The index is keyed on the short name (it over-invalidates the same
/// name in other databases, which is safe).
fn index_name(collection: &str) -> &str {
    let name = collection.rsplit(':').next().unwrap_or(collection);
    name.trim_matches('`')
}

/// Drop every cached result that read `collection` in `db_name`.
///
/// The entry point for write paths outside the HTTP handlers — replication
/// apply, queue workers, TTL expiry, `/sql` — which must call it after they
/// change documents, or `/cursor` keeps serving the pre-write rows until the
/// TTL runs out. `collection` may be short (`users`) or qualified
/// (`mydb:users`).
pub fn invalidate_collection(db_name: &str, collection: &str) {
    // The index is not partitioned by database; see `index_name`.
    let _ = db_name;
    get_query_cache().invalidate_collection(index_name(collection));
}

/// Drop the whole result cache (e.g. a transaction commit whose write set is
/// not tracked by name).
pub fn invalidate_all() {
    get_query_cache().invalidate_all();
}

/// Stable fingerprint of the caller, so two principals never share a cached
/// result: row policies, `CURRENT_USER()`, `CURRENT_ROLES()` and `CAN()` all
/// make a query's rows depend on who runs it (audit H1).
fn hash_principal<H: std::hash::Hasher>(principal: &crate::sdbql::QueryPrincipal, hasher: &mut H) {
    use std::hash::Hash;
    "principal".hash(hasher);
    principal.user.hash(hasher);
    let mut roles: Vec<&String> = principal.roles.iter().collect();
    roles.sort();
    roles.dedup();
    roles.hash(hasher);
    principal.can_read.hash(hasher);
    principal.can_write.hash(hasher);
    principal.can_admin.hash(hasher);
}

/// Generate a cache key for a read query.
///
/// The key format is `"db/coll1,coll2:hash"` so the cache is partitioned per
/// database (same query text on different databases must not collide) and
/// `invalidate_collection` can still match by collection name. The hash
/// covers the database, the query text, the bind variables and the principal.
///
/// `collections` must come from [`cacheable_collections`] (or
/// [`cache_key_for`], which does both steps); a query whose collection set is
/// unknown must not be cached at all.
pub fn hash_query(
    db_name: &str,
    query: &str,
    bind_vars: &std::collections::HashMap<String, serde_json::Value>,
    principal: &crate::sdbql::QueryPrincipal,
    collections: &[String],
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

    hash_principal(principal, &mut hasher);

    let mut names: Vec<&str> = collections.iter().map(|c| index_name(c)).collect();
    names.sort_unstable();
    names.dedup();
    format!("{}/{}:{:x}", db_name, names.join(","), hasher.finish())
}

/// Full cache-key derivation for a parsed read query: `None` means "do not
/// cache" — the query's collection set cannot be determined from the AST, it
/// names something that is not a collection of `db_name` (a search view, a
/// named graph), or its result depends on state no invalidation tracks.
pub fn cache_key_for(
    storage: &crate::storage::StorageEngine,
    db_name: &str,
    query_text: &str,
    query: &crate::sdbql::Query,
    bind_vars: &std::collections::HashMap<String, serde_json::Value>,
    principal: &crate::sdbql::QueryPrincipal,
) -> Option<String> {
    let refs = cacheable_collections(query)?;
    let exists = |name: &str| {
        let full = if name.contains(':') {
            name.to_string()
        } else {
            format!("{}:{}", db_name, name)
        };
        storage.get_collection(&full).is_ok()
    };
    let mut collections = Vec::with_capacity(refs.required.len());
    for name in refs.required {
        // A search view or a graph name resolves to other collections at run
        // time; invalidating by its own name would never fire.
        if !exists(&name) {
            return None;
        }
        collections.push(name);
    }
    for name in refs.maybe {
        if exists(&name) {
            collections.push(name);
        }
    }
    Some(hash_query(
        db_name,
        query_text,
        bind_vars,
        principal,
        &collections,
    ))
}

/// Collections a query reads, as far as the AST says.
#[derive(Debug, Default, PartialEq)]
pub struct CollectionRefs {
    /// Names used as a collection that are not bound as a variable anywhere
    /// in the query: they must resolve to a real collection.
    pub required: Vec<String>,
    /// `FOR x IN name` where `name` is also bound by a LET/CTE/FOR somewhere:
    /// it is a collection only if one by that name exists.
    pub maybe: Vec<String>,
}

/// Functions whose result depends on the caller or on state the cache cannot
/// invalidate on (models, named graphs, vertex look-ups by `_id`, the catalog).
/// A query that calls one of these is never cached.
const UNCACHEABLE_FUNCTIONS: &[&str] = &[
    // Principal-dependent; the key carries the principal, this is defence in
    // depth (audit H1).
    "CURRENT_USER",
    "CURRENT_ROLES",
    "CAN",
    "ROW_POLICY",
    // Dynamic dispatch hides the function (and its collection) from the AST.
    "APPLY",
    "CALL",
    // Models and network calls.
    "EMBED",
    "EMBED_BATCH",
    "EXTRACT",
    "LLM",
    "CHAT",
    "RERANK",
    "RAG_PIPELINE",
    // Graph functions read edge collections chosen at run time plus whatever
    // vertex collections their `_id`s point at.
    "NEIGHBORS",
    "GRAPH_RAG",
    "GRAPH_RAG_SEARCH",
    "COMMUNITY_SEARCH",
    "PAGERANK",
    "DEGREE_CENTRALITY",
    "SHORTEST_PATH",
    "K_PATHS",
    "GRAPH_INFO",
    // Catalog DDL (also mutating, so never reaches the cache anyway).
    "CREATE_VIEW",
    "DROP_VIEW",
    "CREATE_GRAPH",
    "DROP_GRAPH",
];

/// Functions whose first argument names the collection they read. The name
/// must be a literal for the query to be cacheable.
const COLLECTION_ARG_FUNCTIONS: &[&str] = &[
    "FULLTEXT",
    "SAMPLE",
    "COLLECTION_COUNT",
    "HYBRID_SEARCH",
    "VECTOR_SEARCH",
    "VECTOR_INDEX_STATS",
    "DOC_AS_OF",
    "DOC_HISTORY",
    "SNAPSHOT_DIFF",
    "SEARCH_INDEX",
];

#[derive(Default)]
struct CollectionWalk {
    /// Names read as a collection (FOR/JOIN sources, function arguments).
    sources: std::collections::BTreeSet<String>,
    /// Names only FOR sources can be confused with: LET, CTE and loop variables.
    bound: HashSet<String>,
    /// Set once something makes the collection set unknowable.
    unknown: bool,
}

impl CollectionWalk {
    fn source(&mut self, name: &str) {
        let name = name.trim_matches('`');
        if name.is_empty() {
            return;
        }
        self.sources.insert(name.to_string());
    }

    fn query(&mut self, q: &crate::sdbql::Query) {
        use crate::sdbql::ast::BodyClause;

        if q.create_stream_clause.is_some()
            || q.create_materialized_view_clause.is_some()
            || q.refresh_materialized_view_clause.is_some()
            || q.window_clause.is_some()
        {
            self.unknown = true;
            return;
        }
        if let Some(with) = &q.with_clause {
            for cte in &with.ctes {
                self.bound.insert(cte.name.clone());
                self.query(&cte.query);
            }
        }
        for l in q.let_clauses.iter().chain(q.post_limit_lets.iter()) {
            self.bound.insert(l.variable.clone());
            self.expr(&l.expression);
        }
        for f in &q.for_clauses {
            self.for_clause(f);
        }
        for j in &q.join_clauses {
            self.join(j);
        }
        for f in &q.filter_clauses {
            self.expr(&f.expression);
        }
        if let Some(sort) = &q.sort_clause {
            for (e, _) in &sort.fields {
                self.expr(e);
            }
        }
        if let Some(limit) = &q.limit_clause {
            self.expr(&limit.offset);
            if let Some(c) = &limit.count {
                self.expr(c);
            }
        }
        if let Some(r) = &q.return_clause {
            self.expr(&r.expression);
        }
        for clause in &q.body_clauses {
            match clause {
                BodyClause::For(f) => self.for_clause(f),
                BodyClause::Let(l) => {
                    self.bound.insert(l.variable.clone());
                    self.expr(&l.expression);
                }
                BodyClause::Filter(f) | BodyClause::Search(f) => self.expr(&f.expression),
                BodyClause::Join(j) => self.join(j),
                // Traversals fetch vertices from whatever collections the
                // edges' `_from`/`_to` name, and a graph name resolves through
                // `_graphs` at run time.
                BodyClause::GraphTraversal(_) | BodyClause::ShortestPath(_) => {
                    self.unknown = true;
                }
                BodyClause::Collect(c) => {
                    for (v, e) in &c.group_vars {
                        self.bound.insert(v.clone());
                        self.expr(e);
                    }
                    for a in &c.aggregates {
                        self.bound.insert(a.variable.clone());
                        if let Some(e) = &a.argument {
                            self.expr(e);
                        }
                    }
                    if let Some(v) = &c.into_var {
                        self.bound.insert(v.clone());
                    }
                    if let Some(v) = &c.count_var {
                        self.bound.insert(v.clone());
                    }
                }
                BodyClause::Insert(i) => {
                    self.source(&i.collection);
                    self.expr(&i.document);
                }
                BodyClause::Update(u) => {
                    self.source(&u.collection);
                    self.expr(&u.selector);
                    self.expr(&u.changes);
                }
                BodyClause::Upsert(u) => {
                    self.source(&u.collection);
                    self.expr(&u.search);
                    self.expr(&u.insert);
                    self.expr(&u.update);
                }
                BodyClause::Remove(r) => {
                    self.source(&r.collection);
                    self.expr(&r.selector);
                }
                BodyClause::Window(_) => self.unknown = true,
            }
        }
        for op in &q.set_operations {
            self.query(&op.query);
        }
    }

    fn for_clause(&mut self, f: &crate::sdbql::ast::ForClause) {
        use crate::sdbql::ast::ValidTimeSpec;
        self.bound.insert(f.variable.clone());
        if let Some(e) = &f.source_expression {
            self.expr(e);
        } else {
            self.source(&f.collection);
        }
        if let Some(e) = &f.system_time {
            self.expr(e);
        }
        match &f.valid_time {
            Some(ValidTimeSpec::AsOf(e)) => self.expr(e),
            Some(ValidTimeSpec::Range { from, to }) => {
                self.expr(from);
                self.expr(to);
            }
            None => {}
        }
    }

    fn join(&mut self, j: &crate::sdbql::ast::JoinClause) {
        self.bound.insert(j.variable.clone());
        self.source(&j.collection);
        self.expr(&j.condition);
        if let Some(asof) = &j.asof {
            self.expr(&asof.left_time);
            self.expr(&asof.right_time);
            if let Some(t) = &asof.tolerance {
                self.expr(t);
            }
        }
    }

    fn expr(&mut self, e: &crate::sdbql::ast::Expression) {
        use crate::sdbql::ast::{BinaryOperator, Expression};
        if self.unknown {
            return;
        }
        match e {
            Expression::Subquery(q) => self.query(q),
            Expression::FunctionCall { name, args } => {
                self.function(name, args);
                for a in args {
                    self.expr(a);
                }
            }
            Expression::WindowFunctionCall { function, .. } => {
                if UNCACHEABLE_FUNCTIONS
                    .iter()
                    .any(|f| function.eq_ignore_ascii_case(f))
                {
                    self.unknown = true;
                }
                e.for_each_child(&mut |c| self.expr(c));
            }
            // `~~` embeds its operands with a model.
            Expression::BinaryOp {
                op: BinaryOperator::SemanticMatch,
                ..
            } => self.unknown = true,
            Expression::Lambda { params, body } => {
                self.bound.extend(params.iter().cloned());
                self.expr(body);
            }
            other => other.for_each_child(&mut |c| self.expr(c)),
        }
    }

    fn function(&mut self, name: &str, args: &[crate::sdbql::ast::Expression]) {
        use crate::sdbql::ast::Expression;
        let upper = name.to_ascii_uppercase();
        if UNCACHEABLE_FUNCTIONS.contains(&upper.as_str()) {
            self.unknown = true;
            return;
        }
        if COLLECTION_ARG_FUNCTIONS.contains(&upper.as_str()) {
            match args.first() {
                Some(Expression::Literal(serde_json::Value::String(c))) => self.source(c),
                _ => self.unknown = true,
            }
            return;
        }
        if upper == "DOCUMENT" {
            // DOCUMENT("c/k"), DOCUMENT(["c/k", ...]) or DOCUMENT("c", "k"):
            // cacheable only when every collection is spelled out literally.
            let id_collection = |v: &serde_json::Value| -> Option<String> {
                v.as_str()
                    .and_then(|s| s.split_once('/'))
                    .map(|(c, _)| c.to_string())
            };
            match args {
                [Expression::Literal(serde_json::Value::String(c)), _] => self.source(c),
                [Expression::Literal(v @ serde_json::Value::String(_))] => match id_collection(v) {
                    Some(c) => self.source(&c),
                    None => self.unknown = true,
                },
                [Expression::Literal(serde_json::Value::Array(ids))] => {
                    for id in ids {
                        match id_collection(id) {
                            Some(c) => self.source(&c),
                            None => self.unknown = true,
                        }
                    }
                }
                [Expression::Array(items)] => {
                    for item in items {
                        match item {
                            Expression::Literal(v) => match id_collection(v) {
                                Some(c) => self.source(&c),
                                None => self.unknown = true,
                            },
                            _ => self.unknown = true,
                        }
                    }
                }
                _ => self.unknown = true,
            }
        }
    }
}

/// The collections a parsed query reads, from the AST — every FOR/JOIN
/// source, subqueries, CTEs and set-operation operands, and the literal
/// collection arguments of `DOCUMENT()` and the search functions.
///
/// `None` when the set cannot be determined statically (a collection named by
/// a variable or bind variable, a graph traversal, a model call, ...): the
/// caller must then not cache (audit P2). This replaces a scan for the
/// literal `" IN "`, which missed newlines, tabs, JOIN targets, `DOCUMENT()`
/// and backticked names.
pub fn cacheable_collections(query: &crate::sdbql::Query) -> Option<CollectionRefs> {
    let mut walk = CollectionWalk::default();
    walk.query(query);
    if walk.unknown {
        return None;
    }
    let mut refs = CollectionRefs::default();
    for name in walk.sources {
        if walk.bound.contains(&name) {
            refs.maybe.push(name);
        } else {
            refs.required.push(name);
        }
    }
    Some(refs)
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

    fn refs(q: &str) -> Option<CollectionRefs> {
        cacheable_collections(&crate::sdbql::parse(q).unwrap())
    }

    fn required(q: &str) -> Vec<String> {
        refs(q).expect("cacheable").required
    }

    #[test]
    fn test_hash_query_format() {
        let p = crate::sdbql::QueryPrincipal::from_roles("alice", vec!["viewer".into()]);
        let key = hash_query(
            "mydb",
            "FOR doc IN users RETURN doc",
            &std::collections::HashMap::new(),
            &p,
            &["users".to_string()],
        );
        assert!(key.starts_with("mydb/users:"));
        assert_eq!(
            extract_collections_from_key(&key),
            vec!["users".to_string()]
        );
    }

    #[test]
    fn test_key_differs_per_principal() {
        let q = "FOR o IN orders RETURN o";
        let vars = std::collections::HashMap::new();
        let colls = ["orders".to_string()];
        let admin = crate::sdbql::QueryPrincipal::from_roles("root", vec!["admin".into()]);
        let viewer = crate::sdbql::QueryPrincipal::from_roles("bob", vec!["viewer".into()]);
        let viewer2 = crate::sdbql::QueryPrincipal::from_roles("carol", vec!["viewer".into()]);
        let k_admin = hash_query("db", q, &vars, &admin, &colls);
        let k_viewer = hash_query("db", q, &vars, &viewer, &colls);
        let k_viewer2 = hash_query("db", q, &vars, &viewer2, &colls);
        assert_ne!(k_admin, k_viewer);
        // Same roles, different user: CURRENT_USER-based row policies differ.
        assert_ne!(k_viewer, k_viewer2);
        // Role order does not matter.
        let a = crate::sdbql::QueryPrincipal::from_roles("u", vec!["a".into(), "b".into()]);
        let b = crate::sdbql::QueryPrincipal::from_roles("u", vec!["b".into(), "a".into()]);
        assert_eq!(
            hash_query("db", q, &vars, &a, &colls),
            hash_query("db", q, &vars, &b, &colls)
        );
    }

    #[test]
    fn test_collections_from_ast() {
        assert_eq!(required("FOR doc IN users RETURN doc"), vec!["users"]);
        // Newlines / tabs around IN, which the old " IN " scan missed.
        assert_eq!(required("FOR doc\nIN\tusers\nRETURN doc"), vec!["users"]);
        // Subquery and JOIN-like nesting.
        let mut got = required(
            "FOR u IN users LET os = (FOR o IN orders FILTER o.u == u._key RETURN o) RETURN os",
        );
        got.sort();
        assert_eq!(got, vec!["orders", "users"]);
        // DOCUMENT with a literal id.
        let mut got = required("FOR u IN users RETURN DOCUMENT(\"teams/t1\")");
        got.sort();
        assert_eq!(got, vec!["teams", "users"]);
    }

    #[test]
    fn test_uncacheable_queries() {
        // Principal-dependent functions.
        assert!(refs("RETURN CURRENT_USER()").is_none());
        assert!(refs("FOR d IN docs FILTER CAN(\"read\", d) RETURN d").is_none());
        assert!(refs("RETURN CURRENT_ROLES()").is_none());
        // Collection chosen at run time.
        assert!(refs("FOR d IN docs RETURN DOCUMENT(d.ref)").is_none());
        assert!(refs("RETURN COLLECTION_COUNT(@c)").is_none());
        // Graph traversal reads vertex collections by _id.
        assert!(refs("FOR v IN 1..2 OUTBOUND \"users/a\" follows RETURN v").is_none());
    }

    #[test]
    fn test_let_variable_is_not_required_collection() {
        let r = refs("LET xs = [1, 2] FOR x IN xs RETURN x").expect("cacheable");
        assert!(r.required.is_empty());
        assert_eq!(r.maybe, vec!["xs".to_string()]);
    }

    #[test]
    fn test_put_if_current_skips_after_invalidation() {
        let cache = QueryCache::new(10, 60);
        let gen = current_generation();
        // Any invalidation (on any cache instance) bumps the generation.
        cache.invalidate_collection("users");
        cache.put_if_current("db/users:1".to_string(), vec![json!(1)], gen);
        assert!(cache.get("db/users:1").is_none());
        let gen = current_generation();
        cache.put_if_current("db/users:1".to_string(), vec![json!(1)], gen);
        // Another test may bump the global generation concurrently; only
        // assert the negative case strictly.
        let _ = cache.get("db/users:1");
    }

    #[test]
    fn test_index_name_normalises() {
        assert_eq!(index_name("mydb:users"), "users");
        assert_eq!(index_name("users"), "users");
        assert_eq!(index_name("`users`"), "users");
    }
}
