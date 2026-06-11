use dashmap::DashMap;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Stores query results for cursor-based pagination
#[derive(Clone)]
pub struct CursorStore {
    cursors: Arc<DashMap<String, StoredCursor>>,
    ttl: Duration,
}

struct StoredCursor {
    remaining_results: VecDeque<Value>,
    created_at: Instant,
    batch_size: usize,
    /// Database the originating query ran against; cursor continuation
    /// re-checks read permission on it.
    db_name: String,
}

/// Upper bound on concurrently stored cursors. Cursors hold full result
/// sets in memory; without a cap a client looping on cursor-producing
/// queries (and never draining them) grows memory until the TTL sweep.
const MAX_CURSORS: usize = 10_000;

impl CursorStore {
    /// Create a new cursor store with the specified TTL
    pub fn new(ttl: Duration) -> Self {
        Self {
            cursors: Arc::new(DashMap::new()),
            ttl,
        }
    }

    /// Keep the store under `MAX_CURSORS`: drop expired entries first, then
    /// the oldest live cursor if still full (it would be the first to expire
    /// anyway).
    fn make_room(&self) {
        if self.cursors.len() < MAX_CURSORS {
            return;
        }
        self.cursors
            .retain(|_, cursor| cursor.created_at.elapsed() <= self.ttl);
        while self.cursors.len() >= MAX_CURSORS {
            let oldest = self
                .cursors
                .iter()
                .max_by_key(|entry| entry.created_at.elapsed())
                .map(|entry| entry.key().clone());
            match oldest {
                Some(key) => {
                    self.cursors.remove(&key);
                    tracing::warn!("Cursor store full ({}), evicted oldest cursor", MAX_CURSORS);
                }
                None => break,
            }
        }
    }

    /// Store query results and return a cursor ID
    pub fn store(
        &self,
        db_name: impl Into<String>,
        results: Vec<Value>,
        batch_size: usize,
    ) -> String {
        self.make_room();
        let cursor_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let cursor = StoredCursor {
            remaining_results: VecDeque::from(results),
            created_at: Instant::now(),
            batch_size,
            db_name: db_name.into(),
        };

        self.cursors.insert(cursor_id.clone(), cursor);

        cursor_id
    }

    /// Store results and return the first batch in a single operation.
    /// Returns (cursor_id, first_batch, has_more).
    /// If all results fit in the first batch, no cursor is stored.
    pub fn store_and_get_first_batch(
        &self,
        db_name: impl Into<String>,
        results: Vec<Value>,
        batch_size: usize,
    ) -> (Option<String>, Vec<Value>, bool) {
        let mut iter = results.into_iter();
        let first_batch: Vec<Value> = iter.by_ref().take(batch_size).collect();
        let mut remaining_results: VecDeque<Value> = iter.collect();
        let has_more = !remaining_results.is_empty();

        if !has_more {
            // All results fit in one batch, no cursor needed
            return (None, first_batch, false);
        }

        self.make_room();
        let cursor_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let cursor = StoredCursor {
            remaining_results: std::mem::take(&mut remaining_results),
            created_at: Instant::now(),
            batch_size,
            db_name: db_name.into(),
        };

        self.cursors.insert(cursor_id.clone(), cursor);

        (Some(cursor_id), first_batch, true)
    }

    /// Database the cursor's query ran against (None if expired/unknown).
    pub fn db_name(&self, cursor_id: &str) -> Option<String> {
        self.cursors.get(cursor_id).map(|c| c.db_name.clone())
    }

    /// Get the next batch of results from a cursor
    pub fn get_next_batch(&self, cursor_id: &str) -> Option<(Vec<Value>, bool)> {
        // Try to get mutable access to the cursor
        let mut entry = self.cursors.get_mut(cursor_id)?;
        let cursor = entry.value_mut();

        // Check if cursor has expired
        if cursor.created_at.elapsed() > self.ttl {
            drop(entry);
            self.cursors.remove(cursor_id);
            return None;
        }

        if cursor.remaining_results.is_empty() {
            // No more results
            drop(entry);
            self.cursors.remove(cursor_id);
            return Some((vec![], false));
        }

        let take = cursor.batch_size.min(cursor.remaining_results.len());
        let batch: Vec<Value> = cursor.remaining_results.drain(0..take).collect();

        let has_more = !cursor.remaining_results.is_empty();

        if !has_more {
            // Remove cursor if no more results
            drop(entry);
            self.cursors.remove(cursor_id);
        }

        Some((batch, has_more))
    }

