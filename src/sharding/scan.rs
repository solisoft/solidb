//! Cursor-paged document scans for bulk shard work (audit P8).
//!
//! Resharding, repair and full sync used to call `Collection::all()`, which
//! materialises a whole shard as a `Vec<Document>` — then copied it again into
//! a move list — on a tokio worker. These helpers walk the `doc:` prefix in
//! fixed-size pages instead, resuming strictly after the last raw key, so
//! memory is bounded by one page and each page is read on the blocking pool.
//!
//! Deleting documents that were already returned (the source side of a move)
//! is safe between pages: the cursor is a key, not an offset.

use crate::storage::collection::{Collection, Document, DOC_PREFIX};
use crate::storage::serializer::deserialize_doc;
use rust_rocksdb::{Direction, IteratorMode, ReadOptions};

/// Documents per page for bulk shard work.
pub const DEFAULT_PAGE_SIZE: usize = 1000;

/// Where the next page starts: after this raw RocksDB key, or at the start of
/// the `doc:` prefix when empty.
#[derive(Clone, Debug, Default)]
pub struct ScanCursor(Option<Vec<u8>>);

impl ScanCursor {
    pub fn start() -> Self {
        Self(None)
    }
}

/// Read up to `limit` documents after `cursor`.
///
/// Returns the page and the cursor for the next one, or `None` once the
/// prefix is exhausted. Undecodable entries are skipped but still count
/// toward `limit`, so a page's cost stays bounded.
pub fn scan_page(
    coll: &Collection,
    cursor: &ScanCursor,
    limit: usize,
) -> (Vec<Document>, Option<ScanCursor>) {
    let limit = limit.max(1);
    let db = &coll.db;
    let cf = match db.cf_handle(&coll.name) {
        Some(cf) => cf,
        // Dropped concurrently: nothing left to scan.
        None => return (Vec::new(), None),
    };
    let prefix = DOC_PREFIX.as_bytes();
    let start: Vec<u8> = match &cursor.0 {
        // The smallest key strictly greater than `last`.
        Some(last) => {
            let mut k = last.clone();
            k.push(0);
            k
        }
        None => prefix.to_vec(),
    };

    let mut opts = ReadOptions::default();
    opts.set_readahead_size(256 * 1024);
    let iter = db.iterator_cf_opt(
        &cf,
        opts,
        IteratorMode::From(start.as_slice(), Direction::Forward),
    );

    let mut docs = Vec::with_capacity(limit.min(4096));
    let mut visited = 0usize;
    for item in iter {
        let (key, value) = match item {
            Ok(kv) => kv,
            Err(e) => {
                tracing::warn!("scan_page: iterator error on {}: {}", coll.name, e);
                return (docs, None);
            }
        };
        if !key.starts_with(prefix) {
            return (docs, None);
        }
        visited += 1;
        if let Ok(doc) = deserialize_doc(&value) {
            docs.push(doc);
        }
        if visited >= limit {
            return (docs, Some(ScanCursor(Some(key.to_vec()))));
        }
    }
    (docs, None)
}

/// [`scan_page`] on the blocking pool, for async callers.
pub async fn scan_page_blocking(
    coll: Collection,
    cursor: ScanCursor,
    limit: usize,
) -> Result<(Vec<Document>, Option<ScanCursor>), String> {
    tokio::task::spawn_blocking(move || scan_page(&coll, &cursor, limit))
        .await
        .map_err(|e| format!("shard scan task failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageEngine;
    use tempfile::TempDir;

    #[test]
    fn pages_cover_every_document_exactly_once() {
        let tmp = TempDir::new().unwrap();
        let storage = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
        storage.create_database("d".to_string()).unwrap();
        let db = storage.get_database("d").unwrap();
        db.create_collection("c".to_string(), None).unwrap();
        let coll = db.get_collection("c").unwrap();
        for i in 0..2500 {
            coll.insert(serde_json::json!({"_key": format!("k{:05}", i), "i": i}))
                .unwrap();
        }

        let mut seen = std::collections::HashSet::new();
        let mut cursor = ScanCursor::start();
        let mut pages = 0;
        loop {
            let (docs, next) = scan_page(&coll, &cursor, 1000);
            pages += 1;
            for d in docs {
                assert!(seen.insert(d.key.clone()), "duplicate {}", d.key);
            }
            match next {
                Some(c) => cursor = c,
                None => break,
            }
        }
        assert_eq!(seen.len(), 2500);
        assert!((3..=4).contains(&pages), "pages = {}", pages);
    }

    #[test]
    fn deleting_returned_documents_does_not_skip_the_rest() {
        let tmp = TempDir::new().unwrap();
        let storage = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
        storage.create_database("d".to_string()).unwrap();
        let db = storage.get_database("d").unwrap();
        db.create_collection("c".to_string(), None).unwrap();
        let coll = db.get_collection("c").unwrap();
        for i in 0..250 {
            coll.insert(serde_json::json!({"_key": format!("k{:04}", i)}))
                .unwrap();
        }

        let mut total = 0;
        let mut cursor = ScanCursor::start();
        loop {
            let (docs, next) = scan_page(&coll, &cursor, 100);
            total += docs.len();
            let keys: Vec<String> = docs.into_iter().map(|d| d.key).collect();
            coll.delete_batch(keys).unwrap();
            match next {
                Some(c) => cursor = c,
                None => break,
            }
        }
        assert_eq!(total, 250);
        assert_eq!(coll.count(), 0);
    }
}
