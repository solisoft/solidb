//! Counter fields of a collection: declaring them, and giving an insert
//! its numbers (`storage::counters` holds the definition and the rules).

use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::counters::{
    self, state_key, CounterDef, CounterRecord, CounterState, COUNTERS_KEY,
};
use crate::storage::foreign_keys;
use crate::storage::keyspace::HeldCounterLock;
use crate::storage::kv::{KvBatch, KvWriteOptions};
use crate::transaction::Operation;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// What declares a counter: the definition, and for a restore the state it
/// had when it was dumped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterSpec {
    #[serde(flatten)]
    pub def: CounterDef,
    #[serde(default)]
    pub state: Option<CounterState>,
}

/// The numbers an insert took, until its batch is written: the counters'
/// lock, and the states to store with the documents.
pub(crate) struct Numbering {
    _lock: HeldCounterLock,
    states: Vec<(String, Vec<u8>)>,
    durable: bool,
}

impl Numbering {
    /// Put the counters' new states in the batch that stores the numbered
    /// documents: written together or not at all.
    pub(crate) fn stage(&self, batch: &mut KvBatch, cf: &KsCf) {
        for (key, state) in &self.states {
            batch.put_ks(cf, key.as_bytes(), state);
        }
    }

    /// Whether the batch must be synced before the insert is acknowledged.
    pub(crate) fn durable(&self) -> bool {
        self.durable
    }
}

fn not_a_counter(field: &str) -> DbError {
    DbError::DocumentNotFound(format!("No counter on '{}'", field))
}

impl Collection {
    fn bare_name(&self) -> &str {
        self.name.split_once(':').map_or("", |(_, c)| c)
    }

    /// The counters stored on the collection, read from storage; the
    /// keyspace remembers them for every handle.
    fn load_counters(&self) -> Arc<Vec<CounterDef>> {
        let defs: Vec<CounterDef> = self
            .ks
            .handle(&self.db)
            .and_then(|cf| self.db.get_ks(&cf, COUNTERS_KEY.as_bytes()).ok().flatten())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        self.ks.state().set_counters(defs);
        self.ks.state().counters().unwrap_or_default()
    }

    /// The counters declared on this collection.
    pub fn counter_defs(&self) -> Arc<Vec<CounterDef>> {
        match self.ks.state().counters() {
            Some(defs) => defs,
            None => self.load_counters(),
        }
    }

    /// The counters an insert through this handle must number: `None` when
    /// the collection has none (one atomic load), or for a write another
    /// node already numbered (`foreign_keys::unchecked`).
    pub(crate) fn counters(&self) -> Option<Arc<Vec<CounterDef>>> {
        if self.ks.state().has_no_counter() || foreign_keys::is_unchecked() {
            return None;
        }
        Some(self.counter_defs()).filter(|defs| !defs.is_empty())
    }

    fn read_counter_state(&self, cf: &KsCf, def: &CounterDef) -> DbResult<CounterState> {
        let stored = self
            .db
            .get_ks(cf, state_key(&def.field).as_bytes())
            .map_err(|e| DbError::InternalError(format!("Failed to read a counter: {}", e)))?;
        match stored {
            Some(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| DbError::InternalError(format!("Unreadable counter state: {}", e))),
            None => Ok(def.initial()),
        }
    }

