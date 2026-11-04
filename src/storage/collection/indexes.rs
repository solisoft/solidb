use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::index::extract_field_value;
use crate::storage::kv::KvBatch;
use crate::storage::kv::{Direction, IterMode};
use crate::storage::serializer::deserialize_doc;
use serde_json::Value;

use hex;

type IndexEntry = (Vec<u8>, Vec<u8>);
type IndexEntries = Vec<IndexEntry>;
type IndexKeys = Vec<Vec<u8>>;

/// Every index family reports a duplicate as `DbError::InvalidDocument` with a
/// "... already exists" message, and there is no distinct variant to match on.
/// These two helpers keep that string dependency in one place; they are only
/// used to make index creation and dropping idempotent, so a false negative
/// degrades to a propagated error rather than silent corruption.
fn is_already_exists(e: &DbError) -> bool {
    matches!(e, DbError::InvalidDocument(msg) if msg.contains("already exists"))
}

fn is_not_found(e: &DbError) -> bool {
    matches!(e, DbError::InvalidDocument(msg) if msg.contains("not found"))
}

impl Collection {
    // ==================== Index Operations ====================

    /// Create an index from a serialised [`IndexSpec`].
    ///
    /// Strict: a duplicate name is an error, matching what a client issuing
    /// `POST .../index` expects. Replication and the shard fan-out want the
    /// opposite — use [`Collection::apply_index_spec`] there.
    pub fn create_index_from_spec(&self, spec: &crate::storage::index::IndexSpec) -> DbResult<()> {
        use crate::storage::index::IndexSpec;

        match spec {
            IndexSpec::Regular {
                name,
                fields,
                index_type,
                unique,
                sparse,
            } => self
                .create_index_with(
                    name.clone(),
                    fields.clone(),
                    index_type.clone(),
                    *unique,
                    *sparse,
                )
                .map(|_| ()),
            IndexSpec::Fulltext {
                name,
                fields,
                min_length,
                fold_accents,
            } => self
                .create_fulltext_index_with(
                    name.clone(),
                    fields.clone(),
                    *min_length,
                    *fold_accents,
                )
                .map(|_| ()),
            IndexSpec::Geo { name, field } => self
                .create_geo_index(name.clone(), field.clone())
                .map(|_| ()),
            IndexSpec::Ttl {
                name,
                field,
                expire_after_seconds,
            } => self
                .create_ttl_index(name.clone(), field.clone(), *expire_after_seconds)
                .map(|_| ()),
            IndexSpec::Vector(config) => self.create_vector_index(config.clone()).map(|_| ()),
        }
    }

