//! Per-key write serialisation (audit D4).
//!
//! Every single-document write is read → compute index diff → write. Without
//! a lock between the read and the write, two concurrent updates A→B and A→C
//! each delete `idx(A)` and one of `idx(B)`/`idx(C)` is left behind forever,
//! and two inserts carrying the same unique value both pass the check.
//!
//! Locks are kept per collection and shared by every `Collection` handle for
//! the same column family (the engine and `Database` caches build independent
//! handles, so a field on `Collection` would not serialise them).
//!
//! Document keys are locked one by one: a key is held while it sits in a
//! table of held keys, so two writes wait for each other only when they share
//! a key. They used to be 256 striped mutexes, and a batch of 1,000 keys took
//! nearly all of them, so concurrent bulk loads into one collection ran one
//! at a time. A key is identified by a 64-bit hash of it; two keys with the
//! same hash share a lock, which can only make one write wait for the other.
//!
//! Unique-index values are still striped: they are taken only when the
//! collection has a unique index.
//!
//! A fixed acquisition order prevents deadlock: document keys are always
//! taken before unique-value stripes, keys in ascending hash order and
//! stripes in ascending stripe order.

use dashmap::DashMap;
use once_cell::sync::Lazy;
use parking_lot::{Condvar, Mutex, MutexGuard};
use std::collections::HashSet;
use std::hash::{BuildHasherDefault, Hasher};

use super::Collection;

const STRIPES: usize = 256;

/// Shards of the held-key table. A shard's mutex is held only while keys are
/// added to or removed from it, never while a write runs.
const KEY_SHARDS: usize = 64;

/// The held-key sets store hashes that are already well mixed: hashing them
/// again would only cost time.
#[derive(Default)]
struct PassThrough(u64);

impl Hasher for PassThrough {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0 << 8) | u64::from(*b);
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

type HeldKeys = HashSet<u64, BuildHasherDefault<PassThrough>>;

struct KeyShard {
    held: Mutex<HeldKeys>,
    released: Condvar,
}

pub(crate) struct StripeSet {
    keys: [KeyShard; KEY_SHARDS],
    uniques: [Mutex<()>; STRIPES],
}

impl StripeSet {
    fn new() -> Self {
        Self {
            keys: std::array::from_fn(|_| KeyShard {
                held: Mutex::new(HeldKeys::default()),
                released: Condvar::new(),
            }),
            uniques: std::array::from_fn(|_| Mutex::new(())),
        }
    }
}

/// One leaked `StripeSet` per (DB instance, column family). Leaked on purpose:
/// guards borrow `'static`, and a set is a few KB, bounded by the number of
/// distinct collections a process ever writes to. A DB address reused by a
/// later engine simply shares the set, which only adds contention.
static STRIPE_SETS: Lazy<DashMap<u64, &'static StripeSet>> = Lazy::new(DashMap::new);

/// Document keys held by one operation; released when dropped.
struct HeldKeyGuard {
    set: &'static StripeSet,
    /// Ascending, deduplicated.
    hashes: Vec<u64>,
}

impl Drop for HeldKeyGuard {
    fn drop(&mut self) {
        let mut i = 0;
        while i < self.hashes.len() {
            let shard_idx = shard_of(self.hashes[i]);
            let shard = &self.set.keys[shard_idx];
            let mut held = shard.held.lock();
            while i < self.hashes.len() && shard_of(self.hashes[i]) == shard_idx {
                held.remove(&self.hashes[i]);
                i += 1;
            }
            drop(held);
            shard.released.notify_all();
        }
    }
}

/// Held locks; dropping it releases them all.
pub(crate) struct WriteGuard {
    _keys: Option<HeldKeyGuard>,
    _guards: Vec<MutexGuard<'static, ()>>,
}

/// A 64-bit hash of `bytes`, one family per `seed`. SeaHash rather than the
/// standard SipHash: every write hashes its keys here, and nothing outside
/// chooses these inputs to collide on purpose to any effect but waiting.
fn hash_of(seed: u64, bytes: &[u8]) -> u64 {
    seahash::hash_seeded(
        bytes,
        seed,
        0x243f_6a88_85a3_08d3,
        0x1319_8a2e_0370_7344,
        0xa409_3822_299f_31d0,
    )
}

/// The shard of a key hash: its top bits, so ascending hash order visits the
/// shards in ascending order too.
#[inline]
fn shard_of(hash: u64) -> usize {
    (hash >> (64 - KEY_SHARDS.trailing_zeros())) as usize
}

fn lock_sorted(
    family: &'static [Mutex<()>; STRIPES],
    mut idx: Vec<usize>,
) -> Vec<MutexGuard<'static, ()>> {
    idx.sort_unstable();
    idx.dedup();
    idx.into_iter().map(|i| family[i].lock()).collect()
}

