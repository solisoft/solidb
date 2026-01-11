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
    pub async fn start(self: Arc<Self>) {
        tracing::info!("Starting TTL Worker (interval: {}s)", self.interval_secs);
        loop {
            tokio::time::sleep(Duration::from_secs(self.interval_secs)).await;
            self.cleanup_expired_documents().await;
        }
    }

    /// Cleanup expired documents across all databases and collections
    async fn cleanup_expired_documents(&self) {
        let databases = self.storage.list_databases();
        let mut total_deleted = 0;

        for db_name in databases {
            let db = match self.storage.get_database(&db_name) {
                Ok(db) => db,
                Err(_) => continue,
            };

            let collections = db.list_collections();
            for coll_name in collections {
                let collection = match db.get_collection(&coll_name) {
                    Ok(coll) => coll,
                    Err(_) => continue,
                };

                // Check if this collection has TTL indexes
                let ttl_indexes = collection.list_ttl_indexes();
                if ttl_indexes.is_empty() {
                    continue;
                }

                // Run cleanup (this is CPU-bound, so run in blocking task)
                let coll = collection.clone();
                match tokio::task::spawn_blocking(move || coll.cleanup_all_expired_documents())
                    .await
                {
                    Ok(Ok(count)) => {
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
                    Ok(Err(e)) => {
                        tracing::warn!("TTL cleanup failed for {}.{}: {}", db_name, coll_name, e);
                    }
                    Err(e) => {
                        tracing::error!("TTL cleanup task panicked: {}", e);
                    }
                }
            }
        }

        if total_deleted > 0 {
            tracing::debug!(
                "TTL cleanup cycle complete: {} total documents deleted",
                total_deleted
            );
        }
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
}
