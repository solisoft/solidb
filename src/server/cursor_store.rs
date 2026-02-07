use dashmap::DashMap;
use serde_json::Value;
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
    results: Vec<Value>,
    position: usize,
    created_at: Instant,
    batch_size: usize,
}

impl CursorStore {
    /// Create a new cursor store with the specified TTL
    pub fn new(ttl: Duration) -> Self {
        Self {
            cursors: Arc::new(DashMap::new()),
            ttl,
        }
    }

    /// Store query results and return a cursor ID
    pub fn store(&self, results: Vec<Value>, batch_size: usize) -> String {
        let cursor_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let cursor = StoredCursor {
            results,
            position: 0,
            created_at: Instant::now(),
            batch_size,
        };

        self.cursors.insert(cursor_id.clone(), cursor);

        cursor_id
    }

    /// Store results and return the first batch in a single operation.
    /// Returns (cursor_id, first_batch, has_more).
    /// If all results fit in the first batch, no cursor is stored.
    pub fn store_and_get_first_batch(
        &self,
        results: Vec<Value>,
        batch_size: usize,
    ) -> (Option<String>, Vec<Value>, bool) {
        let total = results.len();
        let end = batch_size.min(total);
        let has_more = end < total;

        if !has_more {
            // All results fit in one batch, no cursor needed
            return (None, results, false);
        }

        let cursor_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let first_batch = results[..end].to_vec();

        let cursor = StoredCursor {
            results,
            position: end,
            created_at: Instant::now(),
            batch_size,
        };

        self.cursors.insert(cursor_id.clone(), cursor);

        (Some(cursor_id), first_batch, true)
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

        let start = cursor.position;
        let end = (start + cursor.batch_size).min(cursor.results.len());

        if start >= cursor.results.len() {
            // No more results
            drop(entry);
            self.cursors.remove(cursor_id);
            return Some((vec![], false));
        }

        let batch = cursor.results[start..end].to_vec();
        cursor.position = end;

        let has_more = end < cursor.results.len();

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

        let cursor_id = store.store(results, 2);

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

        let cursor_id = store.store(results, 10);

        // Wait for expiration
        std::thread::sleep(Duration::from_millis(150));

        // Should return None (expired)
        assert!(store.get_next_batch(&cursor_id).is_none());
    }

    #[test]
    fn test_delete_cursor() {
        let store = CursorStore::new(Duration::from_secs(300));
        let results = vec![json!({"id": 1})];

        let cursor_id = store.store(results, 10);

        // Delete cursor
        assert!(store.delete(&cursor_id));

        // Should return None (deleted)
        assert!(store.get_next_batch(&cursor_id).is_none());
    }

    #[test]
    fn test_small_result_set() {
        let store = CursorStore::new(Duration::from_secs(300));
        let results = vec![json!({"id": 1}), json!({"id": 2})];

        let cursor_id = store.store(results, 10);

        // Single batch contains all results
        let (batch, has_more) = store.get_next_batch(&cursor_id).unwrap();
        assert_eq!(batch.len(), 2);
        assert!(!has_more);
    }

    #[test]
    fn test_store_and_get_first_batch() {
        let store = CursorStore::new(Duration::from_secs(300));
        let results = vec![json!({"id": 1}), json!({"id": 2}), json!({"id": 3})];

        let (cursor_id, first_batch, has_more) = store.store_and_get_first_batch(results, 2);

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

        let (cursor_id, first_batch, has_more) = store.store_and_get_first_batch(results, 10);

        assert!(!has_more);
        assert!(cursor_id.is_none());
        assert_eq!(first_batch.len(), 2);
        // No cursor stored
        assert_eq!(store.count(), 0);
    }
}
