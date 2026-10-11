//! Counters kept per thread and summed at scrape time.
//!
//! A shared atomic sits on one cache line, so every core that increments it
//! waits on the others: time added to whatever is being counted. Here each
//! thread increments its own slot, registered once in a [`PerThread`], and a
//! scrape adds the slots up.
//!
//! A thread that exits folds its slot into the registry's `retired` totals
//! and unregisters, under the same lock a scrape takes, so the sum never goes
//! backwards and the registry holds one slot per *live* thread. Without that,
//! the blocking pool (which retires idle threads and spawns new ones) would
//! grow it for as long as the process runs.

use std::ops::Deref;
use std::sync::{Arc, Mutex, MutexGuard};

/// A per-thread slot: atomics a single thread writes and a scrape reads.
pub trait Slot: Send + Sync + 'static {
    /// Add this slot's values into `retired`. Called once, as the owning
    /// thread exits, with the registry locked.
    fn retire_into(&self, retired: &Self);
}

pub struct PerThread<T: Slot> {
    live: Mutex<Vec<Arc<T>>>,
    retired: T,
}

impl<T: Slot> PerThread<T> {
    /// `retired` is the zero value; it accumulates exited threads' counts.
    pub const fn new(retired: T) -> Self {
        Self {
            live: Mutex::new(Vec::new()),
            retired,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Arc<T>>> {
        self.live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Register `slot` for the calling thread. Keep the returned value in a
    /// `thread_local!`: dropping it retires the slot.
    pub fn register(&'static self, slot: T) -> Registration<T> {
        let slot = Arc::new(slot);
        self.lock().push(Arc::clone(&slot));
        Registration { owner: self, slot }
    }

    /// Call `visit` on the retired totals and on every live slot, with the
    /// registry locked so no thread retires halfway through the sum.
    pub fn for_each(&self, mut visit: impl FnMut(&T)) {
        let live = self.lock();
        visit(&self.retired);
        for slot in live.iter() {
            visit(slot);
        }
    }
}

pub struct Registration<T: Slot> {
    owner: &'static PerThread<T>,
    slot: Arc<T>,
}

impl<T: Slot> Deref for Registration<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.slot
    }
}

impl<T: Slot> Drop for Registration<T> {
    fn drop(&mut self) {
        let mut live = self.owner.lock();
        self.slot.retire_into(&self.owner.retired);
        live.retain(|slot| !Arc::ptr_eq(slot, &self.slot));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Count(AtomicU64);

    impl Slot for Count {
        fn retire_into(&self, retired: &Self) {
            retired
                .0
                .fetch_add(self.0.load(Ordering::Relaxed), Ordering::Relaxed);
        }
    }

    static COUNTS: PerThread<Count> = PerThread::new(Count(AtomicU64::new(0)));

    thread_local! {
        static MINE: Registration<Count> = COUNTS.register(Count(AtomicU64::new(0)));
    }

    fn total() -> u64 {
        let mut sum = 0;
        COUNTS.for_each(|c| sum += c.0.load(Ordering::Relaxed));
        sum
    }

    #[test]
    fn exited_threads_keep_their_counts_and_leave_the_registry() {
        let before = total();
        let live_before = COUNTS.lock().len();
        for _ in 0..8 {
            std::thread::spawn(|| MINE.with(|c| c.0.fetch_add(5, Ordering::Relaxed)))
                .join()
                .unwrap();
        }
        assert_eq!(total() - before, 40);
        assert_eq!(COUNTS.lock().len(), live_before);
    }
}
