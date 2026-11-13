//! Foreign keys: a field of one collection (the *child*) that must hold the
//! `_key` of a document of another collection of the same database (the
//! *parent*), or null.
//!
//! A constraint is declared on the child and stored in its keyspace
//! (`_stats:foreign_keys`). Two things enforce it:
//!
//! - a write that gives a child a reference checks that the parent exists
//!   ([`Collection::check_references`], and [`check_operations`] for a
//!   transaction's commit);
//! - a delete in a referenced collection looks the children up through the
//!   index on the referring field, and is refused, deletes them or sets
//!   their field to null (`on_delete`), in the same write as the delete
//!   ([`check_operations`]).
//!
//! What makes the two hold together is the *gate*, one per database: every
//! write to a collection that takes part in a constraint — child or parent —
//! holds it shared, a delete in a referenced collection holds it
//! exclusively. So a parent cannot go between a child's check and its
//! write, and a cascade is the only writer of the collections it walks: it
//! needs no order between their key locks. The gate is taken before any key
//! lock. A collection that takes part in no constraint never touches it.
//!
//! Not covered (see `doc/`): sharded collections, and nodes that replicate —
//! a cascade is written by storage and does not reach the replication log.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use dashmap::DashMap;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::collection::Collection;
use super::database::Database;
use super::keyspace::{Keyspace, KsDbExt};
use super::pending_drops::PendingCfDrops;
use super::RocksDb as DB;
use crate::error::{DbError, DbResult};
use crate::transaction::Operation;

/// Where a child collection keeps its constraints.
pub const FOREIGN_KEYS_KEY: &str = "_stats:foreign_keys";

/// What happens to the children of a document that is deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnDelete {
    /// The delete is refused while a child refers to the document.
    #[default]
    Restrict,
    /// The children are deleted with it (and theirs, in turn).
    Cascade,
    /// The children's field is set to null.
    SetNull,
}

/// What the child's field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceBy {
    /// The parent's `_key`.
    #[default]
    Key,
    /// The parent's `_id`, with or without its database: `customers/ann`
    /// or `shop:customers/ann`. What an edge's `_from` and `_to` hold.
    Id,
}

impl ReferenceBy {
    fn is_key(&self) -> bool {
        *self == ReferenceBy::Key
    }
}

/// One constraint, as stored on the child collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKey {
    pub name: String,
    /// The child's field that holds the reference (top level).
    pub field: String,
    /// Whether the field holds the parent's `_key` or its `_id`.
    #[serde(default, skip_serializing_if = "ReferenceBy::is_key")]
    pub by: ReferenceBy,
    /// The parent collection, in the child's database.
    pub references: String,
    #[serde(default)]
    pub on_delete: OnDelete,
    /// The index on `field` the constraint looks children up through.
    pub index: String,
}

/// A constraint seen from the child: where its parent's documents live.
pub(crate) struct Outgoing {
    pub key: ForeignKey,
    pub parent: String,
    pub parent_ks: Option<Keyspace>,
}

/// A constraint seen from the parent.
pub(crate) struct Referrer {
    /// The child collection, `db:coll`.
    pub child: String,
    pub key: ForeignKey,
}

/// What one collection has to do with foreign keys.
pub(crate) struct Links {
    pub outgoing: Vec<Outgoing>,
    pub referrers: Vec<Referrer>,
    pub gate: Arc<Gate>,
}

/// Moves whenever a constraint, or a collection that takes part in one,
/// is created or dropped. Whatever was read before is read again.
static EPOCH: AtomicU64 = AtomicU64::new(0);

pub(crate) fn epoch() -> u64 {
    EPOCH.load(Ordering::Acquire)
}

pub(crate) fn bump_epoch() {
    EPOCH.fetch_add(1, Ordering::AcqRel);
}

struct DbLinks {
    db: Weak<DB>,
    epoch: u64,
    by_collection: HashMap<String, Arc<Links>>,
}

type DbId = (usize, String);

static LINKS: Lazy<DashMap<DbId, Arc<DbLinks>>> = Lazy::new(DashMap::new);
static GATES: Lazy<DashMap<DbId, Arc<Gate>>> = Lazy::new(DashMap::new);

fn db_id(db: &Arc<DB>, db_name: &str) -> DbId {
    (Arc::as_ptr(db) as usize, db_name.to_string())
}

fn split(full_name: &str) -> (&str, &str) {
    full_name.split_once(':').unwrap_or(("", full_name))
}

