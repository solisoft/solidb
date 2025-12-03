use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use serde_json::Value;
use uuid::Uuid;

/// Stores query results for cursor-based pagination
#[derive(Clone)]
pub struct CursorStore {
    cursors: Arc<RwLock<HashMap<String, StoredCursor>>>,
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
            cursors: Arc::new(RwLock::new(HashMap::new())),
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

        let mut cursors = self.cursors.write().unwrap();
        cursors.insert(cursor_id.clone(), cursor);
        
        // Clean up expired cursors
        self.cleanup_expired(&mut cursors);
        
        cursor_id
    }

    /// Get the next batch of results from a cursor
    pub fn get_next_batch(&self, cursor_id: &str) -> Option<(Vec<Value>, bool)> {
        let mut cursors = self.cursors.write().unwrap();
        
        if let Some(cursor) = cursors.get_mut(cursor_id) {
            // Check if cursor has expired
            if cursor.created_at.elapsed() > self.ttl {
                cursors.remove(cursor_id);
                return None;
            }

            let start = cursor.position;
            let end = (start + cursor.batch_size).min(cursor.results.len());
            
            if start >= cursor.results.len() {
                // No more results
                cursors.remove(cursor_id);
                return Some((vec![], false));
            }

            let batch = cursor.results[start..end].to_vec();
            cursor.position = end;
            
            let has_more = end < cursor.results.len();
            
            // Remove cursor if no more results
            if !has_more {
                cursors.remove(cursor_id);
            }
            
            Some((batch, has_more))
        } else {
            None
        }
    }

    /// Delete a cursor explicitly
    pub fn delete(&self, cursor_id: &str) -> bool {
        let mut cursors = self.cursors.write().unwrap();
        cursors.remove(cursor_id).is_some()
    }

    /// Clean up expired cursors
    fn cleanup_expired(&self, cursors: &mut HashMap<String, StoredCursor>) {
        cursors.retain(|_, cursor| cursor.created_at.elapsed() <= self.ttl);
    }

    /// Get the total number of active cursors (for testing/debugging)
    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        let cursors = self.cursors.read().unwrap();
        cursors.len()
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
}
