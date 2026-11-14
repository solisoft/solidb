//! What graph traversals read, kept between queries.
//!
//! Per collection: where each vertex's edges lead (with or without the
//! edges' keys), the documents read with each projection, and whole
//! neighbourhoods with their documents. A database that keeps its graph in
//! memory walks it without touching storage; these maps give a graph that is
//! read more than it is written the same, filled as it is walked.
//!
//! **What keeps them right.** Every document write reaches
//! `Collection::emit_change` or `Collection::invalidate_cached_queries`.
//! - A single write (`emit_change`) forgets exactly what it touches
//!   ([`changed`]): the document under every projection, and for an edge
//!   the neighbourhoods of both its ends, as they were and as they are.
//! - Anything else (a batch, a truncate, an index created or dropped) moves
//!   the collection's coarse generation, which empties every map of it.
//! - A reader that fills a map read storage before or after a concurrent
//!   write: it notes the map's epoch before reading and its entry is kept
//!   only if nothing was forgotten meanwhile ([`Kept::put`]).
//!
//! A write that reaches neither function would leave a traversal reading
//! what was there before, as it would leave the query cache.

use super::collection::ChangeEvent;
use super::keyspace::KsState;
use dashmap::DashMap;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

pub(crate) type Ends = Arc<[Arc<str>]>;
pub(crate) type Neighbourhoods = Kept<Ends>;
/// `(far end, edge key)` of each of a vertex's edges.
pub(crate) type Steps = Arc<[(Arc<str>, Box<str>)]>;
/// The documents of a collection as one projection of a traversal reads
/// them.
pub(crate) type Vertices = Kept<Arc<Value>>;
/// (neighbour, its document, the edge that leads there when the entry was
/// built for a query that reads the edges)
pub(crate) type Step = (Arc<str>, Option<Arc<Value>>, Option<Arc<Value>>);

/// Values read from a collection, by key, valid for one coarse generation
/// of it.
pub(crate) struct Kept<V> {
    state: Arc<KsState>,
    generation: u64,
    values: DashMap<Box<str>, (V, usize), foldhash::fast::RandomState>,
    bytes: AtomicUsize,
    /// Moved each time an entry is forgotten for a write.
    epoch: AtomicU64,
}

/// `SOLIDB_GRAPH_CACHE_MB`: what one such map may hold (default 256, `0`
/// turns them off).
fn budget() -> usize {
    static BUDGET: once_cell::sync::Lazy<usize> = once_cell::sync::Lazy::new(|| {
        std::env::var("SOLIDB_GRAPH_CACHE_MB")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(256)
            << 20
    });
    *BUDGET
}

type Registry<V> = once_cell::sync::Lazy<DashMap<(usize, u64), Arc<Kept<V>>>>;
/// Keyed by the collection's shared write state (its address, which the
/// entry keeps alive) and what is kept: the direction, or the fields.
static ENDS: Registry<Ends> = once_cell::sync::Lazy::new(DashMap::new);
static STEPS: Registry<Steps> = once_cell::sync::Lazy::new(DashMap::new);
static FACTS: Registry<bool> = once_cell::sync::Lazy::new(DashMap::new);
static AROUND: Registry<Arc<Around>> = once_cell::sync::Lazy::new(DashMap::new);
static VERTICES: Registry<Arc<Value>> = once_cell::sync::Lazy::new(DashMap::new);
/// The collections (by write state) that have a map: a write to any other
/// has nothing to forget.
static WALKED: once_cell::sync::Lazy<DashMap<usize, ()>> = once_cell::sync::Lazy::new(DashMap::new);

fn open<V>(
    registry: &DashMap<(usize, u64), Arc<Kept<V>>>,
    state: &Arc<KsState>,
    kind: u64,
) -> Option<Arc<Kept<V>>> {
    if budget() == 0 || state.is_dead() {
        return None;
    }
    let address = Arc::as_ptr(state) as usize;
    let key = (address, kind);
    let generation = state.coarse_generation();
    if let Some(found) = registry.get(&key) {
        if found.generation == generation {
            return Some(found.clone());
        }
    }
    // A new generation, or a first use: start empty. Entries of dropped
    // collections go at the same time.
    registry.retain(|_, kept| !kept.state.is_dead());
    WALKED.insert(address, ());
    let fresh = Arc::new(Kept {
        state: state.clone(),
        generation,
        values: DashMap::with_hasher(foldhash::fast::RandomState::default()),
        bytes: AtomicUsize::new(0),
        epoch: AtomicU64::new(0),
    });
    registry.insert(key, fresh.clone());
    Some(fresh)
}

/// The neighbourhoods of this collection in `direction` at its current
/// generation, or `None` when the cache is off.
pub(crate) fn of(state: &Arc<KsState>, direction: u8) -> Option<Arc<Neighbourhoods>> {
    open(&ENDS, state, direction as u64)
}

/// A yes-or-no fact about the collection's content (`which` tells them
/// apart), computed once per coarse generation. A single write that could
/// change it must move that generation (see [`changed`]).
pub(crate) fn fact(state: &Arc<KsState>, which: u64, compute: impl FnOnce() -> bool) -> bool {
    let Some(kept) = open(&FACTS, state, which) else {
        return compute();
    };
    kept.get("").unwrap_or_else(|| {
        let seen = kept.epoch();
        let fact = compute();
        kept.put("", &fact, 1, seen);
        fact
    })
}