/// The gate of the database `db_name`. Kept for the life of the process: a
/// constraint dropped and declared again must find the same one.
pub(crate) fn gate_of(db: &Arc<DB>, db_name: &str) -> Arc<Gate> {
    let id = db_id(db, db_name);
    if let Some(gate) = GATES.get(&id) {
        return Arc::clone(&gate);
    }
    Arc::clone(&GATES.entry(id).or_default())
}

/// The constraints stored on the collection whose keys live in `ks`.
pub(crate) fn stored(db: &Arc<DB>, ks: &Keyspace) -> Vec<ForeignKey> {
    let Some(cf) = ks.handle(db) else {
        return Vec::new();
    };
    db.get_ks(&cf, FOREIGN_KEYS_KEY.as_bytes())
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn build(db: &Arc<DB>, db_name: &str, epoch: u64) -> DbLinks {
    let mut outgoing: HashMap<String, Vec<Outgoing>> = HashMap::new();
    let mut referrers: HashMap<String, Vec<Referrer>> = HashMap::new();
    for collection in super::collection_registry::list(db, db_name) {
        let child = format!("{}:{}", db_name, collection);
        let Some(ks) = super::collection_registry::keyspace_of(db, &child, |_| false) else {
            continue;
        };
        for key in stored(db, &ks) {
            let parent = format!("{}:{}", db_name, key.references);
            referrers.entry(parent.clone()).or_default().push(Referrer {
                child: child.clone(),
                key: key.clone(),
            });
            let parent_ks = super::collection_registry::keyspace_of(db, &parent, |_| false);
            outgoing.entry(child.clone()).or_default().push(Outgoing {
                key,
                parent,
                parent_ks,
            });
        }
    }

    let mut by_collection = HashMap::new();
    if !outgoing.is_empty() {
        let gate = gate_of(db, db_name);
        let names: Vec<String> = outgoing.keys().chain(referrers.keys()).cloned().collect();
        for name in names {
            if by_collection.contains_key(&name) {
                continue;
            }
            let links = Links {
                outgoing: outgoing.remove(&name).unwrap_or_default(),
                referrers: referrers.remove(&name).unwrap_or_default(),
                gate: Arc::clone(&gate),
            };
            by_collection.insert(name, Arc::new(links));
        }
    }
    DbLinks {
        db: Arc::downgrade(db),
        epoch,
        by_collection,
    }
}

thread_local! {
    /// How many [`unchecked`] calls this thread is inside.
    static UNCHECKED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Run `apply` with foreign keys neither checked nor followed on this
/// thread: for a write that another node already checked and that arrives
/// here by replication. Entries can arrive out of order (a child before
/// its parent), and a cascade arrives as its own entries; checking or
/// cascading again would make this node refuse or repeat what the origin
/// did. `apply` must not hand the work to another thread.
pub fn unchecked<T>(apply: impl FnOnce() -> T) -> T {
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            UNCHECKED.with(|n| n.set(n.get().saturating_sub(1)));
        }
    }
    UNCHECKED.with(|n| n.set(n.get() + 1));
    let _leave = Leave;
    apply()
}

pub(crate) fn is_unchecked() -> bool {
    UNCHECKED.with(|n| n.get() > 0)
}

/// A write a delete cascaded into: the child's document as it now is, or
/// `None` when it was deleted.
#[derive(Debug, Clone)]
pub struct Cascaded {
    pub database: String,
    pub collection: String,
    pub key: String,
    pub document: Option<Value>,
}

pub(crate) type CascadeObserver = Arc<dyn Fn(Vec<Cascaded>) + Send + Sync>;

static OBSERVERS: Lazy<DashMap<usize, CascadeObserver>> = Lazy::new(DashMap::new);

/// Have `observer` told of every write a delete cascades into on `db`,
/// once it is stored: how they reach the replication log, which is fed
/// above storage and sees only the delete that was asked for.
pub(crate) fn observe_cascades(db: &Arc<DB>, observer: CascadeObserver) {
    OBSERVERS.insert(Arc::as_ptr(db) as usize, observer);
}

pub(crate) fn cascade_observer(db: &Arc<DB>) -> Option<CascadeObserver> {
    OBSERVERS
        .get(&(Arc::as_ptr(db) as usize))
        .map(|o| Arc::clone(&o))
}