    /// What is stored under the state key of `field`, which outlives the
    /// counter as the record of its drop.
    fn stored_counter_state(&self, cf: &KsCf, field: &str) -> Option<CounterState> {
        self.db
            .get_ks(cf, state_key(field).as_bytes())
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// The counters with where each stands.
    pub fn counter_records(&self) -> DbResult<Vec<CounterRecord>> {
        let cf = self.live_cf()?;
        self.counter_defs()
            .iter()
            .map(|def| {
                Ok(CounterRecord {
                    def: def.clone(),
                    state: self.read_counter_state(&cf, def)?,
                })
            })
            .collect()
    }

    /// The counter on `field` with its state.
    pub fn counter_record(&self, field: &str) -> DbResult<CounterRecord> {
        self.counter_records()?
            .into_iter()
            .find(|r| r.def.field == field)
            .ok_or_else(|| not_a_counter(field))
    }

    /// Before a new document is validated: refuse a counter field the
    /// client set, and put a value of the right shape in its place so that
    /// a schema that requires the field accepts the document. The number
    /// itself is given under the lock, by [`Self::number`].
    pub(crate) fn expect_numbers(&self, defs: &[CounterDef], data: &mut Value) -> DbResult<()> {
        self.refuse_given_numbers(defs, data)?;
        let Some(object) = data.as_object_mut() else {
            return Ok(());
        };
        for def in defs {
            let like = def.peek(&def.initial(), counters::now())?;
            object.insert(def.field.clone(), like);
        }
        Ok(())
    }

    /// Refuse a new document whose counter field the client set, and an
    /// insert on a node that does not number.
    pub(crate) fn refuse_given_numbers(&self, defs: &[CounterDef], data: &Value) -> DbResult<()> {
        for def in defs {
            if data.get(&def.field).is_some_and(|v| !v.is_null()) {
                return Err(DbError::BadRequest(format!(
                    "'{}' is a counter of '{}': the database assigns it, leave it out",
                    def.field,
                    self.bare_name()
                )));
            }
            counters::check_owner(&self.db, self.bare_name(), def)?;
        }
        Ok(())
    }

    /// Number the documents a transaction inserts into this collection, as
    /// its commit is prepared: `operations` are this collection's, in
    /// order. A key inserted and then deleted takes no number; the number
    /// of an inserted key is carried into the updates the transaction made
    /// to it afterwards. `None` when nothing is numbered; otherwise what is
    /// returned is staged in the commit's batch and held until it is
    /// written.
    pub(crate) fn number_operations(
        &self,
        operations: &mut [Operation],
    ) -> DbResult<Option<Numbering>> {
        if self.counters().is_none() {
            return Ok(None);
        }
        // Where each key's surviving insert is.
        let mut inserted: HashMap<String, usize> = HashMap::new();
        for (at, operation) in operations.iter().enumerate() {
            match operation {
                Operation::Insert { key, .. } => {
                    inserted.insert(key.clone(), at);
                }
                Operation::Delete { key, .. } => {
                    inserted.remove(key);
                }
                _ => {}
            }
        }
        if inserted.is_empty() {
            return Ok(None);
        }
        let numbered: HashSet<usize> = inserted.values().copied().collect();
        let numbering = self.number(
            &[],
            operations
                .iter_mut()
                .enumerate()
                .filter(|(at, _)| numbered.contains(at))
                .filter_map(|(_, operation)| match operation {
                    Operation::Insert { data, .. } => Some(data),
                    _ => None,
                }),
        )?;
        // What the transaction did to those documents afterwards was
        // staged without their numbers.
        let fields: Vec<String> = self
            .counter_defs()
            .iter()
            .map(|d| d.field.clone())
            .collect();
        let mut given: HashMap<String, Vec<(String, Value)>> = HashMap::new();
        for (at, operation) in operations.iter_mut().enumerate() {
            match operation {
                Operation::Insert { key, data, .. } if numbered.contains(&at) => {
                    let values = fields
                        .iter()
                        .filter_map(|f| Some((f.clone(), data.get(f)?.clone())))
                        .collect();
                    given.insert(key.clone(), values);
                }
                Operation::Update {
                    key,
                    old_data,
                    new_data,
                    ..
                } => {
                    if let Some(values) = given.get(key.as_str()) {
                        for document in [old_data, new_data] {
                            if let Some(object) = document.as_object_mut() {
                                for (field, value) in values {
                                    object.insert(field.clone(), value.clone());
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(Some(numbering))
    }

    /// Give each of `documents`, in order, its numbers. The caller holds
    /// the foreign-key gate if the collection has one and no key lock yet;
    /// what is returned holds the counters' lock and must live until the
    /// batch that stores the documents (and [`Numbering::stage`]) has been
    /// written.
    ///
    /// `expected` is what [`Self::expect_numbers`] was given: a counter
    /// dropped since then leaves no placeholder behind.
    pub(crate) fn number<'a>(
        &self,
        expected: &[CounterDef],
        documents: impl Iterator<Item = &'a mut Value>,
    ) -> DbResult<Numbering> {
        let lock = self.ks.state().hold_counter_lock();
        // Under the lock: a counter declared, dropped or moved meanwhile is
        // the one that counts.
        let defs = self.counter_defs();
        let cf = self.live_cf()?;
        let now = counters::now();
        let mut states = Vec::with_capacity(defs.len());
        for def in defs.iter() {
            counters::check_owner(&self.db, self.bare_name(), def)?;
            states.push(self.read_counter_state(&cf, def)?);
        }
        let gone: Vec<&CounterDef> = expected
            .iter()
            .filter(|e| !defs.iter().any(|d| d.field == e.field))
            .collect();
        for data in documents {
            let Some(object) = data.as_object_mut() else {
                continue;
            };
            for def in &gone {
                object.remove(&def.field);
            }
            for (def, state) in defs.iter().zip(states.iter_mut()) {
                object.insert(def.field.clone(), def.take(state, now)?);
            }
        }
        let mut staged = Vec::with_capacity(defs.len());
        for (def, state) in defs.iter().zip(&states) {
            staged.push((state_key(&def.field), serde_json::to_vec(state)?));
        }
        Ok(Numbering {
            _lock: lock,
            states: staged,
            durable: defs.iter().any(|d| d.durable),
        })
    }

    /// Write `batch`, with the states of `numbering` when the write is a
    /// numbered insert, synced when a counter asks for it.
    pub(crate) fn write_numbered(
        &self,
        mut batch: KvBatch,
        cf: &KsCf,
        numbering: Option<&Numbering>,
    ) -> Result<(), crate::storage::kv::KvError> {
        match numbering {
            None => self.db.write(batch),
            Some(numbering) => {
                numbering.stage(&mut batch, cf);
                if numbering.durable() {
                    self.db.write_opt(batch, &KvWriteOptions::synced())
                } else {
                    self.db.write(batch)
                }
            }
        }
    }

    /// An update may not change a counter: refuse `new` when it holds
    /// another value than `old`, and give it `old`'s when it holds none (a
    /// replace that leaves the field out).
    pub(crate) fn keep_numbers(&self, old: &Value, new: &mut Value) -> DbResult<()> {
        let Some(defs) = self.counters() else {
            return Ok(());
        };
        for def in defs.iter() {
            let before = old.get(&def.field).filter(|v| !v.is_null());
            match new.get(&def.field).filter(|v| !v.is_null()) {
                Some(after) if Some(after) != before => {
                    return Err(DbError::BadRequest(format!(
                        "'{}' is a counter of '{}': it cannot be changed",
                        def.field,
                        self.bare_name()
                    )));
                }
                Some(_) => {}
                None => {
                    if let (Some(before), Some(object)) = (before, new.as_object_mut()) {
                        object.insert(def.field.clone(), before.clone());
                    }
                }
            }
        }
        Ok(())
    }

    /// Refuse an update that leaves a counter other than it was. `old` and
    /// `new` need only hold what the indexes read: a counter's field is
    /// indexed.
    pub(crate) fn numbers_unchanged(&self, old: &Value, new: &Value) -> DbResult<()> {
        let Some(defs) = self.counters() else {
            return Ok(());
        };
        for def in defs.iter() {
            let set = |doc: &Value| doc.get(&def.field).filter(|v| !v.is_null()).cloned();
            if set(old) != set(new) {
                return Err(DbError::BadRequest(format!(
                    "'{}' is a counter of '{}': it cannot be changed",
                    def.field,
                    self.bare_name()
                )));
            }
        }
        Ok(())
    }

    /// Refuse a write that would create a document without numbering it:
    /// the paths that merge into whatever is stored (`upsert_batch`) do not
    /// number.
    pub(crate) fn refuse_unnumbered(&self, what: &str) -> DbResult<()> {
        match self.counters() {
            None => Ok(()),
            Some(defs) => Err(DbError::OperationNotSupported(format!(
                "'{}' has a counter ('{}'): {} cannot create a document in it, insert it instead",
                self.bare_name(),
                defs[0].field,
                what
            ))),
        }
    }

    fn store_counters(
        &self,
        defs: &[CounterDef],
        states: &[(&CounterDef, &CounterState)],
    ) -> DbResult<()> {
        let cf = self.live_cf()?;
        let mut batch = KvBatch::default();
        if defs.is_empty() {
            batch.delete_ks(&cf, COUNTERS_KEY.as_bytes());
        } else {
            batch.put_ks(&cf, COUNTERS_KEY.as_bytes(), serde_json::to_vec(defs)?);
        }
        for (def, state) in states {
            batch.put_ks(
                &cf,
                state_key(&def.field).as_bytes(),
                serde_json::to_vec(state)?,
            );
        }
        self.db
            .write_opt(batch, &KvWriteOptions::synced())
            .map_err(|e| DbError::InternalError(format!("Failed to store a counter: {}", e)))?;
        self.ks.state().set_counters(defs.to_vec());
        Ok(())
    }

    fn can_count(&self) -> DbResult<()> {
        let bare = self.bare_name();
        if bare.starts_with('_') {
            return Err(DbError::BadRequest(format!(
                "A system collection ('{}') cannot have a counter",
                bare
            )));
        }
        let kind = self.get_type();
        if kind != "document" {
            return Err(DbError::BadRequest(format!(
                "A counter is declared on a document collection; '{}' is a {} collection",
                bare, kind
            )));
        }
        if self.get_shard_config().map_or(0, |c| c.num_shards) > 0 {
            return Err(DbError::OperationNotSupported(format!(
                "Counters are not supported on sharded collections ('{}')",
                bare
            )));
        }
        Ok(())
    }

    /// The unique index a counter on `field` relies on: an existing unique
    /// one on the field alone, or one created here (`true`).
    fn counter_index(&self, field: &str) -> DbResult<(String, bool)> {
        if let Some(index) = self.get_single_field_index_for_field(field) {
            if index.unique && matches!(index.index_type, IndexType::Persistent | IndexType::Hash) {
                return Ok((index.name, false));
            }
            return Err(DbError::BadRequest(format!(
                "Index '{}' on '{}' cannot serve a counter: it must be a unique persistent or hash index. Drop it first",
                index.name, field
            )));
        }
        let name = format!("counter_{}", field);
        self.create_index_with(
            name.clone(),
            vec![field.to_string()],
            IndexType::Persistent,
            true,
            true,
        )?;
        Ok((name, true))
    }

    /// Two stored documents that hold the same value in `field`, which a
    /// unique index built over them would not have refused.
    fn repeated_number(&self, field: &str) -> DbResult<Option<Value>> {
        let fields = [field.to_string()];
        let mut seen: HashSet<String> = HashSet::new();
        let mut repeated = None;
        self.for_each_value(Some(&fields), |doc| {
            if repeated.is_some() {
                return Ok(());
            }
            if let Some(value) = doc.get(field).filter(|v| !v.is_null()) {
                if !seen.insert(value.to_string()) {
                    repeated = Some(value.clone());
                }
            }
            Ok(())
        })?;
        Ok(repeated)
    }

    /// Refuse a state whose next number a stored document already holds:
    /// every insert would then be refused by the unique index.
    fn check_next_is_free(&self, def: &CounterDef, state: &CounterState) -> DbResult<()> {
        let next = def.peek(state, counters::now())?;
        let taken = self
            .index_lookup_eq(&def.field, &next)
            .is_some_and(|found| !found.is_empty());
        if taken {
            return Err(DbError::ConflictError(format!(
                "A document of '{}' already holds {} in '{}': the counter's next number must be free (set a higher one)",
                self.bare_name(),
                next,
                def.field
            )));
        }
        Ok(())
    }

    /// Declare that the database numbers `spec.def.field` on insert.
    /// Creates a unique index on the field unless one is there. Documents
    /// already stored are left as they are; refused, and nothing is kept,
    /// when two of them hold the same value in the field or one holds the
    /// first number.
    pub fn add_counter(&self, spec: CounterSpec) -> DbResult<CounterRecord> {
        let mut def = spec.def;
        def.check()?;
        self.can_count()?;
        let local = counters::local(&self.db);
        if def.node.is_none() {
            def.node = local.node.clone();
        }
        if def.node.is_none() && local.peers {
            return Err(DbError::BadRequest(
                "A counter needs to know which node numbers: start this node with --node-id, or name one in 'node'"
                    .to_string(),
            ));
        }

        let _lock = self.ks.state().hold_counter_lock();
        let before = self.counter_defs().to_vec();
        if before.iter().any(|d| d.field == def.field) {
            return Err(DbError::ConflictError(format!(
                "'{}' already is a counter",
                def.field
            )));
        }
        if let Some(value) = self.repeated_number(&def.field)? {
            return Err(DbError::ConflictError(format!(
                "Two documents of '{}' hold {} in '{}': a counter's values are unique",
                self.bare_name(),
                value,
                def.field
            )));
        }
        let (index, created) = self.counter_index(&def.field)?;
        def.index = index;
        // Later than a counter dropped from the field before.
        let cf = self.live_cf()?;
        let before_epoch = self
            .stored_counter_state(&cf, &def.field)
            .map_or(0, |s| s.epoch);
        let mut state = spec.state.unwrap_or_else(|| def.initial());
        state.dropped = false;
        state.epoch = counters::next_epoch(state.epoch.max(before_epoch));
        let stored = self.check_next_is_free(&def, &state).and_then(|()| {
            let mut after = before.clone();
            after.push(def.clone());
            self.store_counters(&after, &[(&def, &state)])
        });
        if let Err(error) = stored {
            if created {
                let _ = self.drop_index(&def.index);
            }
            return Err(error);
        }
        Ok(CounterRecord { def, state })
    }

    /// Stop numbering `field`. The field of the stored documents, and its
    /// unique index, stay.
    ///
    /// Returns the epoch of the drop, which is what peers are told.
    pub fn drop_counter(&self, field: &str) -> DbResult<u64> {
        let _lock = self.ks.state().hold_counter_lock();
        let before = self.counter_defs().to_vec();
        let Some(def) = before.iter().find(|d| d.field == field).cloned() else {
            return Err(not_a_counter(field));
        };
        let cf = self.live_cf()?;
        let mut state = self.read_counter_state(&cf, &def)?;
        state.epoch = counters::next_epoch(state.epoch);
        state.dropped = true;
        let after: Vec<CounterDef> = before.into_iter().filter(|d| d.field != field).collect();
        self.store_counters(&after, &[(&def, &state)])?;
        Ok(state.epoch)
    }

    /// Change the counter on `field` under its lock: `change` gets the
    /// definition and the state, and what it leaves is stored.
    fn change_counter(
        &self,
        field: &str,
        change: impl FnOnce(&mut CounterDef, &mut CounterState) -> DbResult<()>,
    ) -> DbResult<CounterRecord> {
        let _lock = self.ks.state().hold_counter_lock();
        let mut defs = self.counter_defs().to_vec();
        let at = defs
            .iter()
            .position(|d| d.field == field)
            .ok_or_else(|| not_a_counter(field))?;
        let cf = self.live_cf()?;
        let mut state = self.read_counter_state(&cf, &defs[at])?;
        counters::check_owner(&self.db, self.bare_name(), &defs[at])?;
        change(&mut defs[at], &mut state)?;
        state.epoch = counters::next_epoch(state.epoch);
        let def = defs[at].clone();
        self.store_counters(&defs, &[(&def, &state)])?;
        Ok(CounterRecord { def, state })
    }

    /// Make `next` the number the next insert gets (in the current period,
    /// for a counter that restarts). Refused when a document already holds
    /// it. On the node that numbers.
    pub fn set_counter_next(&self, field: &str, next: u64) -> DbResult<CounterRecord> {
        if next == 0 {
            return Err(DbError::BadRequest(
                "A counter's next number is 1 or more".to_string(),
            ));
        }
        self.change_counter(field, |def, state| {
            state.next = next;
            self.check_next_is_free(def, state)
        })
    }

    /// Hand the numbering to `node` (or to whichever node receives the
    /// insert, with `None`, for an instance that runs alone). Done by the
    /// node that numbers, which stops numbering as it stores this: the
    /// record returned, replicated, tells `node` where to go on.
    pub fn move_counter(&self, field: &str, node: Option<String>) -> DbResult<CounterRecord> {
        self.change_counter(field, |def, _| {
            def.node = node;
            Ok(())
        })
    }

    /// Store a counter as another node declared, changed or handed it
    /// over. Its index is created if missing. Idempotent, and an older
    /// record than the one stored changes nothing.
    pub fn apply_counter(&self, record: &CounterRecord) -> DbResult<()> {
        let _lock = self.ks.state().hold_counter_lock();
        let mut defs = self.counter_defs().to_vec();
        let cf = self.live_cf()?;
        let mut def = record.def.clone();
        if let Some(ours) = self.stored_counter_state(&cf, &def.field) {
            if !counters::supersedes(&record.state, &ours) {
                return Ok(());
            }
        }
        match defs.iter().position(|d| d.field == def.field) {
            Some(at) => {
                def.index = defs[at].index.clone();
                defs[at] = def.clone();
            }
            None => {
                def.index = match self.get_single_field_index_for_field(&def.field) {
                    Some(index) => index.name,
                    None => {
                        let name = format!("counter_{}", def.field);
                        self.create_index_with(
                            name.clone(),
                            vec![def.field.clone()],
                            IndexType::Persistent,
                            true,
                            true,
                        )?;
                        name
                    }
                };
                defs.push(def.clone());
            }
        }
        self.store_counters(&defs, &[(&def, &record.state)])
    }

    /// Drop a counter another node dropped at `epoch`. Idempotent; a
    /// counter declared since then stays.
    pub fn apply_counter_drop(&self, field: &str, epoch: u64) -> DbResult<()> {
        let _lock = self.ks.state().hold_counter_lock();
        let cf = self.live_cf()?;
        let ours = self.stored_counter_state(&cf, field);
        if ours.as_ref().is_some_and(|s| s.epoch >= epoch) {
            return Ok(());
        }
        let before = self.counter_defs().to_vec();
        let def = before
            .iter()
            .find(|d| d.field == field)
            .cloned()
            .unwrap_or_else(|| CounterDef {
                field: field.to_string(),
                ..Self::no_counter()
            });
        let state = CounterState {
            epoch,
            dropped: true,
            ..ours.unwrap_or_default()
        };
        let after: Vec<CounterDef> = before.into_iter().filter(|d| d.field != field).collect();
        self.store_counters(&after, &[(&def, &state)])
    }

    fn no_counter() -> CounterDef {
        CounterDef {
            field: String::new(),
            start: 1,
            reset: None,
            timezone: "UTC".to_string(),
            prefix: String::new(),
            suffix: String::new(),
            padding: 0,
            durable: true,
            node: None,
            index: String::new(),
        }
    }

    /// The counter, if any, that `index` serves: it cannot be dropped
    /// while the counter is declared.
    pub(crate) fn counter_using_index(&self, index: &str) -> Option<String> {
        self.counter_defs()
            .iter()
            .find(|d| d.index == index)
            .map(|d| d.field.clone())
    }
}
