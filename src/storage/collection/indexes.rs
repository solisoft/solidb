use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::index::{extract_field_value, generate_ngrams, tokenize};
use rocksdb::{Direction, IteratorMode, WriteBatch};
use serde_json::Value;

use hex;

impl Collection {
    // ==================== Index Operations ====================

    /// Get all index metadata
    pub fn get_all_indexes(&self) -> Vec<Index> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let prefix = IDX_META_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);

        iter.filter_map(|result| {
            result.ok().and_then(|(key, value)| {
                if key.starts_with(prefix) {
                    serde_json::from_slice(&value).ok()
                } else {
                    None
                }
            })
        })
        .collect()
    }

    /// Get an index by name
    pub(crate) fn get_index(&self, name: &str) -> Option<Index> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        db.get_cf(cf, Self::idx_meta_key(name))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Create an index on a field
    pub fn create_index(
        &self,
        name: String,
        fields: Vec<String>,
        index_type: IndexType,
        unique: bool,
    ) -> DbResult<IndexStats> {
        // Check if index already exists
        if self.get_index(&name).is_some() {
            return Err(DbError::InvalidDocument(format!(
                "Index '{}' already exists",
                name
            )));
        }

        // Create index metadata
        let index = Index::new(name.clone(), fields.clone(), index_type.clone(), unique);
        let index_bytes = serde_json::to_vec(&index)?;

        // Store index metadata and build index
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::idx_meta_key(&name), &index_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to create index: {}", e)))?;
        }

        // Build index from existing documents
        let docs = self.all();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for doc in &docs {
            let doc_value = doc.to_value();
            let field_values: Vec<Value> = fields
                .iter()
                .map(|f| extract_field_value(&doc_value, f))
                .collect();

            if !field_values.iter().all(|v| v.is_null()) {
                let entry_key = Self::idx_entry_key(&name, &field_values, &doc.key);
                db.put_cf(cf, entry_key, doc.key.as_bytes())
                    .map_err(|e| DbError::InternalError(format!("Failed to build index: {}", e)))?;

                // If bloom/cuckoo filter, also update in-memory filter
                if index_type == IndexType::Bloom {
                    for value in &field_values {
                        self.bloom_insert(&name, &value.to_string());
                    }
                } else if index_type == IndexType::Cuckoo {
                    for value in &field_values {
                        self.cuckoo_insert(&name, &value.to_string());
                    }
                }
            }
        }

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
            unique_values: docs.len(), // Approximation
            indexed_documents: docs.len(),
        })
    }

    /// Drop an index
    pub fn drop_index(&self, name: &str) -> DbResult<()> {
        if self.get_index(name).is_none() {
            return Err(DbError::InvalidDocument(format!(
                "Index '{}' not found",
                name
            )));
        }

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Delete index metadata
        db.delete_cf(cf, Self::idx_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop index: {}", e)))?;

        // Delete all index entries
        let prefix = format!("{}{}:", IDX_PREFIX, name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        for result in iter.flatten() {
            let (key, _) = result;
            if key.starts_with(prefix.as_bytes()) {
                db.delete_cf(cf, &key).map_err(|e| {
                    DbError::InternalError(format!("Failed to drop index entry: {}", e))
                })?;
            } else {
                break;
            }
        }

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
            });
        }

        stats
    }

    /// Rebuild all indexes from existing documents
    /// Call this after bulk imports using insert_no_index()
    pub fn rebuild_all_indexes(&self) -> DbResult<usize> {
        let total_start = std::time::Instant::now();

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

        // Clear existing index entries using WriteBatch
        let clear_start = std::time::Instant::now();
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            let mut batch = WriteBatch::default();

            // Clear regular indexes
            for index in &indexes {
                let prefix = format!("{}{}:", IDX_PREFIX, index.name);
                let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());
                for result in iter.flatten() {
                    let (key, _) = result;
                    if key.starts_with(prefix.as_bytes()) {
                        batch.delete_cf(cf, &key);
                    } else {
                        break;
                    }
                }
            }

            // Clear geo indexes
            for geo_index in &geo_indexes {
                let prefix = format!("{}{}:", GEO_PREFIX, geo_index.name);
                let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());
                for result in iter.flatten() {
                    let (key, _) = result;
                    if key.starts_with(prefix.as_bytes()) {
                        batch.delete_cf(cf, &key);
                    } else {
                        break;
                    }
                }
            }

            // Clear fulltext indexes
            for ft_index in &ft_indexes {
                let ngram_prefix = format!("{}{}:", FT_PREFIX, ft_index.name);
                let iter = db.prefix_iterator_cf(cf, ngram_prefix.as_bytes());
                for result in iter.flatten() {
                    let (key, _) = result;
                    if key.starts_with(ngram_prefix.as_bytes()) {
                        batch.delete_cf(cf, &key);
                    } else {
                        break;
                    }
                }

                let term_prefix = format!("{}{}:", FT_TERM_PREFIX, ft_index.name);
                let iter = db.prefix_iterator_cf(cf, term_prefix.as_bytes());
                for result in iter.flatten() {
                    let (key, _) = result;
                    if key.starts_with(term_prefix.as_bytes()) {
                        batch.delete_cf(cf, &key);
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

        // Load all documents
        let load_start = std::time::Instant::now();
        let docs = self.all();
        let doc_count = docs.len();
        tracing::info!(
            "rebuild_all_indexes: Loaded {} docs in {:?}",
            doc_count,
            load_start.elapsed()
        );

        // Rebuild regular indexes
        if !indexes.is_empty() {
            let idx_start = std::time::Instant::now();
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            let mut batch = WriteBatch::default();

            for doc in &docs {
                let doc_value = doc.to_value();
                for index in &indexes {
                    let field_values: Vec<Value> = index
                        .fields
                        .iter()
                        .map(|f| extract_field_value(&doc_value, f))
                        .collect();

                    if !field_values.iter().all(|v| v.is_null()) {
                        let entry_key = Self::idx_entry_key(&index.name, &field_values, &doc.key);
                        batch.put_cf(cf, entry_key, doc.key.as_bytes());
                    }
                }
            }

            let _ = db.write(batch);
            tracing::info!(
                "rebuild_all_indexes: Regular indexes took {:?}",
                idx_start.elapsed()
            );
        }

        // Rebuild geo indexes
        if !geo_indexes.is_empty() {
            let geo_start = std::time::Instant::now();
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            let mut batch = WriteBatch::default();

            for doc in &docs {
                let doc_value = doc.to_value();
                for geo_index in &geo_indexes {
                    let field_value = extract_field_value(&doc_value, &geo_index.field);
                    if !field_value.is_null() {
                        let entry_key = Self::geo_entry_key(&geo_index.name, &doc.key);
                        if let Ok(geo_data) = serde_json::to_vec(&field_value) {
                            batch.put_cf(cf, entry_key, &geo_data);
                        }
                    }
                }
            }

            let _ = db.write(batch);
            tracing::info!(
                "rebuild_all_indexes: Geo indexes took {:?}",
                geo_start.elapsed()
            );
        }

        // Rebuild fulltext indexes
        if !ft_indexes.is_empty() {
            let ft_start = std::time::Instant::now();
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            let mut batch = WriteBatch::default();

            for doc in &docs {
                let doc_value = doc.to_value();
                for ft_index in &ft_indexes {
                    for field in &ft_index.fields {
                        let field_value = extract_field_value(&doc_value, field);
                        if let Some(text) = field_value.as_str() {
                            let terms = tokenize(text);
                            for term in &terms {
                                if term.len() >= ft_index.min_length {
                                    let term_key =
                                        Self::ft_term_key(&ft_index.name, term, &doc.key);
                                    batch.put_cf(cf, term_key, doc.key.as_bytes());
                                }
                            }

                            let ngrams = generate_ngrams(text, NGRAM_SIZE);
                            for ngram in &ngrams {
                                let ngram_key = Self::ft_ngram_key(&ft_index.name, ngram, &doc.key);
                                batch.put_cf(cf, ngram_key, doc.key.as_bytes());
                            }
                        }
                    }
                }
            }

            let _ = db.write(batch);
            tracing::info!(
                "rebuild_all_indexes: Fulltext indexes took {:?}",
                ft_start.elapsed()
            );
        }

        // Rebuild vector indexes
        let vector_configs = self.get_all_vector_index_configs();
        if !vector_configs.is_empty() {
            let vec_start = std::time::Instant::now();

            // Clear all vector indexes
            for entry in self.vector_indexes.iter() {
                entry.clear();
            }

            // Re-index all documents
            for doc in &docs {
                let doc_value = doc.to_value();
                self.update_vector_indexes_on_upsert(&doc.key, &doc_value);
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

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Build regular indexes
        if !indexes.is_empty() {
            let mut batch = WriteBatch::default();
            for doc in docs {
                let doc_value = doc.to_value();
                for index in &indexes {
                    let field_values: Vec<Value> = index
                        .fields
                        .iter()
                        .map(|f| extract_field_value(&doc_value, f))
                        .collect();

                    if !field_values.iter().all(|v| v.is_null()) {
                        let entry_key = Self::idx_entry_key(&index.name, &field_values, &doc.key);
                        batch.put_cf(cf, entry_key, doc.key.as_bytes());

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
            let mut batch = WriteBatch::default();
            for doc in docs {
                let doc_value = doc.to_value();
                for geo_index in &geo_indexes {
                    let field_value = extract_field_value(&doc_value, &geo_index.field);
                    if !field_value.is_null() {
                        let entry_key = Self::geo_entry_key(&geo_index.name, &doc.key);
                        if let Ok(geo_data) = serde_json::to_vec(&field_value) {
                            batch.put_cf(cf, entry_key, &geo_data);
                        }
                    }
                }
            }
            let _ = db.write(batch);
        }

        // Build fulltext indexes
        if !ft_indexes.is_empty() {
            let mut batch = WriteBatch::default();
            for doc in docs {
                let doc_value = doc.to_value();
                for ft_index in &ft_indexes {
                    for field in &ft_index.fields {
                        let field_value = extract_field_value(&doc_value, field);
                        if let Some(text) = field_value.as_str() {
                            let terms = tokenize(text);
                            for term in &terms {
                                if term.len() >= ft_index.min_length {
                                    let term_key =
                                        Self::ft_term_key(&ft_index.name, term, &doc.key);
                                    batch.put_cf(cf, term_key, doc.key.as_bytes());
                                }
                            }

                            let ngrams = generate_ngrams(text, NGRAM_SIZE);
                            for ngram in &ngrams {
                                let ngram_key = Self::ft_ngram_key(&ft_index.name, ngram, &doc.key);
                                batch.put_cf(cf, ngram_key, doc.key.as_bytes());
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

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        // Count entries
        let prefix = format!("{}{}:", IDX_PREFIX, name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());
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
        })
    }

    /// Check unique constraints before inserting/updating a document
    pub(crate) fn check_unique_constraints(
        &self,
        doc_key: &str,
        doc_value: &Value,
    ) -> DbResult<()> {
        let indexes = self.get_all_indexes();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for index in indexes {
            if index.unique {
                // For compound indexes, extract all field values
                let field_values: Vec<Value> = index
                    .fields
                    .iter()
                    .map(|f| extract_field_value(doc_value, f))
                    .collect();

                // Skip if all values are null
                if field_values.iter().all(|v| v.is_null()) {
                    continue;
                }

                // Construct a prefix that matches the index entry key up to the values
                // Uses custom key encoding
                let encoded_values: Vec<String> = field_values
                    .iter()
                    .map(|v| hex::encode(crate::storage::codec::encode_key(v)))
                    .collect();
                let value_part = encoded_values.join("_");
                // Key format: idx:<index_name>:<value_part>:<doc_key>
                // We want to check if ANY doc_key exists for this (index_name, value_part) combo
                let prefix = format!("{}{}:{}:", IDX_PREFIX, index.name, value_part);
                let mut iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

                // Check if any OTHER document already has this value
                if let Some(Ok((key, value))) = iter.next() {
                    if key.starts_with(prefix.as_bytes()) {
                        let existing_key = String::from_utf8_lossy(&value); // Value in index is doc_key
                                                                            // Allow update of the same document
                        if existing_key != doc_key {
                            return Err(DbError::InvalidDocument(format!(
                                "Unique constraint violated: fields '{:?}' with value {:?} already exists in index '{}'",
                                index.fields, field_values, index.name
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Update indexes on document insert
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

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for index in indexes {
            let field_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(doc_value, f))
                .collect();

            if !field_values.iter().all(|v| v.is_null()) {
                let entry_key = Self::idx_entry_key(&index.name, &field_values, doc_key);
                db.put_cf(cf, entry_key, doc_key.as_bytes()).map_err(|e| {
                    DbError::InternalError(format!("Failed to update index: {}", e))
                })?;

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
        drop(db);

        // Update geo indexes
        let geo_indexes = self.get_all_geo_indexes();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for geo_index in geo_indexes {
            let field_value = extract_field_value(doc_value, &geo_index.field);
            if !field_value.is_null() {
                let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
                let geo_data = serde_json::to_vec(&field_value)?;
                db.put_cf(cf, entry_key, &geo_data).map_err(|e| {
                    DbError::InternalError(format!("Failed to update geo index: {}", e))
                })?;
            }
        }

        Ok(())
    }

    /// Update indexes on document update
    pub(crate) fn update_indexes_on_update(
        &self,
        doc_key: &str,
        old_value: &Value,
        new_value: &Value,
    ) -> DbResult<()> {
        let indexes = self.get_all_indexes();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
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
            if !old_values.iter().all(|v| v.is_null()) {
                let old_entry_key = Self::idx_entry_key(&index.name, &old_values, doc_key);
                db.delete_cf(cf, old_entry_key).map_err(|e| {
                    DbError::InternalError(format!("Failed to update index: {}", e))
                })?;
            }

            // Add new entry
            if !new_values.iter().all(|v| v.is_null()) {
                let new_entry_key = Self::idx_entry_key(&index.name, &new_values, doc_key);
                db.put_cf(cf, new_entry_key, doc_key.as_bytes())
                    .map_err(|e| {
                        DbError::InternalError(format!("Failed to update index: {}", e))
                    })?;
            }
        }
        drop(db);

        // Update geo indexes
        let geo_indexes = self.get_all_geo_indexes();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for geo_index in geo_indexes {
            let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
            let new_field = extract_field_value(new_value, &geo_index.field);

            if !new_field.is_null() {
                let geo_data = serde_json::to_vec(&new_field)?;
                db.put_cf(cf, entry_key, &geo_data).map_err(|e| {
                    DbError::InternalError(format!("Failed to update geo index: {}", e))
                })?;
            } else {
                db.delete_cf(cf, entry_key).map_err(|e| {
                    DbError::InternalError(format!("Failed to update geo index: {}", e))
                })?;
            }
        }

        Ok(())
    }

    /// Update indexes on document delete
    pub(crate) fn update_indexes_on_delete(
        &self,
        doc_key: &str,
        doc_value: &Value,
    ) -> DbResult<()> {
        let indexes = self.get_all_indexes();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for index in indexes {
            let field_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(doc_value, f))
                .collect();

            if !field_values.iter().all(|v| v.is_null()) {
                let entry_key = Self::idx_entry_key(&index.name, &field_values, doc_key);
                db.delete_cf(cf, entry_key).map_err(|e| {
                    DbError::InternalError(format!("Failed to update index: {}", e))
                })?;
            }
        }
        drop(db);

        // Update geo indexes
        let geo_indexes = self.get_all_geo_indexes();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for geo_index in geo_indexes {
            let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
            db.delete_cf(cf, entry_key).map_err(|e| {
                DbError::InternalError(format!("Failed to update geo index: {}", e))
            })?;
        }

        Ok(())
    }

    /// Get an index for a field
    pub fn get_index_for_field(&self, field: &str) -> Option<Index> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        // Try to find index by checking idx_meta entries
        let prefix = IDX_META_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);

        for result in iter.flatten() {
            let (key, value) = result;
            if !key.starts_with(prefix) {
                break;
            }
            if let Ok(index) = serde_json::from_slice::<Index>(&value) {
                // Check if field is the first field in the index
                if index.fields.first().map(|s| s.as_str()) == Some(field) {
                    return Some(index);
                }
            }
        }
        None
    }

    /// Lookup documents where field > value
    pub fn index_lookup_gt(&self, field: &str, value: &Value) -> Option<Vec<Document>> {
        self.index_range_scan(field, value, false, true)
    }

    /// Lookup documents where field >= value
    pub fn index_lookup_gte(&self, field: &str, value: &Value) -> Option<Vec<Document>> {
        self.index_range_scan(field, value, true, true)
    }

    /// Lookup documents where field < value
    pub fn index_lookup_lt(&self, field: &str, value: &Value) -> Option<Vec<Document>> {
        self.index_range_scan(field, value, false, false)
    }

    /// Lookup documents where field <= value
    pub fn index_lookup_lte(&self, field: &str, value: &Value) -> Option<Vec<Document>> {
        self.index_range_scan(field, value, true, false)
    }

    fn index_range_scan(
        &self,
        field: &str,
        value: &Value,
        inclusive: bool,
        forward: bool,
    ) -> Option<Vec<Document>> {
        let index = self.get_index_for_field(field)?;
        let index_name = &index.name;
        let value_key = crate::storage::codec::encode_key(value);
        let value_str = hex::encode(value_key);

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        let prefix_base = format!("{}{}:", IDX_PREFIX, index_name);
        let seek_key = format!("{}{}:", prefix_base, value_str);

        let mut doc_keys = Vec::new();

        if forward {
            let mode = IteratorMode::From(seek_key.as_bytes(), Direction::Forward);
            let iter = db.iterator_cf(cf, mode);

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
                if doc_keys.len() >= 1000 {
                    break;
                } // Safety limit
            }
        } else {
            // For reverse, we might land ON the key or BEFORE it.
            // If we land on it, it matches value.
            let mode = IteratorMode::From(seek_key.as_bytes(), Direction::Reverse);
            let iter = db.iterator_cf(cf, mode);

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
                if doc_keys.len() >= 1000 {
                    break;
                }
            }
        }

        if doc_keys.is_empty() {
            return Some(Vec::new());
        }

        let results = db.multi_get_cf(doc_keys.iter().map(|k| (cf, k.as_slice())));
        let docs: Vec<Document> = results
            .into_iter()
            .filter_map(|r| r.ok())
            .flatten()
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect();

        Some(docs)
    }

    /// Lookup documents using index (equality)
    pub fn index_lookup_eq(&self, field: &str, value: &Value) -> Option<Vec<Document>> {
        let index = self.get_index_for_field(field)?;

        // Fast-path: Check Bloom/Cuckoo Filter if available
        if (index.index_type == IndexType::Bloom
            && !self.bloom_check(&index.name, &value.to_string()))
            || (index.index_type == IndexType::Cuckoo
                && !self.cuckoo_check(&index.name, &value.to_string()))
        {
            return Some(Vec::new());
        }

        let value_str = hex::encode(crate::storage::codec::encode_key(value));

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        // Prefix for the lookup
        let prefix = format!("{}{}:{}:", IDX_PREFIX, index.name, value_str);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

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
        let results = db.multi_get_cf(doc_keys.iter().map(|k| (cf, k.as_slice())));

        let docs: Vec<Document> = results
            .into_iter()
            .filter_map(|r| r.ok())
            .flatten()
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
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
        let index = self.get_index_for_field(field)?;
        // Skip bloom check for limit query? No, same logic.
        // It's just a limit.

        let value_str = hex::encode(crate::storage::codec::encode_key(value));

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        let prefix = format!("{}{}:{}:", IDX_PREFIX, index.name, value_str);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

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

        let results = db.multi_get_cf(doc_keys.iter().map(|k| (cf, k.as_slice())));
        let docs: Vec<Document> = results
            .into_iter()
            .filter_map(|r| r.ok())
            .flatten()
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
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
            let db = self.db.read().unwrap();
            let cf = db.cf_handle(&self.name)?;
            let prefix = DOC_PREFIX.as_bytes();

            let iter = if ascending {
                let mode = IteratorMode::From(prefix, Direction::Forward);
                db.iterator_cf(cf, mode)
            } else {
                // For descending, we seek past the end of the prefix
                let mut seek_key = prefix.to_vec();
                seek_key.push(0xFF);
                let mode = IteratorMode::From(&seek_key, Direction::Reverse);
                db.iterator_cf(cf, mode)
            };

            let docs: Vec<Document> = iter
                .filter_map(|r| r.ok())
                .take_while(|(k, _)| k.starts_with(prefix))
                .filter_map(|(_, v)| serde_json::from_slice::<Document>(&v).ok())
                .take(limit.unwrap_or(usize::MAX))
                .collect();

            return Some(docs);
        }

        let index = self.get_index_for_field(field)?;
        let index_name = index.name.clone();
        let prefix = format!("{}{}:", IDX_PREFIX, index_name);

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        let prefix_bytes = prefix.as_bytes();

        let iter = if ascending {
            let mode = IteratorMode::From(prefix_bytes, Direction::Forward);
            db.iterator_cf(cf, mode)
        } else {
            let mut seek_key = prefix.as_bytes().to_vec();
            seek_key.push(0xFF);
            let mode = IteratorMode::From(&seek_key, Direction::Reverse);
            db.iterator_cf(cf, mode)
        };

        let doc_keys: Vec<String> = iter
            .filter_map(|r| r.ok())
            .take_while(|(k, _)| k.starts_with(prefix_bytes))
            .map(|(_, v)| String::from_utf8_lossy(&v).to_string())
            .take(limit.unwrap_or(usize::MAX))
            .collect();

        drop(db);

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

    // ==================== Bloom/Cuckoo Filter Support ====================

    pub(crate) fn get_or_create_bloom_filter(&self, index_name: &str) -> DbResult<BloomFilter> {
        if let Some(filter) = self.bloom_filters.get(index_name) {
            return Ok(filter.clone());
        }

        // Try load from DB
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .ok_or(DbError::InternalError("CF not found".into()))?;
        let key = format!("{}{}", BLO_IDX_PREFIX, index_name);
        let new_filter = if let Ok(Some(bytes)) = db.get_cf(cf, key.as_bytes()) {
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
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .ok_or(DbError::InternalError("CF not found".into()))?;
        let key = format!("{}{}", BLO_IDX_PREFIX, index_name);
        let bytes = serde_json::to_vec(filter)?;
        db.put_cf(cf, key.as_bytes(), &bytes)
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

        let db = self.db.read().unwrap();
        if let Some(cf) = db.cf_handle(&self.name) {
            let key = format!("{}{}", CFO_IDX_PREFIX, index_name);
            if let Ok(Some(_bytes)) = db.get_cf(cf, key.as_bytes()) {
                // FIXME: CuckooFilter deserialization failing due to DefaultHasher
                // if let Ok(filter) = serde_json::from_slice(&bytes) {
                //     self.cuckoo_filters.insert(index_name.to_string(), filter);
                // }
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
        let db = self.db.read().unwrap();
        let _cf = db
            .cf_handle(&self.name)
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
}