/// What the collection `full_name` (`db:coll`) has to do with foreign keys;
/// `None`, for nearly every collection, when nothing.
pub(crate) fn links_of(db: &Arc<DB>, full_name: &str) -> Option<Arc<Links>> {
    if is_unchecked() {
        return None;
    }
    let (db_name, _) = split(full_name);
    let id = db_id(db, db_name);
    let now = epoch();
    if let Some(known) = LINKS.get(&id) {
        if known.epoch == now && Weak::as_ptr(&known.db) == Arc::as_ptr(db) {
            return known.by_collection.get(full_name).cloned();
        }
    }
    let built = Arc::new(build(db, db_name, now));
    let links = built.by_collection.get(full_name).cloned();
    LINKS.insert(id, built);
    links
}

/// Before the collection `full_name` is dropped: refused while another
/// collection refers to it. Returns whether it takes part in a constraint,
/// in which case the caller moves the epoch once it is gone
/// ([`bump_epoch`]).
pub(crate) fn before_drop(db: &Arc<DB>, full_name: &str) -> DbResult<bool> {
    // A replicated drop is applied whatever refers to the collection here,
    // but what was read of it still has to be forgotten.
    if is_unchecked() {
        return Ok(true);
    }
    let Some(links) = links_of(db, full_name) else {
        return Ok(false);
    };
    if let Some(referrer) = links.referrers.iter().find(|r| r.child != full_name) {
        return Err(DbError::ConflictError(format!(
            "Collection '{}' is referenced by the foreign key '{}' of '{}': drop that foreign key first",
            split(full_name).1,
            referrer.key.name,
            split(&referrer.child).1
        )));
    }
    Ok(true)
}

/// The database `db_name` is dropped: what was read of it is forgotten,
/// and the epoch moves if it held a constraint.
pub(crate) fn forget_database(db: &Arc<DB>, db_name: &str) {
    if let Some((_, known)) = LINKS.remove(&db_id(db, db_name)) {
        if !known.by_collection.is_empty() {
            bump_epoch();
        }
    }
}

// ---------------------------------------------------------------- the gate

/// See the module documentation.
#[derive(Default)]
pub(crate) struct Gate(parking_lot::RwLock<()>);

thread_local! {
    /// The gates this thread holds through [`Gate::enter`], so that a write
    /// nested in another (a cascade, a caller holding a document's lock)
    /// does not take its gate a second time: a second shared hold waits for
    /// ever behind a writer that waits for the first.
    static HELD: RefCell<Vec<(usize, bool)>> = const { RefCell::new(Vec::new()) };
}

/// A hold on a gate, released when dropped.
pub(crate) struct HeldGate {
    gate: Arc<Gate>,
    exclusive: bool,
    /// Entered on this thread, to be left on it.
    scoped: bool,
}

impl Gate {
    fn id(self: &Arc<Self>) -> usize {
        Arc::as_ptr(self) as usize
    }

    /// How this thread already holds the gate: `Some(exclusive)`.
    fn held_here(self: &Arc<Self>) -> Option<bool> {
        let id = self.id();
        HELD.with(|held| {
            held.borrow()
                .iter()
                .filter(|(gate, _)| *gate == id)
                .map(|(_, exclusive)| *exclusive)
                .max()
        })
    }

    fn take(self: &Arc<Self>, exclusive: bool, scoped: bool) -> DbResult<Option<HeldGate>> {
        match self.held_here() {
            Some(true) => return Ok(None),
            Some(false) if !exclusive => return Ok(None),
            Some(false) => return Err(DbError::ConflictError(
                "A document other documents refer to cannot be deleted from inside another write"
                    .to_string(),
            )),
            None => {}
        }
        if exclusive {
            std::mem::forget(self.0.write());
        } else {
            std::mem::forget(self.0.read());
        }
        if scoped {
            HELD.with(|held| held.borrow_mut().push((self.id(), exclusive)));
        }
        Ok(Some(HeldGate {
            gate: Arc::clone(self),
            exclusive,
            scoped,
        }))
    }

    /// Hold the gate for a write that begins and ends on this thread.
    /// `None` when the thread holds it already.
    pub(crate) fn enter(self: &Arc<Self>, exclusive: bool) -> DbResult<Option<HeldGate>> {
        self.take(exclusive, true)
    }

    /// Hold the gate as a value that may be released by another thread (a
    /// commit written by the group committer).
    pub(crate) fn hold(self: &Arc<Self>, exclusive: bool) -> DbResult<Option<HeldGate>> {
        self.take(exclusive, false)
    }
}

