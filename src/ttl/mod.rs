use crate::storage::StorageEngine;
use std::sync::Arc;
use std::time::Duration;

/// TTL Worker - background task that cleans up expired documents
/// based on TTL indexes defined on collections
pub struct TtlWorker {
    storage: Arc<StorageEngine>,
    interval_secs: u64,
}

impl TtlWorker {
    /// Create a new TTL worker with the specified storage engine
    pub fn new(storage: Arc<StorageEngine>) -> Self {
        Self {
            storage,
            interval_secs: 60, // Default: check every 60 seconds
        }
    }

    /// Run the TTL cleanup loop
    ///
    /// Nothing inside one sweep can end the loop: the sweep runs as its own
    /// blocking task, so a panic surfaces here as a `JoinError`, and each
    /// collection is additionally isolated with `catch_unwind` so one bad
    /// collection does not skip the rest of the instance. Before, an
    /// `expect` on the index-metadata path (e.g. a collection dropped
    /// mid-sweep) killed the worker for the life of the process and
    /// expired documents silently stopped being removed.
    pub async fn start(self: Arc<Self>) {
        tracing::info!("Starting TTL Worker (interval: {}s)", self.interval_secs);
        loop {
            tokio::time::sleep(Duration::from_secs(self.interval_secs)).await;
            self.cleanup_expired_documents().await;
        }
    }

    /// Cleanup expired documents across all databases and collections
    async fn cleanup_expired_documents(&self) {
        let storage = self.storage.clone();
        match tokio::task::spawn_blocking(move || Self::sweep(&storage)).await {
            Ok(total_deleted) => {
                if total_deleted > 0 {
                    tracing::debug!(
                        "TTL cleanup cycle complete: {} total documents deleted",
                        total_deleted
                    );
                }
            }
            Err(e) => {
                tracing::error!("TTL cleanup sweep panicked; retrying next interval: {}", e);
            }
        }
    }

    /// One pass over every collection. Synchronous: the whole pass is
    /// storage work, so it runs on one blocking thread rather than hopping
    /// on and off the runtime per collection.
    fn sweep(storage: &StorageEngine) -> usize {
        let mut total_deleted = 0;

        // One pass over the column families instead of one per database.
        // `Database::list_collections` calls `DB::cf_names`, which clones every
        // column-family name in the instance on each call, so driving it from
        // the database list cost `databases × total collections` string
        // allocations every interval before a single expiry was examined.
        for (db_name, coll_names) in storage.collections_grouped() {
            let db = match storage.get_database(&db_name) {
                Ok(db) => db,
                Err(_) => continue,
            };

            for coll_name in coll_names {
                let collection = match db.system_collection(&coll_name) {
                    Ok(coll) => coll,
                    Err(_) => continue,
                };

                // `cleanup_all_expired_documents` returns at once for a
                // collection with no TTL index (the index list is cached).
                let outcome = Self::guarded(|| collection.cleanup_all_expired_documents());
                match outcome {
                    Some(Ok(count)) => {
                        if count > 0 {
                            tracing::info!(
                                "TTL cleanup: deleted {} expired documents from {}.{}",
                                count,
                                db_name,
                                coll_name
                            );
                            total_deleted += count;
                        }
                    }
                    Some(Err(e)) => {
                        tracing::warn!("TTL cleanup failed for {}.{}: {}", db_name, coll_name, e);
                    }
                    None => {
                        tracing::error!(
                            "TTL cleanup panicked for {}.{}; skipping it this interval",
                            db_name,
                            coll_name
                        );
                    }
                }
            }
        }

        total_deleted
    }

    /// Run `f`, turning a panic into `None`.
    fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_ttl_worker_new() {
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());

        let worker = TtlWorker::new(storage);

        // Default interval is 60 seconds
        assert_eq!(worker.interval_secs, 60);
    }

    #[tokio::test]
    async fn test_ttl_cleanup_empty_database() {
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());

        let worker = TtlWorker::new(storage);

        // Should not panic on empty database
        worker.cleanup_expired_documents().await;
    }

    #[tokio::test]
    async fn test_ttl_cleanup_no_ttl_indexes() {
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());

        // Create a database and collection without TTL index
        storage.create_database("test_db".to_string()).unwrap();
        let db = storage.get_database("test_db").unwrap();
        db.create_collection("test_coll".to_string(), None).unwrap();

        let worker = TtlWorker::new(storage);

        // Should skip collections without TTL indexes
        worker.cleanup_expired_documents().await;
    }

    #[test]
    fn test_guarded_contains_panic() {
        assert_eq!(TtlWorker::guarded(|| 7), Some(7));
        let caught: Option<()> = TtlWorker::guarded(|| panic!("boom"));
        assert!(caught.is_none());
    }

    #[tokio::test]
    async fn test_sweep_survives_dropped_collection() {
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
        storage.create_database("ttl_db".to_string()).unwrap();
        let db = storage.get_database("ttl_db").unwrap();
        db.create_collection("c".to_string(), None).unwrap();
        db.delete_collection("c").unwrap();

        let worker = TtlWorker::new(storage);
        // Twice: a sweep that panicked or errored must not stop the next.
        worker.cleanup_expired_documents().await;
        worker.cleanup_expired_documents().await;
    }
}
