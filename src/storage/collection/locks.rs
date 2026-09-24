//! Per-key write serialisation (audit D4).
//!
//! Every single-document write is read → compute index diff → write. Without
//! a lock between the read and the write, two concurrent updates A→B and A→C
//! each delete `idx(A)` and one of `idx(B)`/`idx(C)` is left behind forever,
//! and two inserts carrying the same unique value both pass the check.
//!
//! Locks are striped per collection and shared by every `Collection` handle
//! for the same column family (the engine and `Database` caches build
//! independent handles, so a field on `Collection` would not serialise them).
//!
//! Two stripe families with a fixed acquisition order prevent deadlock:
//! document-key stripes are always taken before unique-value stripes, and
//! each family is taken in ascending stripe order.

use dashmap::DashMap;
use once_cell::sync::Lazy;
use parking_lot::{Mutex, MutexGuard};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::Collection;

const STRIPES: usize = 256;

pub(crate) struct StripeSet {
    keys: [Mutex<()>; STRIPES],
    uniques: [Mutex<()>; STRIPES],
}

impl StripeSet {
    fn new() -> Self {
        Self {
            keys: std::array::from_fn(|_| Mutex::new(())),
            uniques: std::array::from_fn(|_| Mutex::new(())),
        }
    }
}

/// One leaked `StripeSet` per (DB instance, column family). Leaked on purpose:
/// guards borrow `'static`, and a set is ~512 bytes, bounded by the number of
/// distinct collections a process ever writes to. A DB address reused by a
/// later engine simply shares the set, which only adds contention.
static STRIPE_SETS: Lazy<DashMap<u64, &'static StripeSet>> = Lazy::new(DashMap::new);

/// Held guards; dropping it releases every stripe.
pub(crate) struct WriteGuard {
    _guards: Vec<MutexGuard<'static, ()>>,
}

fn hash_of<T: Hash + ?Sized>(seed: u64, value: &T) -> u64 {
    let mut h = DefaultHasher::new();
    seed.hash(&mut h);
    value.hash(&mut h);
    h.finish()
}

fn lock_sorted(
    family: &'static [Mutex<()>; STRIPES],
    mut idx: Vec<usize>,
) -> Vec<MutexGuard<'static, ()>> {
    idx.sort_unstable();
    idx.dedup();
    idx.into_iter().map(|i| family[i].lock()).collect()
}

impl Collection {
    fn stripe_set(&self) -> &'static StripeSet {
        let id = hash_of(
            std::sync::Arc::as_ptr(&self.db) as usize as u64,
            self.name.as_str(),
        );
        if let Some(set) = STRIPE_SETS.get(&id) {
            return *set;
        }
        *STRIPE_SETS
            .entry(id)
            .or_insert_with(|| Box::leak(Box::new(StripeSet::new())))
    }

    /// Lock the stripes covering `keys`. Must be taken before any
    /// `lock_unique_tokens` in the same operation, and never while already
    /// holding a guard from this collection.
    pub(crate) fn lock_keys<'a, I>(&self, keys: I) -> WriteGuard
    where
        I: IntoIterator<Item = &'a str>,
    {
        let set = self.stripe_set();
        let idx = keys
            .into_iter()
            .map(|k| (hash_of(0, k) as usize) % STRIPES)
            .collect();
        WriteGuard {
            _guards: lock_sorted(&set.keys, idx),
        }
    }

    /// Lock the stripes covering unique-index values (tokens built by
    /// `unique_tokens`). Taken after the key stripes, so a holder of a unique
    /// stripe never waits on a key stripe.
    pub(crate) fn lock_unique_tokens<'a, I>(&self, tokens: I) -> WriteGuard
    where
        I: IntoIterator<Item = &'a String>,
    {
        let set = self.stripe_set();
        let idx: Vec<usize> = tokens
            .into_iter()
            .map(|t| (hash_of(1, t.as_str()) as usize) % STRIPES)
            .collect();
        if idx.is_empty() {
            return WriteGuard {
                _guards: Vec::new(),
            };
        }
        WriteGuard {
            _guards: lock_sorted(&set.uniques, idx),
        }
    }
}