impl Drop for HeldGate {
    fn drop(&mut self) {
        if self.scoped {
            let id = self.gate.id();
            HELD.with(|held| {
                let mut held = held.borrow_mut();
                if let Some(at) = held.iter().rposition(|(gate, _)| *gate == id) {
                    held.remove(at);
                }
            });
        }
        // SAFETY: `take` locked the gate this way and forgot the guard; this
        // value is the only thing that releases it, once.
        unsafe {
            if self.exclusive {
                self.gate.0.force_unlock_write();
            } else {
                self.gate.0.force_unlock_read();
            }
        }
    }
}

// ------------------------------------------------- collections by their name

/// How to find the one cached handle of a collection of this instance: the
/// engine's own caches. A handle built aside would keep its own document
/// count, change channel and vector indexes.
struct Resolver {
    db: Weak<DB>,
    databases: Weak<DashMap<String, Database>>,
    collections: Weak<DashMap<String, Collection>>,
    pending: Weak<PendingCfDrops>,
}

static RESOLVERS: Lazy<DashMap<usize, Resolver>> = Lazy::new(DashMap::new);

/// Called by the storage engine when it opens `db`.
pub(crate) fn register_engine(
    db: &Arc<DB>,
    databases: &Arc<DashMap<String, Database>>,
    collections: &Arc<DashMap<String, Collection>>,
    pending: &Arc<PendingCfDrops>,
) {
    RESOLVERS.insert(
        Arc::as_ptr(db) as usize,
        Resolver {
            db: Arc::downgrade(db),
            databases: Arc::downgrade(databases),
            collections: Arc::downgrade(collections),
            pending: Arc::downgrade(pending),
        },
    );
}

/// Called when the last handle on the engine of `db` goes.
pub(crate) fn forget_engine(db: &Arc<DB>) {
    let id = Arc::as_ptr(db) as usize;
    RESOLVERS.remove(&id);
    OBSERVERS.remove(&id);
    LINKS.retain(|(owner, _), _| *owner != id);
}

/// The handle of the collection `full_name` (`db:coll`).
pub(crate) fn resolve(db: &Arc<DB>, full_name: &str) -> DbResult<Collection> {
    let (db_name, collection) = split(full_name);
    let engine = RESOLVERS.get(&(Arc::as_ptr(db) as usize)).and_then(|r| {
        if Weak::as_ptr(&r.db) != Arc::as_ptr(db) {
            return None;
        }
        Some((
            r.databases.upgrade()?,
            r.collections.upgrade()?,
            r.pending.upgrade()?,
        ))
    });
    if let Some((databases, collections, pending)) = engine {
        let known = databases.get(db_name).map(|d| d.clone());
        let database = match known {
            Some(database) => database,
            None => {
                let database = Database::new(db_name.to_string(), Arc::clone(db), pending)
                    .with_engine_cache(collections);
                databases
                    .entry(db_name.to_string())
                    .or_insert(database)
                    .clone()
            }
        };
        return database.system_collection(collection);
    }
    // No engine (a collection opened on its own, in tests).
    let ks = super::collection_registry::keyspace_of(db, full_name, |_| false)
        .ok_or_else(|| DbError::CollectionNotFound(collection.to_string()))?;
    Ok(Collection::open(full_name.to_string(), ks, Arc::clone(db)))
}

// ------------------------------------------------------------- the checks

pub(crate) fn violation(message: String) -> DbError {
    DbError::ConflictError(format!("Foreign key violation: {}", message))
}

/// The values a child's field may hold to refer to the parent document
/// `parent_key`: what the index on the field is searched for.
pub(crate) fn referring_values(key: &ForeignKey, parent: &str, parent_key: &str) -> Vec<String> {
    match key.by {
        ReferenceBy::Key => vec![parent_key.to_string()],
        ReferenceBy::Id => {
            let (database, collection) = split(parent);
            vec![
                format!("{}/{}", collection, parent_key),
                format!("{}:{}/{}", database, collection, parent_key),
            ]
        }
    }
}