    /// Delete a cursor explicitly
    pub fn delete(&self, cursor_id: &str) -> bool {
        self.cursors.remove(cursor_id).is_some()
    }

    /// Spawn a background task that cleans up expired cursors every 30 seconds
    pub fn spawn_cleanup_task(&self) {
        let cursors = self.cursors.clone();
        let ttl = self.ttl;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                cursors.retain(|_, cursor| cursor.created_at.elapsed() <= ttl);
            }
        });
    }

    /// Get the total number of active cursors (for testing/debugging)
    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        self.cursors.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_store_and_retrieve() {
        let store = CursorStore::new(Duration::from_secs(300));
        let results = vec![json!({"id": 1}), json!({"id": 2}), json!({"id": 3})];

        let cursor_id = store.store("db1", results, 2);

        // First batch
        let (batch, has_more) = store.get_next_batch(&cursor_id).unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0], json!({"id": 1}));
        assert_eq!(batch[1], json!({"id": 2}));
        assert!(has_more);

        // Second batch
        let (batch, has_more) = store.get_next_batch(&cursor_id).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0], json!({"id": 3}));
        assert!(!has_more);
    }

    #[test]
    fn test_cursor_expiration() {
        let store = CursorStore::new(Duration::from_millis(100));
        let results = vec![json!({"id": 1})];

        let cursor_id = store.store("db1", results, 10);

        // Wait for expiration
        std::thread::sleep(Duration::from_millis(150));

        // Should return None (expired)
        assert!(store.get_next_batch(&cursor_id).is_none());
    }

    #[test]
    fn test_delete_cursor() {
        let store = CursorStore::new(Duration::from_secs(300));
        let results = vec![json!({"id": 1})];

        let cursor_id = store.store("db1", results, 10);

        // Delete cursor
        assert!(store.delete(&cursor_id));

        // Should return None (deleted)
        assert!(store.get_next_batch(&cursor_id).is_none());
    }

    #[test]
    fn test_small_result_set() {
        let store = CursorStore::new(Duration::from_secs(300));
        let results = vec![json!({"id": 1}), json!({"id": 2})];

        let cursor_id = store.store("db1", results, 10);

        // Single batch contains all results
        let (batch, has_more) = store.get_next_batch(&cursor_id).unwrap();
        assert_eq!(batch.len(), 2);
        assert!(!has_more);
    }

    #[test]
    fn test_store_and_get_first_batch() {
        let store = CursorStore::new(Duration::from_secs(300));
        let results = vec![json!({"id": 1}), json!({"id": 2}), json!({"id": 3})];

        let (cursor_id, first_batch, has_more) = store.store_and_get_first_batch("db1", results, 2);

        assert!(has_more);
        assert!(cursor_id.is_some());
        assert_eq!(first_batch.len(), 2);
        assert_eq!(first_batch[0], json!({"id": 1}));
        assert_eq!(first_batch[1], json!({"id": 2}));

        // Fetch remaining batch
        let (batch, has_more) = store.get_next_batch(cursor_id.as_ref().unwrap()).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0], json!({"id": 3}));
        assert!(!has_more);
    }

    #[test]
    fn test_store_and_get_first_batch_fits_in_one() {
        let store = CursorStore::new(Duration::from_secs(300));
        let results = vec![json!({"id": 1}), json!({"id": 2})];

        let (cursor_id, first_batch, has_more) =
            store.store_and_get_first_batch("db1", results, 10);

        assert!(!has_more);
        assert!(cursor_id.is_none());
        assert_eq!(first_batch.len(), 2);
        // No cursor stored
        assert_eq!(store.count(), 0);
    }
}