/// The same as [`of`] with the key of each edge.
pub(crate) fn steps(state: &Arc<KsState>, direction: u8) -> Option<Arc<Kept<Steps>>> {
    open(&STEPS, state, direction as u64)
}

/// The documents of this collection as read with `fields` (`None`: whole).
pub(crate) fn vertices(state: &Arc<KsState>, fields: Option<&[String]>) -> Option<Arc<Vertices>> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    fields.hash(&mut hasher);
    open(&VERTICES, state, hasher.finish())
}

/// A vertex's neighbours, each with its document as one projection reads
/// it: what a walk needs of a whole neighbourhood, in one piece. The
/// documents belong to other collections than the edges, so the entry also
/// records the write generation of each of them when it was read, and is
/// used only while none has moved.
pub(crate) struct Around {
    stamps: Box<[(Arc<KsState>, u64)]>,
    pub(crate) ends: Box<[Step]>,
}

impl Around {
    pub(crate) fn new(stamps: Vec<(Arc<KsState>, u64)>, ends: Vec<Step>) -> Self {
        Self {
            stamps: stamps.into(),
            ends: ends.into(),
        }
    }

    /// Whether the vertex collections have not been written since.
    pub(crate) fn current(&self) -> bool {
        self.stamps
            .iter()
            .all(|(state, generation)| !state.is_dead() && state.write_generation() == *generation)
    }
}

/// The neighbourhoods-with-documents of this edge collection in
/// `direction`, for the vertex fields `fields` and, when the query reads
/// the edges, their fields `edges`.
pub(crate) fn around(
    state: &Arc<KsState>,
    direction: u8,
    fields: Option<&[String]>,
    edges: Option<Option<&[String]>>,
) -> Option<Arc<Kept<Arc<Around>>>> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    fields.hash(&mut hasher);
    edges.hash(&mut hasher);
    open(&AROUND, state, (hasher.finish() << 8) | direction as u64)
}

impl<V: Clone> Kept<V> {
    pub(crate) fn get(&self, key: &str) -> Option<V> {
        self.values.get(key).map(|value| value.0.clone())
    }

    /// To note before reading from storage what will be [`Self::put`].
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Keep `value` (of about `size` bytes) for `key`, read from storage
    /// after `seen` was noted: unless the map is full, or something of the
    /// collection has been written since (what was read may then be older
    /// than a write that already forgot its entry).
    pub(crate) fn put(&self, key: &str, value: &V, size: usize, seen: u64) {
        if self.state.coarse_generation() != self.generation || self.epoch() != seen {
            return;
        }
        let size = size + key.len() + 64;
        if self.bytes.fetch_add(size, Ordering::Relaxed) + size > budget() {
            self.bytes.fetch_sub(size, Ordering::Relaxed);
            return;
        }
        self.values.insert(key.into(), (value.clone(), size));
        // A write that came in between forgot the key before it was there.
        if self.epoch() != seen {
            self.drop_entry(key);
        }
    }

    fn drop_entry(&self, key: &str) {
        if let Some((_, (_, size))) = self.values.remove(key) {
            self.bytes.fetch_sub(size, Ordering::Relaxed);
        }
    }

    /// A write changed what `key` should read as.
    fn forget(&self, key: &str) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.drop_entry(key);
    }
}

fn forget_in<V: Clone>(registry: &DashMap<(usize, u64), Arc<Kept<V>>>, address: usize, key: &str) {
    for kept in registry.iter().filter(|kept| kept.key().0 == address) {
        kept.value().forget(key);
    }
}

/// One document of the collection `name` (`"db:collection"`), of write
/// state `state`, was inserted, updated or deleted: forget what is kept of
/// it. Returns `false` when that cannot be done from the event, and the
/// caller must move the coarse generation instead: an edge whose ends the
/// event does not give, or one written with the database in its ids (which
/// changes how every vertex of the collection is looked up).
pub(crate) fn changed(
    state: &Arc<KsState>,
    name: &str,
    is_edge: bool,
    event: &ChangeEvent,
) -> bool {
    let address = Arc::as_ptr(state) as usize;
    if !WALKED.contains_key(&address) {
        return true;
    }
    // The document itself, under every projection (edges are read that way
    // too).
    forget_in(&VERTICES, address, &event.key);
    if !is_edge {
        return true;
    }
    let db_prefix = name.split_once(':').map(|(db, _)| format!("{db}:"));
    let mut ends: Vec<&str> = Vec::with_capacity(4);
    for doc in [event.data.as_ref(), event.old_data.as_ref()]
        .into_iter()
        .flatten()
    {
        for field in ["_from", "_to"] {
            let Some(id) = doc.get(field).and_then(Value::as_str) else {
                return false;
            };
            match db_prefix
                .as_deref()
                .and_then(|prefix| id.strip_prefix(prefix))
            {
                Some(_) => return false,
                None => ends.push(id),
            }
        }
    }
    // An update or a delete says what the edge was only through `old_data`.
    let known = match event.type_ {
        super::collection::ChangeType::Insert => event.data.is_some(),
        super::collection::ChangeType::Update => event.data.is_some() && event.old_data.is_some(),
        super::collection::ChangeType::Delete => event.old_data.is_some() || event.data.is_some(),
        super::collection::ChangeType::Truncate => false,
    };
    if !known {
        return false;
    }
    for end in ends {
        forget_in(&ENDS, address, end);
        forget_in(&STEPS, address, end);
        forget_in(&AROUND, address, end);
    }
    true
}