/// The parent key a document refers to through `field`: `None` for null or
/// an absent field, an error for anything that cannot name a document of
/// the parent collection.
pub(crate) fn reference<'a>(
    key: &ForeignKey,
    child: &str,
    doc: &'a Value,
) -> DbResult<Option<&'a str>> {
    let value = match doc.get(&key.field) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(value)) => value.as_str(),
        Some(_) => {
            return Err(violation(format!(
                "'{}' in {} must be the {} of a document of '{}' (a string) or null",
                key.field,
                split(child).1,
                if key.by.is_key() { "_key" } else { "_id" },
                key.references
            )))
        }
    };
    if key.by.is_key() {
        return Ok(Some(value));
    }
    // `coll/key` or `db:coll/key`, of the parent collection in this database.
    let (database, _) = split(child);
    let local = match value.split_once(':') {
        Some((db, rest)) if !db.contains('/') => (db == database).then_some(rest),
        _ => Some(value),
    };
    match local.and_then(|id| id.split_once('/')) {
        Some((collection, parent_key)) if collection == key.references => Ok(Some(parent_key)),
        _ => Err(violation(format!(
            "'{}' in {} is '{}', which is not the _id of a document of '{}'",
            key.field,
            split(child).1,
            value,
            key.references
        ))),
    }
}

/// Whether the parent document `key` is stored.
pub(crate) fn parent_is_stored(db: &Arc<DB>, link: &Outgoing, key: &str) -> DbResult<bool> {
    let Some(cf) = link.parent_ks.as_ref().and_then(|ks| ks.handle(db)) else {
        return Ok(false);
    };
    db.get_pinned_ks(&cf, Collection::doc_key(key))
        .map(|found| found.is_some())
        .map_err(|e| DbError::InternalError(format!("Failed to read a referenced document: {}", e)))
}

pub(crate) fn missing_parent(link: &Outgoing, child: &str, parent_key: &str) -> DbError {
    violation(format!(
        "'{}' in {} refers to '{}', which is not a document of '{}'",
        link.key.field,
        split(child).1,
        parent_key,
        link.key.references
    ))
}

