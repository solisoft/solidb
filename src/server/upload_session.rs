use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Default chunk size: 1MB
const DEFAULT_CHUNK_SIZE: u32 = 1024 * 1024;

/// Maximum total upload size: 10 GiB
const MAX_TOTAL_SIZE: u64 = 10 * 1024 * 1024 * 1024;

/// Minimum chunk size: 64 KiB
const MIN_CHUNK_SIZE: u32 = 64 * 1024;

/// Maximum chunk size: 16 MiB
const MAX_CHUNK_SIZE: u32 = 16 * 1024 * 1024;

/// Maximum number of upload sessions held at once.
///
/// A session is cheap to create and expensive to hold: `received_chunks` is a
/// `Vec<bool>` sized to the declared chunk count, so a 10 GiB upload declared
/// with 64 KiB chunks reserves ~160 KB that lives for the full 24-hour TTL
/// even if not one byte is ever sent. Nothing capped the *number* of sessions
/// — unlike `cursor_store`, which is bounded at 10k with eviction — so
/// looping the create endpoint was a straightforward way to exhaust memory.
const MAX_SESSIONS: usize = 10_000;

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
    ) -> Result<UploadSessionInfo, crate::error::DbError> {
        if total_size > MAX_TOTAL_SIZE {
            return Err(crate::error::DbError::BadRequest(format!(
                "total_size {} exceeds maximum allowed size of {}",
                total_size, MAX_TOTAL_SIZE
            )));
        }

        let chunk_size = chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE);
        if chunk_size < MIN_CHUNK_SIZE {
            return Err(crate::error::DbError::BadRequest(format!(
                "chunk_size {} is below minimum allowed size of {}",
                chunk_size, MIN_CHUNK_SIZE
            )));
        }
        if chunk_size > MAX_CHUNK_SIZE {
            return Err(crate::error::DbError::BadRequest(format!(
                "chunk_size {} exceeds maximum allowed size of {}",
                chunk_size, MAX_CHUNK_SIZE
            )));
        }

        let upload_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let blob_key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
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

        self.make_room()?;
        self.sessions.insert(upload_id.clone(), session);

        Ok(UploadSessionInfo {
            upload_id,
            blob_key,
            chunk_size,
            total_chunks,
        })
    }

    /// Keep the store under [`MAX_SESSIONS`] before admitting a new session.
    ///
    /// Sweeps expired sessions first — under normal use that is always
    /// enough, because the ceiling is far above any real concurrent-upload
    /// count. If a sweep does not free a slot, every session is live and the
    /// request is refused rather than served by evicting somebody else's
    /// in-progress upload.
    fn make_room(&self) -> Result<(), crate::error::DbError> {
        if self.sessions.len() < MAX_SESSIONS {
            return Ok(());
        }

        let ttl = self.ttl;
        self.sessions
            .retain(|_, session| session.created_at.elapsed() <= ttl);

        if self.sessions.len() >= MAX_SESSIONS {
            tracing::warn!(
                "Refusing upload session: {} live sessions is at the {} ceiling",
                self.sessions.len(),
                MAX_SESSIONS
            );
            return Err(crate::error::DbError::BadRequest(format!(
                "Too many upload sessions in progress (limit {}). Complete or \
                 abort an existing upload and retry.",
                MAX_SESSIONS
            )));
        }
        Ok(())
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
        let info = store
            .create(
                "testdb".into(),
                "blobs".into(),
                Some("test.bin".into()),
                Some("application/octet-stream".into()),
                1024 * 1024 * 5, // 5MB
                None,
            )
            .unwrap();

        assert_eq!(info.chunk_size, DEFAULT_CHUNK_SIZE);
        assert_eq!(info.total_chunks, 5);
        assert!(store.get(&info.upload_id).is_some());
    }

    #[test]
    fn test_session_expiration() {
        let store = UploadSessionStore::new(Duration::from_millis(50));
        let info = store
            .create("db".into(), "col".into(), None, None, 1024, None)
            .unwrap();

        std::thread::sleep(Duration::from_millis(100));

        assert!(store.get(&info.upload_id).is_none());
    }

    #[test]
    fn test_remove_session() {
        let store = UploadSessionStore::new(Duration::from_secs(300));
        let info = store
            .create("db".into(), "col".into(), None, None, 1024, None)
            .unwrap();

        assert!(store.remove(&info.upload_id).is_some());
        assert!(store.get(&info.upload_id).is_none());
    }

    #[test]
    fn test_chunk_count_calculation() {
        let store = UploadSessionStore::new(Duration::from_secs(300));

        // Exact multiple
        let info = store
            .create("db".into(), "c".into(), None, None, 3 * 1024 * 1024, None)
            .unwrap();
        assert_eq!(info.total_chunks, 3);

        // Not exact - rounds up
        let info = store
            .create(
                "db".into(),
                "c".into(),
                None,
                None,
                3 * 1024 * 1024 + 1,
                None,
            )
            .unwrap();
        assert_eq!(info.total_chunks, 4);

        // Zero size
        let info = store
            .create("db".into(), "c".into(), None, None, 0, None)
            .unwrap();
        assert_eq!(info.total_chunks, 0);
    }
}