/// Take every key of `hashes` (ascending, deduplicated) in order, each as
/// soon as it is free. An operation only ever waits for a key greater than
/// every key it already holds, so two operations cannot wait on each other.
fn lock_key_hashes(set: &'static StripeSet, hashes: Vec<u64>) -> HeldKeyGuard {
    let mut i = 0;
    while i < hashes.len() {
        let shard_idx = shard_of(hashes[i]);
        let shard = &set.keys[shard_idx];
        let mut held = shard.held.lock();
        while i < hashes.len() && shard_of(hashes[i]) == shard_idx {
            let h = hashes[i];
            while held.contains(&h) {
                shard.released.wait(&mut held);
            }
            held.insert(h);
            i += 1;
        }
    }
    HeldKeyGuard { set, hashes }
}

impl Collection {
    fn stripe_set(&self) -> &'static StripeSet {
        let id = hash_of(
            std::sync::Arc::as_ptr(&self.db) as usize as u64,
            self.name.as_bytes(),
        );
        if let Some(set) = STRIPE_SETS.get(&id) {
            return *set;
        }
        *STRIPE_SETS
            .entry(id)
            .or_insert_with(|| Box::leak(Box::new(StripeSet::new())))
    }

    /// Lock the document `keys`. Must be taken before any
    /// `lock_unique_tokens` in the same operation, and never while already
    /// holding a guard from this collection.
    pub(crate) fn lock_keys<'a, I>(&self, keys: I) -> WriteGuard
    where
        I: IntoIterator<Item = &'a str>,
    {
        let set = self.stripe_set();
        let mut hashes: Vec<u64> = keys.into_iter().map(|k| hash_of(0, k.as_bytes())).collect();
        hashes.sort_unstable();
        hashes.dedup();
        WriteGuard {
            _keys: Some(lock_key_hashes(set, hashes)),
            _guards: Vec::new(),
        }
    }

    /// Lock the stripes covering unique-index values (tokens built by
    /// `unique_tokens`). Taken after the document keys, so a holder of a
    /// unique stripe never waits on a key.
    pub(crate) fn lock_unique_tokens<'a, I>(&self, tokens: I) -> WriteGuard
    where
        I: IntoIterator<Item = &'a String>,
    {
        let idx: Vec<usize> = tokens
            .into_iter()
            .map(|t| (hash_of(1, t.as_bytes()) as usize) % STRIPES)
            .collect();
        if idx.is_empty() {
            return WriteGuard {
                _keys: None,
                _guards: Vec::new(),
            };
        }
        let set = self.stripe_set();
        WriteGuard {
            _keys: None,
            _guards: lock_sorted(&set.uniques, idx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn leaked_set() -> &'static StripeSet {
        Box::leak(Box::new(StripeSet::new()))
    }

    fn spread(k: u64) -> u64 {
        k.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1
    }

    #[test]
    fn shards_follow_hash_order() {
        assert_eq!(shard_of(0), 0);
        assert_eq!(shard_of(u64::MAX), KEY_SHARDS - 1);
        let mut hashes: Vec<u64> = (0..1000).map(spread).collect();
        hashes.sort_unstable();
        let shards: Vec<usize> = hashes.iter().map(|h| shard_of(*h)).collect();
        assert!(shards.windows(2).all(|w| w[0] <= w[1]));
        assert!(shards.iter().any(|s| *s > 0) && shards.iter().any(|s| *s < KEY_SHARDS - 1));
    }

    /// A held key makes a second taker wait; a different key does not, even
    /// in the same shard; everything is released on drop.
    #[test]
    fn same_key_waits_other_key_does_not() {
        let set = leaked_set();
        let held = lock_key_hashes(set, vec![42]);

        let other = lock_key_hashes(set, vec![43]);
        drop(other);

        let acquired = Arc::new(AtomicUsize::new(0));
        let waiter = {
            let acquired = acquired.clone();
            std::thread::spawn(move || {
                let _g = lock_key_hashes(set, vec![7, 42]);
                acquired.store(1, Ordering::SeqCst);
            })
        };
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(acquired.load(Ordering::SeqCst), 0, "42 is still held");
        drop(held);
        waiter.join().unwrap();
        assert_eq!(acquired.load(Ordering::SeqCst), 1);
        for shard in &set.keys {
            assert!(shard.held.lock().is_empty());
        }
    }

    /// Overlapping multi-key operations neither deadlock nor ever hold one
    /// key twice at the same time.
    #[test]
    fn overlapping_batches_serialise_per_key() {
        let set = leaked_set();
        let in_use: Arc<Vec<AtomicUsize>> =
            Arc::new((0..64).map(|_| AtomicUsize::new(0)).collect());
        let threads: Vec<_> = (0..8u64)
            .map(|t| {
                let in_use = in_use.clone();
                std::thread::spawn(move || {
                    for round in 0..300u64 {
                        let mut keys: Vec<u64> =
                            (0..6).map(|j| (t * 7 + round * 3 + j * 11) % 64).collect();
                        keys.sort_unstable();
                        keys.dedup();
                        let mut hashes: Vec<u64> = keys.iter().map(|k| spread(*k)).collect();
                        hashes.sort_unstable();
                        let _g = lock_key_hashes(set, hashes);
                        for k in &keys {
                            assert_eq!(in_use[*k as usize].fetch_add(1, Ordering::SeqCst), 0);
                        }
                        std::thread::yield_now();
                        for k in &keys {
                            in_use[*k as usize].fetch_sub(1, Ordering::SeqCst);
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
    }
}