/// Documents a cascade may write before it is refused.
fn max_cascade() -> usize {
    std::env::var("SOLIDB_FK_MAX_CASCADE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(100_000)
}

/// A set of operations found to respect the foreign keys, with what its
/// write has to hold.
pub(crate) struct Checked {
    /// The operations, followed by those their deletes cascade into.
    pub operations: Vec<Operation>,
    /// Where the cascaded ones start.
    pub cascaded_from: usize,
    pub gates: Vec<HeldGate>,
}

fn full_name(op: &Operation) -> String {
    format!("{}:{}", op.database(), op.collection())
}

/// The value an operation leaves its document with: `None` once deleted.
fn outcome(op: &Operation) -> Option<Option<&Value>> {
    match op {
        Operation::Insert { data, .. } => Some(Some(data)),
        Operation::Update { new_data, .. } => Some(Some(new_data)),
        Operation::Delete { .. } => Some(None),
        _ => None,
    }
}

/// Check the operations of one atomic write (a transaction's commit, or a
/// delete in a referenced collection) against the foreign keys, as a whole:
/// a parent inserted by the same write exists, one it deletes does not.
/// Deletes in a referenced collection are followed to the children:
/// refused, or completed with the children's deletes or updates.
///
/// Takes the gates. Costs a lookup per distinct collection when none of
/// them takes part in a constraint.
pub(crate) fn check_operations(
    db: &Arc<DB>,
    mut operations: Vec<Operation>,
    resolve: &dyn Fn(&str) -> DbResult<Collection>,
) -> DbResult<Checked> {
    let mut links: HashMap<String, Option<Arc<Links>>> = HashMap::new();
    // database -> whether one of its referenced collections loses a document
    let mut databases: Vec<(String, bool, Arc<Gate>)> = Vec::new();
    for op in &operations {
        let Some(outcome) = outcome(op) else { continue };
        let name = full_name(op);
        let found = links
            .entry(name)
            .or_insert_with_key(|name| links_of(db, name));
        let Some(found) = found else { continue };
        let exclusive = outcome.is_none() && !found.referrers.is_empty();
        match databases
            .iter_mut()
            .find(|(name, ..)| name == op.database())
        {
            Some(entry) => entry.1 |= exclusive,
            None => databases.push((
                op.database().to_string(),
                exclusive,
                Arc::clone(&found.gate),
            )),
        }
    }
    if databases.is_empty() {
        return Ok(Checked {
            cascaded_from: operations.len(),
            operations,
            gates: Vec::new(),
        });
    }
    let cascaded_from = operations.len();

    databases.sort_by(|a, b| a.0.cmp(&b.0));
    let mut gates = Vec::new();
    for (_, exclusive, gate) in &databases {
        gates.extend(gate.hold(*exclusive)?);
    }

    // collection -> key -> the operation that decides its final state
    let mut view: HashMap<String, HashMap<String, usize>> = HashMap::new();
    let mut deletes: Vec<usize> = Vec::new();
    for (at, op) in operations.iter().enumerate() {
        if outcome(op).is_none() {
            continue;
        }
        view.entry(full_name(op))
            .or_default()
            .insert(op.key().to_string(), at);
    }
    for (at, op) in operations.iter().enumerate() {
        if matches!(op, Operation::Delete { .. }) {
            deletes.push(at);
        }
    }

    // Follow each delete to the children. `operations` grows as it goes.
    let limit = max_cascade();
    let mut added = 0usize;
    let mut next = 0;
    while next < deletes.len() {
        let at = deletes[next];
        next += 1;
        let parent = full_name(&operations[at]);
        let parent_key = operations[at].key().to_string();
        // Deleted, then inserted again by the same write: still there.
        if view.get(&parent).and_then(|keys| keys.get(&parent_key)) != Some(&at) {
            continue;
        }
        let parent_links = links
            .entry(parent.clone())
            .or_insert_with_key(|name| links_of(db, name))
            .clone();
        let Some(parent_links) = parent_links else {
            continue;
        };
        for referrer in &parent_links.referrers {
            let child = resolve(&referrer.child)?;
            let values = referring_values(&referrer.key, &parent, &parent_key);
            for child_key in child.keys_referencing(&referrer.key.field, &values)? {
                let touched = view
                    .get(&referrer.child)
                    .and_then(|keys| keys.get(&child_key))
                    .copied();
                if let Some(by) = touched {
                    // Its final state is this write's: checked below, with
                    // the other documents the write leaves behind.
                    let _ = by;
                    continue;
                }
                let (child_db, child_collection) = split(&referrer.child);
                match referrer.key.on_delete {
                    OnDelete::Restrict => {
                        return Err(violation(format!(
                            "'{}' in '{}' is still referred to by '{}' in '{}' ({}.{})",
                            parent_key,
                            split(&parent).1,
                            child_key,
                            child_collection,
                            child_collection,
                            referrer.key.field
                        )));
                    }
                    OnDelete::Cascade => {
                        let old_data = child.get(&child_key)?.to_value();
                        operations.push(Operation::Delete {
                            database: child_db.to_string(),
                            collection: child_collection.to_string(),
                            key: child_key.clone(),
                            old_data,
                        });
                        deletes.push(operations.len() - 1);
                    }
                    OnDelete::SetNull => {
                        let old_data = child.get(&child_key)?.to_value();
                        let mut change = serde_json::Map::new();
                        change.insert(referrer.key.field.clone(), Value::Null);
                        let new_data = Collection::updated_value(
                            &referrer.child,
                            &child_key,
                            &old_data,
                            Value::Object(change),
                        );
                        operations.push(Operation::Update {
                            database: child_db.to_string(),
                            collection: child_collection.to_string(),
                            key: child_key.clone(),
                            old_data,
                            new_data,
                        });
                    }
                }
                view.entry(referrer.child.clone())
                    .or_default()
                    .insert(child_key, operations.len() - 1);
                added += 1;
                if added > limit {
                    return Err(DbError::BadRequest(format!(
                        "Deleting '{}' from '{}' cascades into more than {} documents (SOLIDB_FK_MAX_CASCADE)",
                        parent_key,
                        split(&parent).1,
                        limit
                    )));
                }
            }
        }
    }

    // Every document the write leaves behind refers to a parent that is
    // there once it is written.
    for (child, keys) in &view {
        let child_links = links
            .entry(child.clone())
            .or_insert_with_key(|name| links_of(db, name))
            .clone();
        let Some(child_links) = child_links else {
            continue;
        };
        if child_links.outgoing.is_empty() {
            continue;
        }
        for &at in keys.values() {
            let op = &operations[at];
            let Some(Some(value)) = outcome(op) else {
                continue;
            };
            for link in &child_links.outgoing {
                let Some(parent_key) = reference(&link.key, child, value)? else {
                    continue;
                };
                let in_write = view
                    .get(&link.parent)
                    .and_then(|keys| keys.get(parent_key))
                    .map(|&by| matches!(outcome(&operations[by]), Some(Some(_))));
                let there = match in_write {
                    Some(there) => there,
                    None => {
                        let unchanged = matches!(op, Operation::Update { old_data, .. }
                            if old_data.get(&link.key.field) == value.get(&link.key.field));
                        unchanged || parent_is_stored(db, link, parent_key)?
                    }
                };
                if !there {
                    return Err(missing_parent(link, child, parent_key));
                }
            }
        }
    }

    Ok(Checked {
        operations,
        cascaded_from,
        gates,
    })
}