    /// Idempotent [`Collection::create_index_from_spec`], for replication and
    /// the shard fan-out.
    ///
    /// Log entries are replayed at least once and the fan-out can overlap with
    /// an entry a node already applied, so an existing index is the expected
    /// outcome rather than a failure. Kept separate from the strict version so
    /// that tolerance never reaches a client's own create request.
    pub fn apply_index_spec(&self, spec: &crate::storage::index::IndexSpec) -> DbResult<()> {
        match self.create_index_from_spec(spec) {
            Ok(()) => Ok(()),
            Err(e) if is_already_exists(&e) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Drop an index, routing to the right family.
    ///
    /// Strict: a missing index is an error. Replication should use
    /// [`Collection::apply_index_drop`].
    pub fn drop_index_of_kind(
        &self,
        kind: crate::storage::index::IndexKind,
        name: &str,
    ) -> DbResult<()> {
        use crate::storage::index::IndexKind;

        match kind {
            // Fulltext indexes travel as `Regular` (`IndexSpec::kind`), but
            // live under their own `ft_meta:` key, not in `idx_meta`. Without
            // this fallback a replicated or shard-fanned fulltext drop found
            // nothing, was treated as already done, and left the index on
            // every peer.
            IndexKind::Regular => match self.drop_index(name) {
                Err(e) if is_not_found(&e) && self.get_fulltext_index(name).is_some() => {
                    self.drop_fulltext_index(name)
                }
                other => other,
            },
            IndexKind::Geo => self.drop_geo_index(name),
            IndexKind::Ttl => self.drop_ttl_index(name),
            IndexKind::Vector => self.drop_vector_index(name),
        }
    }

    /// Idempotent [`Collection::drop_index_of_kind`], for replication and the
    /// shard fan-out: a replayed drop may arrive after the index is gone.
    pub fn apply_index_drop(
        &self,
        kind: crate::storage::index::IndexKind,
        name: &str,
    ) -> DbResult<()> {
        match self.drop_index_of_kind(kind, name) {
            Ok(()) => Ok(()),
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Get all index metadata
    pub fn get_all_indexes(&self) -> Vec<Index> {
        // Empty when the column family is gone (dropped mid-operation): a
        // background caller such as the TTL worker must not panic (audit P11).
        self.index_meta()
            .map(|m| m.indexes.clone())
            .unwrap_or_default()
    }

    /// Get an index by name
    pub(crate) fn get_index(&self, name: &str) -> Option<Index> {
        self.index_meta()?
            .indexes
            .iter()
            .find(|i| i.name == name)
            .cloned()
    }

    /// Create a (sparse) index on a field
    pub fn create_index(
        &self,
        name: String,
        fields: Vec<String>,
        index_type: IndexType,
        unique: bool,
    ) -> DbResult<IndexStats> {
        self.create_index_with(name, fields, index_type, unique, true)
    }

    /// [`Self::create_index`] choosing whether the index is sparse (leaves
    /// out documents whose index fields are all null) — see [`Index::sparse`].
    pub fn create_index_with(
        &self,
        name: String,
        fields: Vec<String>,
        index_type: IndexType,
        unique: bool,
        sparse: bool,
    ) -> DbResult<IndexStats> {
        if index_type == IndexType::Trigram && (fields.len() != 1 || unique) {
            return Err(DbError::BadRequest(
                "A trigram index covers exactly one field and cannot be unique".to_string(),
            ));
        }
        // Check if index already exists
        if self.get_index(&name).is_some() {
            return Err(DbError::InvalidDocument(format!(
                "Index '{}' already exists",
                name
            )));
        }

        // Whatever the outcome below, from here on cached results may have
        // been computed against a half-built or half-removed index.
        let _invalidate = self.invalidate_cached_queries_on_exit();

        // Create index metadata
        let index = Index::new(name.clone(), fields.clone(), index_type.clone(), unique)
            .with_sparse(sparse);
        let index_bytes = serde_json::to_vec(&index)?;

        // Store index metadata and build index
        {
            let db = &self.db;
            let cf = self
                .ks
                .handle(&self.db)
                .expect("Column family should exist");
            db.put_ks(&cf, Self::idx_meta_key(&name), &index_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to create index: {}", e)))?;
        }
        self.invalidate_index_meta();

        // The metadata above is already visible to readers, so a backfill that
        // fails must not leave it behind: the index would be half-built,
        // `get_index` would report it as ready, and every later lookup would
        // silently return fewer documents than the collection holds.
        let indexed_count = match self.backfill_index(&index) {
            Ok(count) => count,
            Err(e) => {
                if let Err(cleanup) = self.drop_index(&name) {
                    tracing::error!(
                        collection = %self.name,
                        index = %name,
                        error = %cleanup,
                        "index backfill failed and the partial index could not be dropped; \
                         drop it by hand before querying the field"
                    );
                }
                return Err(e);
            }
        };

        // Save bloom/cuckoo filter if applicable
        if index_type == IndexType::Bloom {
            if let Some(filter) = self.bloom_filters.get(&name) {
                self.save_bloom_filter(&name, &filter)?;
            }
        } else if index_type == IndexType::Cuckoo {
            if let Some(filter) = self.cuckoo_filters.get(&name) {
                self.save_cuckoo_filter(&name, &filter)?;
            }
        }

        Ok(IndexStats {
            name,
            field: fields.first().cloned().unwrap_or_default(),
            fields,
            index_type,
            unique,
            unique_values: indexed_count, // Approximation
            indexed_documents: indexed_count,
            min_length: None,
            fold_accents: None,
            sparse: Some(sparse),
        })
    }

    /// Write the index entries for the documents already in the collection.
    ///
    /// Streams the column family rather than collecting `self.all()` first: a
    /// backfill on a large collection would otherwise hold every decoded
    /// document in memory at once, inside whatever request triggered it.
    ///
    /// Returns the number of documents given an entry: for a sparse index,
    /// those with a non-null value for its fields — 0 means no document
    /// carries them, so the index cannot match anything as it stands.
    fn backfill_index(&self, index: &Index) -> DbResult<usize> {
        let (name, fields, index_type) = (&index.name, &index.fields, &index.index_type);
        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;
        let prefix = DOC_PREFIX.as_bytes();

        // Use KvBatch for atomic, high-performance index creation
        let mut batch = KvBatch::default();
        let mut indexed_count = 0;
        const BATCH_SIZE_LIMIT: usize = 10000; // Flush every 10k entries to avoid excessive memory usage

        for item in db.prefix_iterator_ks(&cf, prefix) {
            let (key, value) = item.map_err(|e| {
                DbError::InternalError(format!("Failed to read documents for index: {}", e))
            })?;
            if !key.starts_with(prefix) {
                break;
            }
            let Ok(doc) = deserialize_doc(&value) else {
                continue;
            };

            let doc_value = doc.to_value();
            let field_values: Vec<Value> = fields
                .iter()
                .map(|f| extract_field_value(&doc_value, f))
                .collect();

            if !index.holds(&field_values) {
                continue;
            }

            for entry_key in Self::index_entry_keys(index, &field_values, &doc.key) {
                batch.put_ks(&cf, entry_key, doc.key.as_bytes());
            }
            indexed_count += 1;

            // If bloom/cuckoo filter, also update in-memory filter
            if *index_type == IndexType::Bloom {
                for value in &field_values {
                    self.bloom_insert(name, &value.to_string());
                }
            } else if *index_type == IndexType::Cuckoo {
                for value in &field_values {
                    self.cuckoo_insert(name, &value.to_string());
                }
            }

            // Flush batch periodically to avoid excessive memory usage
            if indexed_count % BATCH_SIZE_LIMIT == 0 {
                db.write(batch).map_err(|e| {
                    DbError::InternalError(format!("Failed to write index batch: {}", e))
                })?;
                batch = KvBatch::default();
            }
        }

        // Write any remaining entries in the batch
        if !batch.is_empty() {
            db.write(batch).map_err(|e| {
                DbError::InternalError(format!("Failed to write final index batch: {}", e))
            })?;
        }

        Ok(indexed_count)
    }

    /// Drop an index
    pub fn drop_index(&self, name: &str) -> DbResult<()> {
        if self.get_index(name).is_none() {
            return Err(DbError::InvalidDocument(format!(
                "Index '{}' not found",
                name
            )));
        }

        let _invalidate = self.invalidate_cached_queries_on_exit();

        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        // Use KvBatch for atomic, high-performance index deletion
        let mut batch = KvBatch::default();
        let mut deleted_count = 0;
        const BATCH_SIZE_LIMIT: usize = 10000; // Flush every 10k entries

        // Delete index metadata
        batch.delete_ks(&cf, Self::idx_meta_key(name));

        // Delete all index entries
        let prefix = format!("{}{}:", IDX_PREFIX, name);
        let iter = db.prefix_iterator_ks(&cf, prefix.as_bytes());

        for result in iter.flatten() {
            let (key, _) = result;
            if key.starts_with(prefix.as_bytes()) {
                batch.delete_ks(&cf, &key);
                deleted_count += 1;

                // Flush batch periodically to avoid excessive memory usage
                if deleted_count % BATCH_SIZE_LIMIT == 0 {
                    db.write(batch).map_err(|e| {
                        DbError::InternalError(format!("Failed to write drop batch: {}", e))
                    })?;
                    batch = KvBatch::default();
                }
            } else {
                break;
            }
        }

        // Write any remaining entries in the batch
        if deleted_count > 0 || !batch.is_empty() {
            db.write(batch).map_err(|e| {
                DbError::InternalError(format!("Failed to write final drop batch: {}", e))
            })?;
        }
        self.invalidate_index_meta();

        Ok(())
    }

    /// List all indexes
    pub fn list_indexes(&self) -> Vec<IndexStats> {
        let mut stats: Vec<IndexStats> = self
            .get_all_indexes()
            .iter()
            .filter_map(|idx| self.get_index_stats(&idx.name))
            .collect();

        // Include fulltext indexes
        for idx in self.get_all_fulltext_indexes() {
            stats.push(IndexStats {
                name: idx.name,
                fields: idx.fields.clone(),
                field: idx.fields.first().cloned().unwrap_or_default(),
                index_type: IndexType::Fulltext,
                unique: false,
                unique_values: 0,     // Not calculated for fulltext
                indexed_documents: 0, // Not calculated for fulltext
                min_length: Some(idx.min_length),
                fold_accents: Some(idx.fold_accents),
                sparse: None,
            });
        }

        stats
    }

    /// Rebuild all indexes from existing documents
    /// Call this after bulk imports using insert_no_index()
    pub fn rebuild_all_indexes(&self) -> DbResult<usize> {
        let total_start = std::time::Instant::now();
        // Entries are cleared and rewritten: results cached against them, and
        // completeness answers, must not outlive the rebuild.
        let _invalidate = self.invalidate_cached_queries_on_exit();

        let indexes = self.get_all_indexes();
        let geo_indexes = self.get_all_geo_indexes();
        let ft_indexes = self.get_all_fulltext_indexes();

        tracing::info!(
            "rebuild_all_indexes: {} regular, {} geo, {} fulltext indexes",
            indexes.len(),
            geo_indexes.len(),
            ft_indexes.len()
        );

        if indexes.is_empty() && geo_indexes.is_empty() && ft_indexes.is_empty() {
            return Ok(0);
        }

        // Clear existing index entries using KvBatch
        let clear_start = std::time::Instant::now();
        {
            let db = &self.db;
            let cf = self
                .ks
                .handle(&self.db)
                .expect("Column family should exist");
            let mut batch = KvBatch::default();

            // Clear regular indexes
            for index in &indexes {
                let prefix = format!("{}{}:", IDX_PREFIX, index.name);
                let iter = db.prefix_iterator_ks(&cf, prefix.as_bytes());
                for result in iter.flatten() {
                    let (key, _) = result;
                    if key.starts_with(prefix.as_bytes()) {
                        batch.delete_ks(&cf, &key);
                    } else {
                        break;
                    }
                }
            }

            // Clear geo indexes
            for geo_index in &geo_indexes {
                let prefix = format!("{}{}:", GEO_PREFIX, geo_index.name);
                let iter = db.prefix_iterator_ks(&cf, prefix.as_bytes());
                for result in iter.flatten() {
                    let (key, _) = result;
                    if key.starts_with(prefix.as_bytes()) {
                        batch.delete_ks(&cf, &key);
                    } else {
                        break;
                    }
                }
            }

            // Clear fulltext indexes. `ft:` n-gram entries are no longer
            // written (audit P5: nothing read them); clearing them here drops
            // what older versions left behind.
            for ft_index in &ft_indexes {
                let ngram_prefix = format!("{}{}:", FT_PREFIX, ft_index.name);
                let iter = db.prefix_iterator_ks(&cf, ngram_prefix.as_bytes());
                for result in iter.flatten() {
                    let (key, _) = result;
                    if key.starts_with(ngram_prefix.as_bytes()) {
                        batch.delete_ks(&cf, &key);
                    } else {
                        break;
                    }
                }

                let term_prefix = format!("{}{}:", FT_TERM_PREFIX, ft_index.name);
                let iter = db.prefix_iterator_ks(&cf, term_prefix.as_bytes());
                for result in iter.flatten() {
                    let (key, _) = result;
                    if key.starts_with(term_prefix.as_bytes()) {
                        batch.delete_ks(&cf, &key);
                    } else {
                        break;
                    }
                }
            }

            let _ = db.write(batch);
        }
        tracing::info!(
            "rebuild_all_indexes: Clear phase took {:?}",
            clear_start.elapsed()
        );

        // Stream documents from storage instead of loading all into memory
        // This allows handling collections with millions of documents without OOM
        let stream_start = std::time::Instant::now();
        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        let prefix = DOC_PREFIX.as_bytes();
        let iter = db.prefix_iterator_ks(&cf, prefix);

        // Build all index types in a single pass with periodic batch flushing
        // This reduces memory usage and I/O compared to loading all docs then multiple passes
        const BATCH_SIZE_LIMIT: usize = 10000; // Flush every 10k entries per index type
        let mut doc_count = 0;

        // Initialize batches for each index type
        let mut regular_batch = if !indexes.is_empty() {
            Some(KvBatch::default())
        } else {
            None
        };
        let mut geo_batch = if !geo_indexes.is_empty() {
            Some(KvBatch::default())
        } else {
            None
        };
        let mut ft_batch = if !ft_indexes.is_empty() {
            Some(KvBatch::default())
        } else {
            None
        };

        // Counters for batch flushing
        let mut regular_count = 0;
        let mut geo_count = 0;
        let mut ft_count = 0;

        for result in iter.flatten() {
            let (key, value) = result;
            if !key.starts_with(prefix) {
                break;
            }

            if let Ok(doc) = deserialize_doc(&value) {
                doc_count += 1;
                let doc_value = doc.to_value();

                // Build regular indexes
                if let Some(ref mut batch) = regular_batch {
                    for index in &indexes {
                        let field_values: Vec<Value> = index
                            .fields
                            .iter()
                            .map(|f| extract_field_value(&doc_value, f))
                            .collect();

                        if index.holds(&field_values) {
                            for entry_key in Self::index_entry_keys(index, &field_values, &doc.key)
                            {
                                batch.put_ks(&cf, entry_key, doc.key.as_bytes());
                            }
                            regular_count += 1;

                            // Flush batch periodically
                            if regular_count % BATCH_SIZE_LIMIT == 0 {
                                let _ = db.write(std::mem::take(batch));
                            }
                        }
                    }
                }

                // Build geo indexes
                if let Some(ref mut batch) = geo_batch {
                    for geo_index in &geo_indexes {
                        let field_value = extract_field_value(&doc_value, &geo_index.field);
                        if !field_value.is_null() {
                            let entry_key = Self::geo_entry_key(&geo_index.name, &doc.key);
                            if let Ok(geo_data) = serde_json::to_vec(&field_value) {
                                batch.put_ks(&cf, entry_key, &geo_data);
                                geo_count += 1;

                                // Flush batch periodically
                                if geo_count % BATCH_SIZE_LIMIT == 0 {
                                    let _ = db.write(std::mem::take(batch));
                                }
                            }
                        }
                    }
                }

                // Build fulltext indexes
                if let Some(ref mut batch) = ft_batch {
                    for ft_index in &ft_indexes {
                        for field in &ft_index.fields {
                            let field_value = extract_field_value(&doc_value, field);
                            if let Some(text) = field_value.as_str() {
                                for term in &Self::ft_terms(ft_index, text) {
                                    let term_key =
                                        Self::ft_term_key(&ft_index.name, term, &doc.key);
                                    batch.put_ks(&cf, term_key, doc.key.as_bytes());
                                    ft_count += 1;
                                }

                                // Flush batch periodically (check after each field)
                                if ft_count >= BATCH_SIZE_LIMIT {
                                    let _ = db.write(std::mem::take(batch));
                                    ft_count = 0;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Write any remaining entries in batches
        if let Some(batch) = regular_batch {
            if regular_count > 0 && regular_count % BATCH_SIZE_LIMIT != 0 {
                let _ = db.write(batch);
            }
        }
        if let Some(batch) = geo_batch {
            if geo_count > 0 && geo_count % BATCH_SIZE_LIMIT != 0 {
                let _ = db.write(batch);
            }
        }
        if let Some(batch) = ft_batch {
            if ft_count > 0 {
                let _ = db.write(batch);
            }
        }

        tracing::info!(
            "rebuild_all_indexes: Streamed {} docs in {:?}",
            doc_count,
            stream_start.elapsed()
        );

        // Rebuild vector indexes
        let vector_configs = self.get_all_vector_index_configs();
        if !vector_configs.is_empty() {
            let vec_start = std::time::Instant::now();

            // Clear all vector indexes
            for entry in self.vector_indexes.iter() {
                entry.clear();
            }

            // Re-index all documents - stream from storage
            let prefix = DOC_PREFIX.as_bytes();
            let iter = db.prefix_iterator_ks(&cf, prefix);

            for result in iter.flatten() {
                let (key, value) = result;
                if !key.starts_with(prefix) {
                    break;
                }

                if let Ok(doc) = deserialize_doc(&value) {
                    let doc_value = doc.to_value();
                    self.update_vector_indexes_on_upsert(&doc.key, &doc_value);
                }
            }

            // Persist vector indexes
            if let Err(e) = self.persist_vector_indexes() {
                tracing::warn!("Failed to persist vector indexes during rebuild: {}", e);
            }

            tracing::info!(
                "rebuild_all_indexes: Vector indexes ({}) took {:?}",
                vector_configs.len(),
                vec_start.elapsed()
            );
        }

        tracing::info!(
            "rebuild_all_indexes: Total time {:?}",
            total_start.elapsed()
        );
        Ok(doc_count)
    }

    /// Index only the provided documents (for incremental indexing)
    pub fn index_documents(&self, docs: &[Document]) -> DbResult<usize> {
        let total_start = std::time::Instant::now();

        let indexes = self.get_all_indexes();
        let geo_indexes = self.get_all_geo_indexes();
        let ft_indexes = self.get_all_fulltext_indexes();

        if indexes.is_empty() && geo_indexes.is_empty() && ft_indexes.is_empty() {
            return Ok(0);
        }

        // Preload bloom/cuckoo filters
        for index in &indexes {
            if index.index_type == IndexType::Bloom {
                let _ = self.get_or_create_bloom_filter(&index.name);
            } else if index.index_type == IndexType::Cuckoo {
                self.preload_cuckoo_filter(&index.name);
            }
        }

        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        // Build regular indexes
        if !indexes.is_empty() {
            let mut batch = KvBatch::default();
            for doc in docs {
                let doc_value = doc.to_value();
                for index in &indexes {
                    let field_values: Vec<Value> = index
                        .fields
                        .iter()
                        .map(|f| extract_field_value(&doc_value, f))
                        .collect();

                    if index.holds(&field_values) {
                        for entry_key in Self::index_entry_keys(index, &field_values, &doc.key) {
                            batch.put_ks(&cf, entry_key, doc.key.as_bytes());
                        }

                        if index.index_type == IndexType::Bloom {
                            for value in &field_values {
                                self.bloom_insert(&index.name, &value.to_string());
                            }
                        } else if index.index_type == IndexType::Cuckoo {
                            for value in &field_values {
                                self.cuckoo_insert(&index.name, &value.to_string());
                            }
                        }
                    }
                }
            }
            let _ = db.write(batch);
        }

        // Build geo indexes
        if !geo_indexes.is_empty() {
            let mut batch = KvBatch::default();
            for doc in docs {
                let doc_value = doc.to_value();
                for geo_index in &geo_indexes {
                    let field_value = extract_field_value(&doc_value, &geo_index.field);
                    if !field_value.is_null() {
                        let entry_key = Self::geo_entry_key(&geo_index.name, &doc.key);
                        if let Ok(geo_data) = serde_json::to_vec(&field_value) {
                            batch.put_ks(&cf, entry_key, &geo_data);
                        }
                    }
                }
            }
            let _ = db.write(batch);
        }

        // Build fulltext indexes
        if !ft_indexes.is_empty() {
            let mut batch = KvBatch::default();
            for doc in docs {
                let doc_value = doc.to_value();
                for ft_index in &ft_indexes {
                    for field in &ft_index.fields {
                        let field_value = extract_field_value(&doc_value, field);
                        if let Some(text) = field_value.as_str() {
                            for term in &Self::ft_terms(ft_index, text) {
                                let term_key = Self::ft_term_key(&ft_index.name, term, &doc.key);
                                batch.put_ks(&cf, term_key, doc.key.as_bytes());
                            }
                        }
                    }
                }
            }
            let _ = db.write(batch);
        }

        tracing::info!("index_documents: Total time {:?}", total_start.elapsed());
        Ok(docs.len())
    }

    /// Get index statistics
    pub fn get_index_stats(&self, name: &str) -> Option<IndexStats> {
        let index = self.get_index(name)?;

        let db = &self.db;
        let cf = self.ks.handle(&self.db)?;

        // Count entries
        let prefix = format!("{}{}:", IDX_PREFIX, name);
        let iter = db.prefix_iterator_ks(&cf, prefix.as_bytes());
        let count = iter
            .filter(|r| {
                r.as_ref()
                    .map(|(k, _)| k.starts_with(prefix.as_bytes()))
                    .unwrap_or(false)
            })
            .count();

        Some(IndexStats {
            name: index.name,
            fields: index.fields.clone(),
            field: index.fields.first().cloned().unwrap_or_default(),
            index_type: index.index_type,
            unique: index.unique,
            unique_values: count,
            indexed_documents: count,
            min_length: None,
            fold_accents: None,
            sparse: Some(index.sparse),
        })
    }

    /// The entries a document with these index field values gets in `index`:
    /// one (`idx_entry_key`) for every type but Trigram, which gets one per
    /// distinct trigram of the field's text, `idx:<name>:<hex(trigram)>:<key>`.
    /// The text is what `LIKE(…)` matches against (`text_of`: a string as it
    /// is, anything else stringified), so a document LIKE can match always
    /// has the pattern's trigrams; null has none.
    pub(crate) fn index_entry_keys(index: &Index, values: &[Value], doc_key: &str) -> Vec<Vec<u8>> {
        if index.index_type != IndexType::Trigram {
            return vec![Self::idx_entry_key(&index.name, values, doc_key)];
        }
        let Some(value) = values.first().filter(|v| !v.is_null()) else {
            return Vec::new();
        };
        let text = crate::sdbql::executor::builtins::string::text_of(value);
        crate::storage::index::text_trigrams(&text)
            .into_iter()
            .map(|gram| {
                format!(
                    "{}{}:{}:{}",
                    IDX_PREFIX,
                    index.name,
                    hex::encode(gram.as_bytes()),
                    doc_key
                )
                .into_bytes()
            })
            .collect()
    }

    /// The documents whose `index` (a Trigram index) entries hold every one
    /// of `grams` — a superset of those a LIKE with these trigrams matches;
    /// the caller re-checks the FILTER. Reads each trigram's entries, rarest
    /// first is not known, so in order, intersecting as it goes and stopping
    /// when nothing is left. `None` when `grams` is empty (the index cannot
    /// narrow the read) or the candidates pass `cap`.
    pub fn index_trigram_candidates(
        &self,
        index: &Index,
        grams: &std::collections::BTreeSet<String>,
        cap: usize,
    ) -> Option<Vec<Document>> {
        if index.index_type != IndexType::Trigram || grams.is_empty() {
            return None;
        }
        let cf = self.ks.handle(&self.db)?;
        let mut candidates: Option<std::collections::HashSet<Vec<u8>>> = None;
        for gram in grams {
            let prefix = format!(
                "{}{}:{}:",
                IDX_PREFIX,
                index.name,
                hex::encode(gram.as_bytes())
            );
            let mut keys = std::collections::HashSet::new();
            let read = self.for_each_index_entry(&cf, prefix.as_bytes(), |doc_key| {
                if candidates.as_ref().is_none_or(|c| c.contains(doc_key)) {
                    keys.insert(doc_key.to_vec());
                }
                Ok(true)
            });
            read.ok()?;
            if keys.is_empty() {
                return Some(Vec::new());
            }
            candidates = Some(keys);
        }
        let candidates = candidates?;
        if candidates.len() > cap {
            return None;
        }
        let doc_keys: Vec<Vec<u8>> = candidates
            .iter()
            .map(|k| Self::doc_key(&String::from_utf8_lossy(k)))
            .collect();
        Some(
            self.db
                .multi_get_ks(&cf, doc_keys.iter().map(|k| k.as_slice()))
                .into_iter()
                .filter_map(|r| r.ok())
                .flatten()
                .filter_map(|bytes| deserialize_doc(&bytes).ok())
                .collect(),
        )
    }

    /// Encode index field values the way `idx_entry_key` does: hex of the
    /// order-preserving codec per value, joined with `_`. Neither hex nor `_`
    /// contains `:`, so `idx:<name>:<encoded>:` is an exact value prefix.
    pub(crate) fn encode_index_values(values: &[Value]) -> String {
        values
            .iter()
            .map(|v| hex::encode(crate::storage::codec::encode_key(v)))
            .collect::<Vec<_>>()
            .join("_")
    }

    fn index_field_values(index: &Index, doc_value: &Value) -> Vec<Value> {
        index
            .fields
            .iter()
            .map(|f| extract_field_value(doc_value, f))
            .collect()
    }

    /// One token per unique index the document has a (non-null) value for:
    /// `<index>:<encoded values>`. Used to lock the value (audit D4) so the
    /// uniqueness check and the write that claims the value are atomic, and
    /// to catch duplicates inside a single batch.
    pub(crate) fn unique_tokens(&self, doc_value: &Value) -> Vec<String> {
        let Some(meta) = self.index_meta() else {
            return Vec::new();
        };
        meta.indexes
            .iter()
            .filter(|i| i.unique)
            .filter_map(|index| {
                let values = Self::index_field_values(index, doc_value);
                if values.iter().all(|v| v.is_null()) {
                    None
                } else {
                    Some(format!(
                        "{}:{}",
                        index.name,
                        Self::encode_index_values(&values)
                    ))
                }
            })
            .collect()
    }

    /// Check unique constraints before inserting/updating a document.
    ///
    /// Only atomic with the write when the caller holds the document's key
    /// stripe and the stripes for `unique_tokens(doc_value)` (audit D4).
    pub(crate) fn check_unique_constraints(
        &self,
        doc_key: &str,
        doc_value: &Value,
    ) -> DbResult<()> {
        let Some(meta) = self.index_meta() else {
            return Ok(());
        };
        if !meta.indexes.iter().any(|i| i.unique) {
            return Ok(());
        }
        let db = &self.db;
        let cf = self.ks.handle(&self.db).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped mid-operation)",
                self.name
            ))
        })?;

        for index in meta.indexes.iter().filter(|i| i.unique) {
            let field_values = Self::index_field_values(index, doc_value);

            // Skip if all values are null
            if field_values.iter().all(|v| v.is_null()) {
                continue;
            }

            let value_part = Self::encode_index_values(&field_values);
            // Key format: idx:<index_name>:<value_part>:<doc_key>
            let prefix = format!("{}{}:{}:", IDX_PREFIX, index.name, value_part);

            // Every entry under the value, not just the first: the first may be
            // this document's own (or a stale) entry while a later one belongs
            // to another document.
            for item in db.prefix_iterator_ks(&cf, prefix.as_bytes()) {
                let Ok((key, value)) = item else {
                    break;
                };
                if !key.starts_with(prefix.as_bytes()) {
                    break;
                }
                let existing_key = String::from_utf8_lossy(&value); // Value in index is doc_key
                if existing_key == doc_key {
                    continue; // Allow update of the same document
                }
                // An entry whose document is gone or no longer holds the
                // value is leftover garbage (from before D4 was fixed), not a
                // conflict — it must not make the value unusable forever.
                let holds_value = self.get(&existing_key).is_ok_and(|doc| {
                    let values = Self::index_field_values(index, &doc.to_value());
                    Self::encode_index_values(&values) == value_part
                });
                if holds_value {
                    return Err(DbError::InvalidDocument(format!(
                        "Unique constraint violated: fields '{:?}' with value {:?} already exists in index '{}'",
                        index.fields, field_values, index.name
                    )));
                }
            }
        }
        Ok(())
    }

    /// Update indexes on document insert
    #[allow(dead_code)]
    pub(crate) fn update_indexes_on_insert(
        &self,
        doc_key: &str,
        doc_value: &Value,
    ) -> DbResult<()> {
        let indexes = self.get_all_indexes();

        // Preload bloom/cuckoo filters
        for index in &indexes {
            if index.index_type == IndexType::Bloom {
                let _ = self.get_or_create_bloom_filter(&index.name);
            } else if index.index_type == IndexType::Cuckoo {
                self.preload_cuckoo_filter(&index.name);
            }
        }

        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        for index in indexes {
            let field_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(doc_value, f))
                .collect();

            if index.holds(&field_values) {
                for entry_key in Self::index_entry_keys(&index, &field_values, doc_key) {
                    db.put_ks(&cf, entry_key, doc_key.as_bytes()).map_err(|e| {
                        DbError::InternalError(format!("Failed to update index: {}", e))
                    })?;
                }

                if index.index_type == IndexType::Bloom {
                    for value in field_values {
                        self.bloom_insert(&index.name, &value.to_string());
                    }
                } else if index.index_type == IndexType::Cuckoo {
                    for value in field_values {
                        self.cuckoo_insert(&index.name, &value.to_string());
                    }
                }
            }
        }
        let _ = db;

        // Update geo indexes
        let geo_indexes = self.get_all_geo_indexes();
        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        for geo_index in geo_indexes {
            let field_value = extract_field_value(doc_value, &geo_index.field);
            if !field_value.is_null() {
                let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
                let geo_data = serde_json::to_vec(&field_value)?;
                db.put_ks(&cf, entry_key, &geo_data).map_err(|e| {
                    DbError::InternalError(format!("Failed to update geo index: {}", e))
                })?;
            }
        }

        Ok(())
    }

    /// Update indexes on document update
    #[allow(dead_code)]
    pub(crate) fn update_indexes_on_update(
        &self,
        doc_key: &str,
        old_value: &Value,
        new_value: &Value,
    ) -> DbResult<()> {
        let indexes = self.get_all_indexes();
        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        for index in indexes {
            let old_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(old_value, f))
                .collect();
            let new_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(new_value, f))
                .collect();

            // Remove old entry
            if index.holds(&old_values) {
                for old_entry_key in Self::index_entry_keys(&index, &old_values, doc_key) {
                    db.delete_ks(&cf, old_entry_key).map_err(|e| {
                        DbError::InternalError(format!("Failed to update index: {}", e))
                    })?;
                }
            }

            // Add new entry
            if index.holds(&new_values) {
                for new_entry_key in Self::index_entry_keys(&index, &new_values, doc_key) {
                    db.put_ks(&cf, new_entry_key, doc_key.as_bytes())
                        .map_err(|e| {
                            DbError::InternalError(format!("Failed to update index: {}", e))
                        })?;
                }
            }
        }
        let _ = db;

        // Update geo indexes
        let geo_indexes = self.get_all_geo_indexes();
        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        for geo_index in geo_indexes {
            let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
            let new_field = extract_field_value(new_value, &geo_index.field);

            if !new_field.is_null() {
                let geo_data = serde_json::to_vec(&new_field)?;
                db.put_ks(&cf, entry_key, &geo_data).map_err(|e| {
                    DbError::InternalError(format!("Failed to update geo index: {}", e))
                })?;
            } else {
                db.delete_ks(&cf, entry_key).map_err(|e| {
                    DbError::InternalError(format!("Failed to update geo index: {}", e))
                })?;
            }
        }

        Ok(())
    }

    /// Update indexes on document delete
    #[allow(dead_code)]
    pub(crate) fn update_indexes_on_delete(
        &self,
        doc_key: &str,
        doc_value: &Value,
    ) -> DbResult<()> {
        let indexes = self.get_all_indexes();
        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        for index in indexes {
            let field_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(doc_value, f))
                .collect();

            if index.holds(&field_values) {
                for entry_key in Self::index_entry_keys(&index, &field_values, doc_key) {
                    db.delete_ks(&cf, entry_key).map_err(|e| {
                        DbError::InternalError(format!("Failed to update index: {}", e))
                    })?;
                }
            }
        }
        let _ = db;

        // Update geo indexes
        let geo_indexes = self.get_all_geo_indexes();
        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        for geo_index in geo_indexes {
            let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
            db.delete_ks(&cf, entry_key).map_err(|e| {
                DbError::InternalError(format!("Failed to update geo index: {}", e))
            })?;
        }

        Ok(())
    }

    /// Get an index for a field
    pub fn get_index_for_field(&self, field: &str) -> Option<Index> {
        self.index_meta()?
            .indexes
            .iter()
            // A trigram index holds trigrams, not the field's values.
            .filter(|index| index.index_type != IndexType::Trigram)
            .find(|index| index.fields.first().map(|s| s.as_str()) == Some(field))
            .cloned()
    }

    /// Get a single-field index for a field. Used by equality and range
    /// lookups that encode only one value into the lookup prefix: a compound
    /// index `[a, b]` stores entries keyed by `<hex_a>_<hex_b>` joined by `_`,
    /// so a prefix built from `<hex_a>` alone never matches its entries and
    /// silently returns no results.
    pub(crate) fn get_single_field_index_for_field(&self, field: &str) -> Option<Index> {
        self.index_meta()?
            .indexes
            .iter()
            .find(|index| {
                // A trigram index holds trigrams, not the field's values.
                index.index_type != IndexType::Trigram
                    && index.fields.len() == 1
                    && index.fields.first().map(|s| s.as_str()) == Some(field)
            })
            .cloned()
    }

    /// Lookup documents where field > value
    pub fn index_lookup_gt(
        &self,
        field: &str,
        value: &Value,
        limit: Option<usize>,
    ) -> Option<Vec<Document>> {
        self.index_range_scan(field, value, false, true, limit)
    }

    /// Lookup documents where field >= value
    pub fn index_lookup_gte(
        &self,
        field: &str,
        value: &Value,
        limit: Option<usize>,
    ) -> Option<Vec<Document>> {
        self.index_range_scan(field, value, true, true, limit)
    }

    /// Lookup documents where field < value
    pub fn index_lookup_lt(
        &self,
        field: &str,
        value: &Value,
        limit: Option<usize>,
    ) -> Option<Vec<Document>> {
        self.index_range_scan(field, value, false, false, limit)
    }

    /// Lookup documents where field <= value
    pub fn index_lookup_lte(
        &self,
        field: &str,
        value: &Value,
        limit: Option<usize>,
    ) -> Option<Vec<Document>> {
        self.index_range_scan(field, value, true, false, limit)
    }

    fn index_range_scan(
        &self,
        field: &str,
        value: &Value,
        inclusive: bool,
        forward: bool,
        limit: Option<usize>,
    ) -> Option<Vec<Document>> {
        let index = self.get_single_field_index_for_field(field)?;
        let index_name = &index.name;
        let value_key = crate::storage::codec::encode_key(value);
        let value_str = hex::encode(value_key);

        let db = &self.db;
        let cf = self.ks.handle(&self.db)?;

        let prefix_base = format!("{}{}:", IDX_PREFIX, index_name);
        let seek_key = format!("{}{}:", prefix_base, value_str);

        // `None` reads the whole range. A hidden default cap here used to stop
        // an unlimited read at 1000 keys and hand back a silently truncated
        // result; bounding memory is the caller's job — the query executor
        // passes `scan_cap()`, so an oversized range fails its row budget
        // instead of losing rows.
        let cap = limit.unwrap_or(usize::MAX);
        let mut doc_keys = Vec::new();

        if forward {
            let mode = IterMode::From(seek_key.as_bytes(), Direction::Forward);
            let iter = db.iterator_ks(&cf, mode);

            for result in iter {
                if let Ok((k, v)) = result {
                    if !k.starts_with(prefix_base.as_bytes()) {
                        break;
                    }
                    // Check inclusion/exclusion
                    // If key starts with seek_key, it matches the value exactly
                    if !inclusive && k.starts_with(seek_key.as_bytes()) {
                        continue;
                    }

                    let val_str = String::from_utf8_lossy(&v);
                    doc_keys.push(Self::doc_key(&val_str));
                }
                if doc_keys.len() >= cap {
                    break;
                }
            }
        } else {
            // Entries equal to `value` are `<seek_key><doc_key>`, which sort
            // AFTER `seek_key` itself: a reverse seek there starts below them
            // and `<=` silently lost every row equal to its bound. Seek past
            // them instead — `;` is the byte after `:`, and no hex digit sits
            // between the two, so only this value's entries lie in the gap.
            let reverse_seek = format!("{}{};", prefix_base, value_str);
            let mode = IterMode::From(reverse_seek.as_bytes(), Direction::Reverse);
            let iter = db.iterator_ks(&cf, mode);

            for result in iter {
                if let Ok((k, v)) = result {
                    if !k.starts_with(prefix_base.as_bytes()) {
                        break;
                    }

                    if !inclusive && k.starts_with(seek_key.as_bytes()) {
                        continue;
                    }

                    let val_str = String::from_utf8_lossy(&v);
                    doc_keys.push(Self::doc_key(&val_str));
                }
                if doc_keys.len() >= cap {
                    break;
                }
            }
        }

        if doc_keys.is_empty() {
            return Some(Vec::new());
        }

        let results = db.multi_get_ks(&cf, doc_keys.iter().map(|k| k.as_slice()));
        let docs: Vec<Document> = results
            .into_iter()
            .filter_map(|r| r.ok())
            .flatten()
            .filter_map(|bytes| deserialize_doc(&bytes).ok())
            .collect();

        Some(docs)
    }

    /// A Bloom or Cuckoo index whose filter says `value` is definitely
    /// absent. Null is never checked against the filter: its entries, in a
    /// non-sparse index, are found by reading the index.
    fn filter_rules_out(&self, index: &Index, value: &Value) -> bool {
        !value.is_null()
            && ((index.index_type == IndexType::Bloom
                && !self.bloom_check(&index.name, &value.to_string()))
                || (index.index_type == IndexType::Cuckoo
                    && !self.cuckoo_check(&index.name, &value.to_string())))
    }

    /// Lookup documents using index (equality)
    pub fn index_lookup_eq(&self, field: &str, value: &Value) -> Option<Vec<Document>> {
        let index = self.get_single_field_index_for_field(field)?;
        // A sparse index skips null values at insert time (`Index::holds`),
        // so there `field == null` falls back to a full scan to find docs
        // whose indexed field is null. A non-sparse one holds them.
        if value.is_null() && index.sparse {
            return None;
        }

        // Fast-path: Check Bloom/Cuckoo Filter if available
        if self.filter_rules_out(&index, value) {
            return Some(Vec::new());
        }

        let value_str = hex::encode(crate::storage::codec::encode_key(value));

        let db = &self.db;
        let cf = self.ks.handle(&self.db)?;

        // Prefix for the lookup
        let prefix = format!("{}{}:{}:", IDX_PREFIX, index.name, value_str);
        let iter = db.prefix_iterator_ks(&cf, prefix.as_bytes());

        // Collect document keys from index
        let doc_keys: Vec<Vec<u8>> = iter
            .filter_map(|r| r.ok())
            .take_while(|(k, _)| k.starts_with(prefix.as_bytes()))
            .map(|(_, v)| {
                // Value in index is doc_key
                // But we need to use doc_key() helper to get the lookup key for the document?
                // Wait, previous implem:
                // let key_str = String::from_utf8_lossy(&v);
                // Self::doc_key(&key_str)
                // Yes.
                let key_str = String::from_utf8_lossy(&v);
                Self::doc_key(&key_str)
            })
            .collect();

        if doc_keys.is_empty() {
            return Some(Vec::new());
        }

        // Use multi_get for batch retrieval
        let results = db.multi_get_ks(&cf, doc_keys.iter().map(|k| k.as_slice()));

        let docs: Vec<Document> = results
            .into_iter()
            .filter_map(|r| r.ok())
            .flatten()
            .filter_map(|bytes| deserialize_doc(&bytes).ok())
            .collect();

        Some(docs)
    }

    /// The index-entry prefix of `field == value`, as [`Self::index_lookup_eq`]
    /// reads it.
    fn index_eq_prefix(&self, field: &str, value: &Value) -> EqPrefix {
        let Some(index) = self.get_single_field_index_for_field(field) else {
            return EqPrefix::NoIndex;
        };
        if value.is_null() && index.sparse {
            return EqPrefix::NoIndex;
        }
        if self.filter_rules_out(&index, value) {
            return EqPrefix::Empty;
        }
        let value_str = hex::encode(crate::storage::codec::encode_key(value));
        EqPrefix::Prefix(format!("{}{}:{}:", IDX_PREFIX, index.name, value_str).into_bytes())
    }

    /// How many entries `index` holds — one per document it indexes, so, for
    /// a sparse index, fewer than [`Self::count`] exactly when some document
    /// has all its index fields null. Reads every key of the index.
    pub(crate) fn index_entry_count(&self, index: &Index) -> usize {
        let Some(cf) = self.ks.handle(&self.db) else {
            return 0;
        };
        let prefix = format!("{}{}:", IDX_PREFIX, index.name);
        let mut n = 0usize;
        let _ = self.for_each_index_entry(&cf, prefix.as_bytes(), |_| {
            n += 1;
            Ok(true)
        });
        n
    }

    /// The index entries under `prefix`, in order: `f` gets each entry's
    /// value (the document key) and returns `false` to stop.
    fn for_each_index_entry(
        &self,
        cf: &KsCf,
        prefix: &[u8],
        mut f: impl FnMut(&[u8]) -> DbResult<bool>,
    ) -> DbResult<()> {
        // The prefix ends in ':' (or '_'), so ';' (or '`') is the first key
        // past it.
        let mut upper = prefix.to_vec();
        *upper.last_mut().expect("non-empty prefix") += 1;
        let iter = self.db.iterator_ks_bounded(
            cf,
            IterMode::From(prefix, Direction::Forward),
            None,
            Some(&upper),
        );
        for item in iter {
            let (k, v) = item.map_err(|e| DbError::InternalError(e.to_string()))?;
            if !k.starts_with(prefix) || !f(&v)? {
                break;
            }
        }
        Ok(())
    }

    /// How many documents [`Self::index_lookup_eq`] would return, read from
    /// the index alone: one entry per document, and no document fetched.
    /// `check` is called every 64k entries so a caller can enforce a
    /// deadline. `None` when no single-field index serves `field`.
    pub fn index_count_eq(
        &self,
        field: &str,
        value: &Value,
        mut check: impl FnMut() -> DbResult<()>,
    ) -> DbResult<Option<u64>> {
        let prefix = match self.index_eq_prefix(field, value) {
            EqPrefix::NoIndex => return Ok(None),
            EqPrefix::Empty => return Ok(Some(0)),
            EqPrefix::Prefix(p) => p,
        };
        let Some(cf) = self.ks.handle(&self.db) else {
            return Ok(None);
        };
        let mut n = 0u64;
        self.for_each_index_entry(&cf, &prefix, |_| {
            n += 1;
            if n.is_multiple_of(65_536) {
                check()?;
            }
            Ok(true)
        })?;
        Ok(Some(n))
    }

    /// How many documents match every `field == value` of `conditions`,
    /// read from a composite (multi-field) index alone — one condition is
    /// enough, read as the index's first field. The index's leading fields
    /// must be exactly the condition fields, in any order: its entries
    /// `idx:<name>:hex(v1)_…_hex(vn)[_…]:<key>` then share one prefix — `:`
    /// after it when the index has no other field, `_` when it has more — and
    /// there is one entry per matching document.
    ///
    /// `None` (the caller scans) when no index fits, when a field appears
    /// twice, when every value is null and the only fitting indexes are
    /// sparse (such rows are not in them), or when a value is an array or
    /// object (the codec compares those as JSON text, not as `==` does).
    /// `check` is called every 64k entries.
    pub fn index_count_eq_composite(
        &self,
        conditions: &[(String, Value)],
        mut check: impl FnMut() -> DbResult<()>,
    ) -> DbResult<Option<u64>> {
        let distinct_fields = conditions
            .iter()
            .enumerate()
            .all(|(i, (f, _))| conditions[..i].iter().all(|(g, _)| g != f));
        if conditions.is_empty()
            || !distinct_fields
            || conditions
                .iter()
                .any(|(_, v)| matches!(v, Value::Array(_) | Value::Object(_)))
        {
            return Ok(None);
        }
        let n = conditions.len();
        // An index with exactly these fields first, else the shortest longer one.
        let Some(index) = self
            .get_all_indexes()
            .into_iter()
            .filter(|index| {
                // One condition on a single-field index is
                // `index_count_eq`'s. Rows whose index fields are all null
                // are only in a non-sparse index.
                matches!(index.index_type, IndexType::Persistent | IndexType::Hash)
                    && index.fields.len() >= n.max(2)
                    && (!index.sparse || !conditions.iter().all(|(_, v)| v.is_null()))
                    && index.fields[..n]
                        .iter()
                        .all(|f| conditions.iter().any(|(cf, _)| cf == f))
            })
            .min_by_key(|index| index.fields.len())
        else {
            return Ok(None);
        };
        let values: Vec<Value> = index.fields[..n]
            .iter()
            .filter_map(|f| {
                conditions
                    .iter()
                    .find(|(cf, _)| cf == f)
                    .map(|(_, v)| v.clone())
            })
            .collect();
        let separator = if index.fields.len() == n { ':' } else { '_' };
        let prefix = format!(
            "{}{}:{}{}",
            IDX_PREFIX,
            index.name,
            Self::encode_index_values(&values),
            separator
        );
        let Some(cf) = self.ks.handle(&self.db) else {
            return Ok(None);
        };
        let mut count = 0u64;
        self.for_each_index_entry(&cf, prefix.as_bytes(), |_| {
            count += 1;
            if count.is_multiple_of(65_536) {
                check()?;
            }
            Ok(true)
        })?;
        Ok(Some(count))
    }

    /// [`Self::index_lookup_eq`] then rows `offset..offset + limit`, written
    /// for `shape` straight from the stored bytes (see
    /// [`crate::storage::raw_rows`]). `Ok(None)` when no single-field index
    /// serves `field`, or when a document is missing or cannot be copied
    /// this way: the caller then runs the query the ordinary way. The flag
    /// says whether the index holds the value at all, rows skipped by
    /// `offset` included.
    pub fn index_raw_eq(
        &self,
        field: &str,
        value: &Value,
        offset: usize,
        limit: usize,
        shape: &crate::storage::raw_rows::RawShape,
        mut check: impl FnMut(usize) -> DbResult<()>,
    ) -> DbResult<Option<(usize, Vec<u8>, bool)>> {
        let prefix = match self.index_eq_prefix(field, value) {
            EqPrefix::NoIndex => return Ok(None),
            EqPrefix::Empty => return Ok(Some((0, Vec::new(), false))),
            EqPrefix::Prefix(p) => p,
        };
        let Some(cf) = self.ks.handle(&self.db) else {
            return Ok(None);
        };
        let mut keys: Vec<Vec<u8>> = Vec::new();
        let mut skipped = 0usize;
        let mut seen = false;
        self.for_each_index_entry(&cf, &prefix, |doc_key| {
            seen = true;
            if limit == 0 {
                return Ok(false);
            }
            if skipped < offset {
                skipped += 1;
                return Ok(true);
            }
            keys.push(Self::doc_key(&String::from_utf8_lossy(doc_key)));
            if keys.len().is_multiple_of(4096) {
                check(keys.len())?;
            }
            Ok(keys.len() < limit)
        })?;
        let mut out = Vec::new();
        for doc in self.db.multi_get_ks(&cf, keys.iter().map(|k| k.as_slice())) {
            // A missing document is dropped by the ordinary path; leave the
            // whole query to it rather than reproduce that here.
            let Ok(Some(bytes)) = doc else {
                return Ok(None);
            };
            if crate::storage::raw_rows::write_row(&bytes, shape, &mut out).is_none() {
                return Ok(None);
            }
        }
        Ok(Some((keys.len(), out, seen)))
    }

    /// Documents of a multi-field `index` whose leading fields equal
    /// `eq_values` and whose next field lies within `lower`/`upper`
    /// (`(value, inclusive)`), read as one bounded range of the index.
    ///
    /// Entries are `idx:<name>:hex(v1)_hex(v2)_…:<doc_key>`. The key codec is
    /// order-preserving, ranks types like `compare_values`, and is
    /// prefix-free, so under the prefix `P` of the equality values the
    /// entries are ordered by the next field: those equal to `v` all start
    /// with `P + hex(v)` followed by `_` or `:`, both below `0x7f`, and only
    /// those entries lie between `P + hex(v)` and `P + hex(v) + 0x7f`.
    ///
    /// Stops after `cap` documents. `None` (the caller scans) when the index
    /// is not a regular one, when `eq_values` covers the whole index while a
    /// bound is given, when any value is an array or object (the codec orders
    /// those as JSON text, not as SDBQL compares them), or, for a sparse
    /// index, when `eq_values` is empty or all null (rows whose index fields
    /// are all null are not in it). On a non-sparse index, with `eq_values`
    /// empty, the bounds apply to the first field.
    pub fn index_scan_composite(
        &self,
        index: &crate::storage::index::Index,
        eq_values: &[Value],
        lower: Option<(&Value, bool)>,
        upper: Option<(&Value, bool)>,
        cap: usize,
    ) -> Option<Vec<Document>> {
        let n = eq_values.len();
        let has_range = lower.is_some() || upper.is_some();
        let complex = |v: &Value| matches!(v, Value::Array(_) | Value::Object(_));
        if !matches!(index.index_type, IndexType::Persistent | IndexType::Hash)
            || n > index.fields.len()
            || (n == index.fields.len() && has_range)
            || (index.sparse && (n == 0 || eq_values.iter().all(Value::is_null)))
            || eq_values.iter().any(complex)
            || lower.is_some_and(|(v, _)| complex(v))
            || upper.is_some_and(|(v, _)| complex(v))
        {
            return None;
        }
        // `idx:<name>:` alone when nothing is fixed: the bounds then apply to
        // the first field's encoding, which starts right after it.
        let prefix = if n == 0 {
            format!("{}{}:", IDX_PREFIX, index.name)
        } else {
            let separator = if n == index.fields.len() { ':' } else { '_' };
            format!(
                "{}{}:{}{}",
                IDX_PREFIX,
                index.name,
                Self::encode_index_values(eq_values),
                separator
            )
        }
        .into_bytes();
        let bound = |v: &Value, past_equal: bool| {
            let mut key = prefix.clone();
            key.extend_from_slice(hex::encode(crate::storage::codec::encode_key(v)).as_bytes());
            if past_equal {
                key.push(0x7f);
            }
            key
        };
        // `>= v` starts at the entries equal to v, `> v` past them.
        let start = match lower {
            Some((v, inclusive)) => bound(v, !inclusive),
            None => prefix.clone(),
        };
        // `<= v` ends past the entries equal to v, `< v` before them.
        let end = match upper {
            Some((v, inclusive)) => bound(v, inclusive),
            None => {
                // The prefix ends in ':' or '_'; the next byte is the first
                // key past every entry under it.
                let mut end = prefix.clone();
                *end.last_mut().expect("non-empty prefix") += 1;
                end
            }
        };
        if start >= end {
            return Some(Vec::new());
        }

        let cf = self.ks.handle(&self.db)?;
        let mut doc_keys = Vec::new();
        let iter = self.db.iterator_ks_bounded(
            &cf,
            IterMode::From(&start, Direction::Forward),
            Some(&start),
            Some(&end),
        );
        for item in iter {
            let Ok((k, v)) = item else { continue };
            if !k.starts_with(&prefix) || k.as_ref() >= end.as_slice() {
                break;
            }
            doc_keys.push(Self::doc_key(&String::from_utf8_lossy(&v)));
            if doc_keys.len() >= cap {
                break;
            }
        }
        if doc_keys.is_empty() {
            return Some(Vec::new());
        }
        let docs = self
            .db
            .multi_get_ks(&cf, doc_keys.iter().map(|k| k.as_slice()))
            .into_iter()
            .filter_map(|r| r.ok())
            .flatten()
            .filter_map(|bytes| deserialize_doc(&bytes).ok())
            .collect();
        Some(docs)
    }

    /// Lookup documents using index (equality) with limit
    pub fn index_lookup_eq_limit(
        &self,
        field: &str,
        value: &Value,
        limit: usize,
    ) -> Option<Vec<Document>> {
        let index = self.get_single_field_index_for_field(field)?;
        if value.is_null() && index.sparse {
            return None;
        }

        // Same Bloom/Cuckoo definite-miss early-exit as `index_lookup_eq`
        if self.filter_rules_out(&index, value) {
            return Some(Vec::new());
        }

        let value_str = hex::encode(crate::storage::codec::encode_key(value));

        let db = &self.db;
        let cf = self.ks.handle(&self.db)?;

        let prefix = format!("{}{}:{}:", IDX_PREFIX, index.name, value_str);
        let iter = db.prefix_iterator_ks(&cf, prefix.as_bytes());

        let doc_keys: Vec<Vec<u8>> = iter
            .filter_map(|r| r.ok())
            .take_while(|(k, _)| k.starts_with(prefix.as_bytes()))
            .take(limit)
            .map(|(_, v)| {
                let key_str = String::from_utf8_lossy(&v);
                Self::doc_key(&key_str)
            })
            .collect();

        if doc_keys.is_empty() {
            return Some(Vec::new());
        }

        let results = db.multi_get_ks(&cf, doc_keys.iter().map(|k| k.as_slice()));
        let docs: Vec<Document> = results
            .into_iter()
            .filter_map(|r| r.ok())
            .flatten()
            .filter_map(|bytes| deserialize_doc(&bytes).ok())
            .collect();

        Some(docs)
    }

    /// Get documents sorted by indexed field
    pub fn index_sorted(
        &self,
        field: &str,
        ascending: bool,
        limit: Option<usize>,
    ) -> Option<Vec<Document>> {
        // Optimization for Primary Key Sort
        if field == "_id" || field == "_key" {
            let db = &self.db;
            let cf = self.ks.handle(&self.db)?;
            let prefix = DOC_PREFIX.as_bytes();

            let iter = if ascending {
                let mode = IterMode::From(prefix, Direction::Forward);
                db.iterator_ks(&cf, mode)
            } else {
                // For descending, we seek past the end of the prefix
                let mut seek_key = prefix.to_vec();
                seek_key.push(0xFF);
                let mode = IterMode::From(&seek_key, Direction::Reverse);
                db.iterator_ks(&cf, mode)
            };

            let docs: Vec<Document> = iter
                .filter_map(|r| r.ok())
                .take_while(|(k, _)| k.starts_with(prefix))
                .filter_map(|(_, v)| deserialize_doc(&v).ok())
                .take(limit.unwrap_or(usize::MAX))
                .collect();

            return Some(docs);
        }

        let index = self.get_index_for_field(field)?;
        let index_name = index.name.clone();
        let prefix = format!("{}{}:", IDX_PREFIX, index_name);

        let db = &self.db;
        let cf = self.ks.handle(&self.db)?;
        let prefix_bytes = prefix.as_bytes();

        let iter = if ascending {
            let mode = IterMode::From(prefix_bytes, Direction::Forward);
            db.iterator_ks(&cf, mode)
        } else {
            let mut seek_key = prefix.as_bytes().to_vec();
            seek_key.push(0xFF);
            let mode = IterMode::From(&seek_key, Direction::Reverse);
            db.iterator_ks(&cf, mode)
        };

        let doc_keys: Vec<String> = iter
            .filter_map(|r| r.ok())
            .take_while(|(k, _)| k.starts_with(prefix_bytes))
            .map(|(_, v)| String::from_utf8_lossy(&v).to_string())
            .take(limit.unwrap_or(usize::MAX))
            .collect();

        let _ = db;

        if doc_keys.is_empty() {
            return Some(Vec::new());
        }

        let docs = self.get_many(&doc_keys);

        // Re-order docs based on doc_keys order
        let doc_map: std::collections::HashMap<_, _> =
            docs.into_iter().map(|d| (d.key.clone(), d)).collect();

        let result: Vec<Document> = doc_keys
            .into_iter()
            .filter_map(|key| doc_map.get(&key).cloned())
            .collect();

        Some(result)
    }

    /// Documents matching `conditions` (field equalities) in `sort_field`
    /// order, read from a composite index that ends with `sort_field` and
    /// whose other fields are all equal to a condition value — the index with
    /// the longest such prefix wins. Inside the exact prefix
    /// `idx:<name>:hex(v1)_…_hex(vk)_` the entries are ordered by the sort
    /// value (the codec is order-preserving and prefix-free), then by
    /// document key, so the read walks that range — backwards for a
    /// descending sort — and stops once `accept` has kept `want` documents.
    ///
    /// Rows with equal sort values come out in key order both ways, as the
    /// stable sort of a key-ordered scan leaves them: a backward read puts
    /// each run of equal values back in key order. That is why the sort field
    /// must be the index's last one.
    ///
    /// `accept` re-checks each document against the whole FILTER: the index
    /// only narrows the candidates, and conditions outside the prefix are
    /// left to it. Returns `None` when no index fits, when every prefix value
    /// is null (such rows are not indexed), when a sort value is an array or
    /// object (the codec orders those as JSON text, not as SORT does) or when
    /// more than `max_scan` entries would be read; the caller then falls back
    /// to the scan.
    pub fn index_sorted_with_prefix(
        &self,
        conditions: &[(String, Value)],
        sort_field: &str,
        ascending: bool,
        want: usize,
        max_scan: usize,
        accept: &mut dyn FnMut(&Document) -> bool,
    ) -> Option<(crate::storage::index::Index, Vec<Document>)> {
        const BATCH: usize = 256;

        let condition_value = |field: &str| {
            conditions
                .iter()
                .find(|(cf, _)| cf == field)
                .map(|(_, v)| v)
        };
        let (index, values) = self
            .get_all_indexes()
            .into_iter()
            .filter(|index| matches!(index.index_type, IndexType::Persistent | IndexType::Hash))
            .filter_map(|index| {
                let (last, leading) = index.fields.split_last()?;
                if last != sort_field {
                    return None;
                }
                let values: Vec<Value> = leading
                    .iter()
                    .map(|f| condition_value(f).cloned())
                    .collect::<Option<_>>()?;
                (!values.is_empty() && index.holds(&values)).then_some((index, values))
            })
            .max_by_key(|(_, values)| values.len())?;

        let prefix = format!(
            "{}{}:{}_",
            IDX_PREFIX,
            index.name,
            Self::encode_index_values(&values)
        );
        let prefix_bytes = prefix.as_bytes();
        let cf = self.ks.handle(&self.db)?;
        let iter = if ascending {
            self.db
                .iterator_ks(&cf, IterMode::From(prefix_bytes, Direction::Forward))
        } else {
            let mut seek_key = prefix_bytes.to_vec();
            seek_key.push(0xFF);
            self.db
                .iterator_ks(&cf, IterMode::From(&seek_key, Direction::Reverse))
        };
        // `<prefix><hex(sort value)>:<doc key>`: the hex part is the tie group.
        let mut entries = iter
            .filter_map(|r| r.ok())
            .take_while(|(k, _)| k.starts_with(prefix_bytes))
            .map(|(k, v)| {
                let rest = &k[prefix_bytes.len()..];
                let group = rest.split(|b| *b == b':').next().unwrap_or(rest).to_vec();
                (group, Self::doc_key(&String::from_utf8_lossy(&v)))
            })
            .peekable();
        let mut ties = std::collections::VecDeque::new();
        let mut keys = std::iter::from_fn(|| {
            if ties.is_empty() {
                let (group, first) = entries.next()?;
                ties.push_back(first);
                while let Some((_, key)) = entries.next_if(|(g, _)| *g == group) {
                    ties.push_back(key);
                }
                if !ascending {
                    ties.make_contiguous().reverse();
                }
            }
            ties.pop_front()
        });

        let mut kept = Vec::new();
        let mut scanned = 0usize;
        while kept.len() < want {
            let batch: Vec<Vec<u8>> = keys.by_ref().take(BATCH).collect();
            if batch.is_empty() {
                break;
            }
            scanned += batch.len();
            if scanned > max_scan {
                return None;
            }
            for bytes in self
                .db
                .multi_get_ks(&cf, batch.iter().map(|k| k.as_slice()))
                .into_iter()
                .filter_map(|r| r.ok())
                .flatten()
            {
                let Ok(doc) = deserialize_doc(&bytes) else {
                    continue;
                };
                if matches!(
                    extract_field_value(&doc.to_value(), sort_field),
                    Value::Array(_) | Value::Object(_)
                ) {
                    return None;
                }
                if accept(&doc) {
                    kept.push(doc);
                }
                if kept.len() == want {
                    break;
                }
            }
        }
        Some((index, kept))
    }

    // ==================== Bloom/Cuckoo Filter Support ====================

    pub(crate) fn get_or_create_bloom_filter(&self, index_name: &str) -> DbResult<BloomFilter> {
        if let Some(filter) = self.bloom_filters.get(index_name) {
            return Ok(filter.clone());
        }

        // Try load from DB
        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .ok_or(DbError::InternalError("CF not found".into()))?;
        let key = format!("{}{}", BLO_IDX_PREFIX, index_name);
        let new_filter = if let Ok(Some(bytes)) = db.get_ks(&cf, key.as_bytes()) {
            // Deserialize bloom filter from bytes
            if let Ok(filter) = serde_json::from_slice::<BloomFilter>(&bytes) {
                filter
            } else {
                BloomFilter::with_num_bits(1024 * 8).expected_items(1000)
            }
        } else {
            BloomFilter::with_num_bits(1024 * 8).expected_items(1000)
        };

        // Insert into cache and return
        self.bloom_filters
            .insert(index_name.to_string(), new_filter.clone());
        Ok(new_filter)
    }

    pub(crate) fn save_bloom_filter(&self, index_name: &str, filter: &BloomFilter) -> DbResult<()> {
        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .ok_or(DbError::InternalError("CF not found".into()))?;
        let key = format!("{}{}", BLO_IDX_PREFIX, index_name);
        let bytes = serde_json::to_vec(filter)?;
        db.put_ks(&cf, key.as_bytes(), &bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        Ok(())
    }

    pub(crate) fn bloom_insert(&self, index_name: &str, item: &str) {
        if let Ok(filter) = self.get_or_create_bloom_filter(index_name) {
            let mut filter = filter;
            filter.insert(item.as_bytes());
            // Update cache
            self.bloom_filters.insert(index_name.to_string(), filter);
        }
    }

    pub fn bloom_check(&self, index_name: &str, item: &str) -> bool {
        if let Ok(filter) = self.get_or_create_bloom_filter(index_name) {
            filter.contains(item.as_bytes())
        } else {
            true // Fallback to true if filter issue
        }
    }

    // Cuckoo filter helpers

    pub(crate) fn preload_cuckoo_filter(&self, index_name: &str) {
        if self.cuckoo_filters.contains_key(index_name) {
            return;
        }

        let db = &self.db;
        if let Some(cf) = self.ks.handle(&self.db) {
            let key = format!("{}{}", CFO_IDX_PREFIX, index_name);
            if let Ok(Some(_bytes)) = db.get_ks(&cf, key.as_bytes()) {
                // CuckooFilter doesn't implement Serialize/Deserialize due to DefaultHasher,
                // so we create a new empty filter instead of deserializing
                // This means bloom filters are rebuilt on restart - acceptable for the use case
                self.cuckoo_filters
                    .insert(index_name.to_string(), CuckooFilter::new());
            } else {
                self.cuckoo_filters
                    .insert(index_name.to_string(), CuckooFilter::new());
            }
        }
    }

    pub(crate) fn save_cuckoo_filter(
        &self,
        index_name: &str,
        _filter: &CuckooFilter<DefaultHasher>,
    ) -> DbResult<()> {
        let _cf = self
            .ks
            .handle(&self.db)
            .ok_or(DbError::InternalError("CF not found".into()))?;
        let _key = format!("{}{}", CFO_IDX_PREFIX, index_name);

        Ok(())
    }

    pub fn cuckoo_insert(&self, index_name: &str, item: &str) {
        self.preload_cuckoo_filter(index_name);
        if let Some(mut filter) = self.cuckoo_filters.get_mut(index_name) {
            let _ = filter.add(item);
        }
    }

    pub fn cuckoo_delete(&self, index_name: &str, item: &str) {
        self.preload_cuckoo_filter(index_name);
        if let Some(mut filter) = self.cuckoo_filters.get_mut(index_name) {
            let _ = filter.delete(item);
        }
    }

    pub fn cuckoo_check(&self, index_name: &str, item: &str) -> bool {
        self.preload_cuckoo_filter(index_name);
        if let Some(filter) = self.cuckoo_filters.get(index_name) {
            filter.contains(item)
        } else {
            true
        }
    }

    // ==================== Index Entry Computation Helpers (for Atomic Batch Operations) ====================

    /// Compute index entries to add for a document insert (without writing to DB)
    /// Returns: (regular_index_entries, geo_index_entries)
    /// Each entry is (key_bytes, value_bytes)
    pub(crate) fn compute_index_entries_for_insert(
        &self,
        doc_key: &str,
        doc_value: &Value,
    ) -> DbResult<(IndexEntries, IndexEntries)> {
        let indexes = self.get_all_indexes();
        let mut regular_entries = Vec::new();

        for index in indexes {
            let field_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(doc_value, f))
                .collect();

            if index.holds(&field_values) {
                for entry_key in Self::index_entry_keys(&index, &field_values, doc_key) {
                    regular_entries.push((entry_key, doc_key.as_bytes().to_vec()));
                }
            }
        }

        // Compute geo index entries
        let geo_indexes = self.get_all_geo_indexes();
        let mut geo_entries = Vec::new();

        for geo_index in &geo_indexes {
            let field_value = extract_field_value(doc_value, &geo_index.field);
            if !field_value.is_null() {
                let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
                let geo_data = serde_json::to_vec(&field_value)?;
                geo_entries.push((entry_key, geo_data));
            }
        }

        Ok((regular_entries, geo_entries))
    }

    /// Compute index entries for a document update (without writing to DB)
    /// Returns: (entries_to_add, keys_to_remove, geo_entries_to_add, geo_keys_to_remove)
    pub(crate) fn compute_index_entries_for_update(
        &self,
        doc_key: &str,
        old_value: &Value,
        new_value: &Value,
    ) -> DbResult<(IndexEntries, IndexKeys, IndexEntries, IndexKeys)> {
        let indexes = self.get_all_indexes();
        let mut entries_to_add = Vec::new();
        let mut keys_to_remove = Vec::new();

        for index in indexes {
            let old_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(old_value, f))
                .collect();
            let new_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(new_value, f))
                .collect();

            // Remove old entry
            if index.holds(&old_values) {
                keys_to_remove.extend(Self::index_entry_keys(&index, &old_values, doc_key));
            }

            // Add new entry
            if index.holds(&new_values) {
                for new_entry_key in Self::index_entry_keys(&index, &new_values, doc_key) {
                    entries_to_add.push((new_entry_key, doc_key.as_bytes().to_vec()));
                }
            }
        }

        // Compute geo index entries
        let geo_indexes = self.get_all_geo_indexes();
        let mut geo_entries_to_add = Vec::new();
        let mut geo_keys_to_remove = Vec::new();

        for geo_index in &geo_indexes {
            let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
            let new_field = extract_field_value(new_value, &geo_index.field);

            // Always remove old geo entry
            geo_keys_to_remove.push(entry_key.clone());

            // Add new if field exists
            if !new_field.is_null() {
                let geo_data = serde_json::to_vec(&new_field)?;
                geo_entries_to_add.push((entry_key, geo_data));
            }
        }

        Ok((
            entries_to_add,
            keys_to_remove,
            geo_entries_to_add,
            geo_keys_to_remove,
        ))
    }

    /// Compute index entries to remove for a document delete (without writing to DB)
    /// Returns: (regular_keys_to_remove, geo_keys_to_remove)
    pub(crate) fn compute_index_entries_for_delete(
        &self,
        doc_key: &str,
        doc_value: &Value,
    ) -> DbResult<(IndexKeys, IndexKeys)> {
        let indexes = self.get_all_indexes();
        let mut keys_to_remove = Vec::new();

        for index in indexes {
            let field_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(doc_value, f))
                .collect();

            if index.holds(&field_values) {
                keys_to_remove.extend(Self::index_entry_keys(&index, &field_values, doc_key));
            }
        }

        // Compute geo index keys to remove
        let geo_indexes = self.get_all_geo_indexes();
        let mut geo_keys_to_remove = Vec::new();

        for geo_index in &geo_indexes {
            let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
            geo_keys_to_remove.push(entry_key);
        }

        Ok((keys_to_remove, geo_keys_to_remove))
    }
}

/// What an equality reads in a single-field index.
enum EqPrefix {
    /// No index serves the field (or the value is null): not answerable here.
    NoIndex,
    /// The index's filter rules the value out.
    Empty,
    /// The entries of the matching documents share this prefix.
    Prefix(Vec<u8>),
}
