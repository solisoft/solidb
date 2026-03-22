use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Default chunk size: 1MB
const DEFAULT_CHUNK_SIZE: u32 = 1024 * 1024;

/// Stores in-progress resumable blob upload sessions
#[derive(Clone)]
pub struct UploadSessionStore {
    sessions: Arc<DashMap<String, UploadSession>>,
    ttl: Duration,
}

pub struct UploadSession {
    pub upload_id: String,
    pub db_name: String,
    pub collection_name: String,
    pub blob_key: String,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub total_size: u64,
    pub chunk_size: u32,
    pub total_chunks: u32,
    pub received_chunks: Vec<bool>,
    pub bytes_received: u64,
    pub created_at: Instant,
    pub last_activity: Instant,
}

impl UploadSessionStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            ttl,
        }
    }

    /// Create a new upload session and return its ID
    pub fn create(
        &self,
        db_name: String,
        collection_name: String,
        file_name: Option<String>,
        mime_type: Option<String>,
        total_size: u64,
        chunk_size: Option<u32>,
    ) -> UploadSessionInfo {
        let upload_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let blob_key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let chunk_size = chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE);
        let total_chunks = if total_size == 0 {
            0
        } else {
            total_size.div_ceil(chunk_size as u64) as u32
        };

        let session = UploadSession {
            upload_id: upload_id.clone(),
            db_name,
            collection_name,
            blob_key: blob_key.clone(),
            file_name,
            mime_type,
            total_size,
            chunk_size,
            total_chunks,
            received_chunks: vec![false; total_chunks as usize],
            bytes_received: 0,
            created_at: Instant::now(),
            last_activity: Instant::now(),
        };

        self.sessions.insert(upload_id.clone(), session);

        UploadSessionInfo {
            upload_id,
            blob_key,
            chunk_size,
            total_chunks,
        }
    }

    /// Get a reference to a session for reading
    pub fn get(
        &self,
        upload_id: &str,
    ) -> Option<dashmap::mapref::one::Ref<'_, String, UploadSession>> {
        let entry = self.sessions.get(upload_id)?;
        if entry.created_at.elapsed() > self.ttl {
            drop(entry);
            self.sessions.remove(upload_id);
            return None;
        }
        Some(entry)
    }

    /// Get a mutable reference to a session
    pub fn get_mut(
        &self,
        upload_id: &str,
    ) -> Option<dashmap::mapref::one::RefMut<'_, String, UploadSession>> {
        let entry = self.sessions.get_mut(upload_id)?;
        if entry.created_at.elapsed() > self.ttl {
            drop(entry);
            self.sessions.remove(upload_id);
            return None;
        }
        Some(entry)
    }

    /// Remove a session
    pub fn remove(&self, upload_id: &str) -> Option<UploadSession> {
        self.sessions.remove(upload_id).map(|(_, s)| s)
    }

    /// Spawn a background cleanup task that evicts expired sessions.
    /// Returns the list of (db_name, collection_name, upload_id) for expired sessions
    /// so the caller can clean up temp chunks.
    pub fn spawn_cleanup_task<F>(&self, on_expire: F)
    where
        F: Fn(&str, &str, &str) + Send + Sync + 'static,
    {
        let sessions = self.sessions.clone();
        let ttl = self.ttl;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                let mut expired = Vec::new();
                sessions.retain(|_, session| {
                    if session.last_activity.elapsed() > ttl {
                        expired.push((
                            session.db_name.clone(),
                            session.collection_name.clone(),
                            session.upload_id.clone(),
                        ));
                        false
                    } else {
                        true
                    }
                });
                for (db, coll, uid) in &expired {
                    on_expire(db, coll, uid);
                }
            }
        });
    }
}

/// Info returned when creating a new upload session
pub struct UploadSessionInfo {
    pub upload_id: String,
    pub blob_key: String,
    pub chunk_size: u32,
    pub total_chunks: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_session() {
        let store = UploadSessionStore::new(Duration::from_secs(300));
        let info = store.create(
            "testdb".into(),
            "blobs".into(),
            Some("test.bin".into()),
            Some("application/octet-stream".into()),
            1024 * 1024 * 5, // 5MB
            None,
        );

        assert_eq!(info.chunk_size, DEFAULT_CHUNK_SIZE);
        assert_eq!(info.total_chunks, 5);
        assert!(store.get(&info.upload_id).is_some());
    }

    #[test]
    fn test_session_expiration() {
        let store = UploadSessionStore::new(Duration::from_millis(50));
        let info = store.create("db".into(), "col".into(), None, None, 1024, None);

        std::thread::sleep(Duration::from_millis(100));

        assert!(store.get(&info.upload_id).is_none());
    }

    #[test]
    fn test_remove_session() {
        let store = UploadSessionStore::new(Duration::from_secs(300));
        let info = store.create("db".into(), "col".into(), None, None, 1024, None);

        assert!(store.remove(&info.upload_id).is_some());
        assert!(store.get(&info.upload_id).is_none());
    }

    #[test]
    fn test_chunk_count_calculation() {
        let store = UploadSessionStore::new(Duration::from_secs(300));

        // Exact multiple
        let info = store.create("db".into(), "c".into(), None, None, 3 * 1024 * 1024, None);
        assert_eq!(info.total_chunks, 3);

        // Not exact - rounds up
        let info = store.create(
            "db".into(),
            "c".into(),
            None,
            None,
            3 * 1024 * 1024 + 1,
            None,
        );
        assert_eq!(info.total_chunks, 4);

        // Zero size
        let info = store.create("db".into(), "c".into(), None, None, 0, None);
        assert_eq!(info.total_chunks, 0);
    }
}
