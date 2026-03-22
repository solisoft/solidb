use super::*;
use crate::error::{DbError, DbResult};
use std::sync::atomic::Ordering;

impl Collection {
    // ==================== Blob Operations ====================

    /// Store a blob chunk
    pub fn put_blob_chunk(&self, key: &str, chunk_index: u32, data: &[u8]) -> DbResult<()> {
        if *self.collection_type.read().unwrap() != "blob" {
            return Err(DbError::OperationNotSupported(
                "Blob operations only supported on blob collections".to_string(),
            ));
        }

        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let chunk_key = Self::blo_chunk_key(key, chunk_index as usize);

        // ... existence check ...
        let exists = db.get_cf(cf, &chunk_key).ok().flatten().is_some();

        db.put_cf(cf, chunk_key, data)
            .map_err(|e| DbError::InternalError(format!("Failed to store blob chunk: {}", e)))?;

        if !exists {
            self.chunk_count.fetch_add(1, Ordering::Relaxed);
            self.count_dirty.store(true, Ordering::Relaxed);
        }

        Ok(())
    }

    /// Get a blob chunk
    pub fn get_blob_chunk(&self, key: &str, chunk_index: u32) -> DbResult<Option<Vec<u8>>> {
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .ok_or(DbError::CollectionNotFound(self.name.clone()))?;

        let chunk_key = Self::blo_chunk_key(key, chunk_index as usize);
        match db.get_cf(cf, chunk_key) {
            Ok(Some(data)) => Ok(Some(data)),
            Ok(None) => Ok(None),
            Err(e) => Err(DbError::InternalError(format!(
                "Failed to get blob chunk: {}",
                e
            ))),
        }
    }

    /// Delete all blob chunks for a document
    pub fn delete_blob_data(&self, key: &str) -> DbResult<()> {
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let prefix = format!("{}{}:", BLO_PREFIX, key);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());
        let mut batch = rocksdb::WriteBatch::default();
        let mut count = 0;

        for result in iter.flatten() {
            let (k, _) = result;
            if !k.starts_with(prefix.as_bytes()) {
                break;
            }
            batch.delete_cf(cf, k);
            count += 1;
        }

        if count > 0 {
            db.write(batch)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
            self.chunk_count.fetch_sub(count, Ordering::Relaxed);
            self.count_dirty.store(true, Ordering::Relaxed);
        }

        Ok(())
    }

    // ==================== Resumable Upload Operations ====================

    /// Store a temporary blob chunk for a resumable upload
    pub fn put_blob_chunk_tmp(
        &self,
        upload_id: &str,
        chunk_index: u32,
        data: &[u8],
    ) -> DbResult<()> {
        if *self.collection_type.read().unwrap() != "blob" {
            return Err(DbError::OperationNotSupported(
                "Blob operations only supported on blob collections".to_string(),
            ));
        }

        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let key = format!("{}{}:{}", BLO_TMP_PREFIX, upload_id, chunk_index);
        db.put_cf(cf, key.as_bytes(), data).map_err(|e| {
            DbError::InternalError(format!("Failed to store temp blob chunk: {}", e))
        })?;

        Ok(())
    }

    /// Finalize a resumable upload: copy temp chunks to permanent blob storage and delete temps.
    /// Uses a WriteBatch for atomicity.
    pub fn finalize_blob_upload(
        &self,
        upload_id: &str,
        blob_key: &str,
        total_chunks: u32,
    ) -> DbResult<()> {
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let mut batch = rocksdb::WriteBatch::default();

        for i in 0..total_chunks {
            let tmp_key = format!("{}{}:{}", BLO_TMP_PREFIX, upload_id, i);
            let data = db
                .get_cf(cf, tmp_key.as_bytes())
                .map_err(|e| DbError::InternalError(format!("Failed to read temp chunk: {}", e)))?
                .ok_or_else(|| {
                    DbError::InternalError(format!(
                        "Missing temp chunk {} for upload {}",
                        i, upload_id
                    ))
                })?;

            let perm_key = Self::blo_chunk_key(blob_key, i as usize);
            batch.put_cf(cf, &perm_key, &data);
            batch.delete_cf(cf, tmp_key.as_bytes());
        }

        db.write(batch).map_err(|e| {
            DbError::InternalError(format!("Failed to finalize blob upload: {}", e))
        })?;

        self.chunk_count
            .fetch_add(total_chunks as usize, Ordering::Relaxed);
        self.count_dirty.store(true, Ordering::Relaxed);

        Ok(())
    }

    /// Delete all temporary chunks for a given upload session (used by cleanup task)
    pub fn delete_upload_chunks(&self, upload_id: &str) -> DbResult<()> {
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let prefix = format!("{}{}:", BLO_TMP_PREFIX, upload_id);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());
        let mut batch = rocksdb::WriteBatch::default();
        let mut count = 0;

        for result in iter.flatten() {
            let (k, _) = result;
            if !k.starts_with(prefix.as_bytes()) {
                break;
            }
            batch.delete_cf(cf, k);
            count += 1;
        }

        if count > 0 {
            db.write(batch)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
        }

        Ok(())
    }

    /// Get blob statistics for this collection
    pub fn blob_stats(&self) -> DbResult<(usize, u64)> {
        if *self.collection_type.read().unwrap() != "blob" {
            return Ok((0, 0));
        }

        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .ok_or(DbError::CollectionNotFound(self.name.clone()))?;

        let prefix = BLO_PREFIX.as_bytes();
        let mut total_bytes = 0u64;
        let mut chunk_count = 0usize;

        let iter = db.prefix_iterator_cf(cf, prefix);
        for item in iter.flatten() {
            let (key, value) = item;
            if !key.starts_with(prefix) {
                break;
            }
            // Key format: "blo:{key}:{chunk_index}"
            // Only count the data entries (even indices), not metadata
            if key.len() > 4 {
                chunk_count += 1;
                total_bytes += value.len() as u64;
            }
        }

        Ok((chunk_count, total_bytes))
    }
}
