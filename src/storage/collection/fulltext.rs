use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::index::{
    extract_field_value, generate_ngrams, levenshtein_distance, tokenize, FulltextMatch,
};
use rust_rocksdb::WriteBatch;
use serde_json::Value;
use std::collections::HashMap;

/// A RocksDB key and value to write.
type KvPair = (Vec<u8>, Vec<u8>);

/// Postings read per query term before giving up on that term (audit P5).
const MAX_POSTINGS_PER_TERM: usize = 50_000;
/// Bounds on how many candidates are loaded and scored per search.
const MIN_SCORED_CANDIDATES: usize = 100;
const MAX_SCORED_CANDIDATES: usize = 5_000;

impl Collection {
    // ==================== Fulltext Index Operations ====================

    /// Get all fulltext indexes
    pub fn get_all_fulltext_indexes(&self) -> Vec<FulltextIndex> {
        // Empty when the column family is gone (dropped mid-operation): a
        // background caller such as the TTL worker must not panic (audit P11).
        self.index_meta()
            .map(|m| m.fulltext.clone())
            .unwrap_or_default()
    }

    /// Get a fulltext index by name
    pub(crate) fn get_fulltext_index(&self, name: &str) -> Option<FulltextIndex> {
        self.index_meta()?
            .fulltext
            .iter()
            .find(|i| i.name == name)
            .cloned()
    }

    /// Get a fulltext index that covers a specific field
    pub fn get_fulltext_index_for_field(&self, field: &str) -> Option<FulltextIndex> {
        let indexes = self.get_all_fulltext_indexes();
        indexes
            .into_iter()
            .find(|idx| idx.fields.contains(&field.to_string()))
    }

    /// Create a fulltext index
    pub fn create_fulltext_index(
        &self,
        name: String,
        fields: Vec<String>,
        min_length: Option<usize>,
    ) -> DbResult<()> {
        let min_length = min_length.unwrap_or_else(default_min_length);

        if self.get_fulltext_index(&name).is_some() {
            return Err(DbError::InvalidDocument(format!(
                "Fulltext Index '{}' already exists",
                name
            )));
        }

        let index = FulltextIndex {
            name: name.clone(),
            fields: fields.clone(),
            min_length,
        };
        let index_bytes = serde_json::to_vec(&index)?;

        // Store metadata
        {
            let db = &self.db;
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(&cf, Self::ft_meta_key(&name), &index_bytes)
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to create fulltext index: {}", e))
                })?;
        }
        self.invalidate_index_meta();

        // Build index
        let docs = self.all();
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let mut batch = WriteBatch::default();
        let mut count = 0;

        for doc in &docs {
            let doc_value = doc.to_value();
            for field in &fields {
                let field_value = extract_field_value(&doc_value, field);
                if let Some(text) = field_value.as_str() {
                    // Index terms
                    let terms = tokenize(text);
                    for term in &terms {
                        if term.len() >= min_length {
                            let term_key = Self::ft_term_key(&name, term, &doc.key);
                            batch.put_cf(&cf, term_key, doc.key.as_bytes());
                        }
                    }
                    // No `ft:` n-gram entries: nothing reads them (audit P5).
                    count += 1;
                }
            }

            if count > 1000 {
                db.write(&batch).map_err(|e| {
                    DbError::InternalError(format!("Failed to build fulltext index: {}", e))
                })?;
                batch = WriteBatch::default();
                count = 0;
            }
        }

        if count > 0 {
            db.write(&batch).map_err(|e| {
                DbError::InternalError(format!("Failed to build fulltext index: {}", e))
            })?;
        }

        Ok(())
    }

    /// Drop a fulltext index
    pub fn drop_fulltext_index(&self, name: &str) -> DbResult<()> {
        if self.get_fulltext_index(name).is_none() {
            return Err(DbError::InvalidDocument(format!(
                "Fulltext Index '{}' not found",
                name
            )));
        }

        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Delete metadata
        db.delete_cf(&cf, Self::ft_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop fulltext index: {}", e)))?;
        self.invalidate_index_meta();

        let mut batch = WriteBatch::default();
        let mut count = 0;

        // Delete ngrams
        let prefix = format!("{}{}:", FT_PREFIX, name);
        let iter = db.prefix_iterator_cf(&cf, prefix.as_bytes());
        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(prefix.as_bytes()) {
                    batch.delete_cf(&cf, key);
                    count += 1;
                } else {
                    break;
                }
            }
            if count > 1000 {
                db.write(&batch).map_err(|e| {
                    DbError::InternalError(format!("Failed to drop fulltext entries: {}", e))
                })?;
                batch = WriteBatch::default();
                count = 0;
            }
        }

        // Delete terms
        let term_prefix = format!("{}{}:", FT_TERM_PREFIX, name);
        let iter = db.prefix_iterator_cf(&cf, term_prefix.as_bytes());
        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(term_prefix.as_bytes()) {
                    batch.delete_cf(&cf, key);
                    count += 1;
                } else {
                    break; // Fixed from original loop which implied break
                }
            }
            if count > 1000 {
                db.write(&batch).map_err(|e| {
                    DbError::InternalError(format!("Failed to drop fulltext entries: {}", e))
                })?;
                batch = WriteBatch::default();
                count = 0;
            }
        }

        if count > 0 {
            db.write(&batch).map_err(|e| {
                DbError::InternalError(format!("Failed to drop fulltext entries: {}", e))
            })?;
        }

        Ok(())
    }

    /// Update fulltext indexes on insert
    #[allow(dead_code)]
    pub(crate) fn update_fulltext_on_insert(
        &self,
        doc_key: &str,
        doc_value: &Value,
    ) -> DbResult<()> {
        let indexes = self.get_all_fulltext_indexes();
        if indexes.is_empty() {
            return Ok(());
        }

        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let mut batch = WriteBatch::default();

        for index in indexes {
            for field in &index.fields {
                let field_value = extract_field_value(doc_value, field);
                if let Some(text) = field_value.as_str() {
                    let terms = tokenize(text);
                    for term in &terms {
                        if term.len() >= index.min_length {
                            let term_key = Self::ft_term_key(&index.name, term, doc_key);
                            batch.put_cf(&cf, term_key, doc_key.as_bytes());
                        }
                    }
                }
            }
        }

        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to update fulltext index: {}", e)))
    }

    /// Update fulltext indexes on delete
    #[allow(dead_code)]
    pub(crate) fn update_fulltext_on_delete(
        &self,
        doc_key: &str,
        doc_value: &Value,
    ) -> DbResult<()> {
        let indexes = self.get_all_fulltext_indexes();
        if indexes.is_empty() {
            return Ok(());
        }

        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let mut batch = WriteBatch::default();

        for index in indexes {
            for field in &index.fields {
                let field_value = extract_field_value(doc_value, field);
                if let Some(text) = field_value.as_str() {
                    let terms = tokenize(text);
                    for term in &terms {
                        if term.len() >= index.min_length {
                            let term_key = Self::ft_term_key(&index.name, term, doc_key);
                            batch.delete_cf(&cf, term_key);
                        }
                    }

                    let ngrams = generate_ngrams(text, NGRAM_SIZE);
                    for ngram in &ngrams {
                        let ngram_key = Self::ft_ngram_key(&index.name, ngram, doc_key);
                        batch.delete_cf(&cf, ngram_key);
                    }
                }
            }
        }

        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to update fulltext index: {}", e)))
    }

    /// List fulltext indexes
    pub fn list_fulltext_indexes(&self) -> Vec<FulltextIndex> {
        self.get_all_fulltext_indexes()
    }

    /// Perform a fulltext search
    pub fn fulltext_search(
        &self,
        query: &str,
        fields: Option<Vec<String>>,
        limit: usize,
    ) -> DbResult<Vec<FulltextMatch>> {
        // 1. Identify relevant indexes
        let all_indexes = self.get_all_fulltext_indexes();
        let indexes: Vec<&FulltextIndex> = if let Some(target_fields) = &fields {
            all_indexes
                .iter()
                .filter(|idx| idx.fields.iter().any(|f| target_fields.contains(f)))
                .collect()
        } else {
            all_indexes.iter().collect()
        };

        if indexes.is_empty() {
            return Ok(Vec::new());
        }

        // 2. Tokenize query
        let query_terms = tokenize(query);
        if query_terms.is_empty() {
            return Ok(Vec::new());
        }

        // 3. Collect candidate documents (using term matching first)
        let mut candidate_counts: HashMap<String, usize> = HashMap::new();
        let db = &self.db;
        let Some(cf) = db.cf_handle(&self.name) else {
            return Ok(Vec::new()); // column family dropped mid-operation
        };

        for index in &indexes {
            for term in &query_terms {
                if term.len() >= index.min_length {
                    // Exact term lookup
                    let prefix = format!("{}{}:{}:", FT_TERM_PREFIX, index.name, term);
                    let iter = db.prefix_iterator_cf(&cf, prefix.as_bytes());

                    for (scanned, result) in iter.flatten().enumerate() {
                        let (key, _) = result;
                        if !key.starts_with(prefix.as_bytes()) || scanned >= MAX_POSTINGS_PER_TERM {
                            break;
                        }
                        // Key is "ft_term:<index>:<term>:<doc_key>". Terms are
                        // alphanumeric, so everything after the exact prefix is
                        // the doc key — which may itself contain ':' (audit D6;
                        // `split(':').last()` truncated such keys).
                        let doc_key = String::from_utf8_lossy(&key[prefix.len()..]).into_owned();
                        *candidate_counts.entry(doc_key).or_insert(0) += 1;
                    }
                }
            }

            // Fuzzy lookup (trigrams) if strict term matching yielded few results?
            // Or always? A proper implementation combines both.
            // For now, let's keep it simple: if strict terms found candidates, score them.
            // If not, maybe use ngrams?
            // The original implementation might have been more complex.
            // We'll proceed with term matching + Levenshtein re-scoring.
        }

        // 4. Score only the most promising candidates (audit P5): scoring
        // loads the document and runs Levenshtein over its terms, so doing it
        // for every posting before applying `limit` scaled with the corpus.
        // Rank by how many query terms matched; score the top N.
        let max_scored = limit
            .saturating_mul(4)
            .clamp(MIN_SCORED_CANDIDATES, MAX_SCORED_CANDIDATES);
        let mut ranked: Vec<(String, usize)> = candidate_counts.into_iter().collect();
        if ranked.len() > max_scored {
            ranked.select_nth_unstable_by(max_scored - 1, |a, b| b.1.cmp(&a.1));
            ranked.truncate(max_scored);
        }

        let mut matches = Vec::new();
        for (doc_key, _count) in ranked {
            // Retrieve document to calculate exact score
            // Optimization: Only load full document if count is promising?
            // Here we assume if it matches term, it's relevant.

            if let Ok(doc) = self.get(&doc_key) {
                let doc_value = doc.to_value();
                let mut best_score = 0;
                let mut valid = false;

                for index in &indexes {
                    for field in &index.fields {
                        if let Some(fields_filter) = &fields {
                            if !fields_filter.contains(field) {
                                continue;
                            }
                        }

                        let field_value = extract_field_value(&doc_value, field);
                        if let Some(text) = field_value.as_str() {
                            // Basic scoring: (matches / total_terms) * 100
                            // Minus Levenshtein penalty
                            // This is a simplified version of likely original logic

                            let doc_terms = tokenize(text);
                            let mut field_score = 0;

                            for q_term in &query_terms {
                                for d_term in &doc_terms {
                                    let dist = levenshtein_distance(q_term, d_term);
                                    if dist == 0 {
                                        field_score += 10; // Exact match
                                    } else if dist <= 2 {
                                        field_score += 5; // Fuzzy match
                                    }
                                }
                            }

                            if field_score > best_score {
                                best_score = field_score;
                                valid = true;
                            }
                        }
                    }
                }

                if valid {
                    matches.push(FulltextMatch {
                        doc_key: doc_key.to_string(),
                        score: best_score as f64,
                        matched_terms: Vec::new(), // Populate if needed or change logic to track terms
                    });
                }
            }
        }

        // 5. Sort and limit
        matches.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        matches.truncate(limit);

        Ok(matches)
    }

    // ==================== Fulltext Index Entry Computation Helpers ====================

    /// Indexed terms of one field value, deduplicated.
    fn ft_terms(index: &FulltextIndex, text: &str) -> std::collections::BTreeSet<String> {
        tokenize(text)
            .into_iter()
            .filter(|t| t.len() >= index.min_length)
            .collect()
    }

    /// Legacy `ft:` n-gram keys for one field value, only if this document
    /// was indexed by a version that still wrote them. One point read on the
    /// first n-gram decides, so documents indexed since P5 pay nothing.
    fn legacy_ngram_keys(&self, index: &FulltextIndex, text: &str, doc_key: &str) -> Vec<Vec<u8>> {
        let ngrams = generate_ngrams(text, NGRAM_SIZE);
        let Some(first) = ngrams.first() else {
            return Vec::new();
        };
        let Some(cf) = self.db.cf_handle(&self.name) else {
            return Vec::new();
        };
        let probe = Self::ft_ngram_key(&index.name, first, doc_key);
        if !matches!(self.db.get_pinned_cf(&cf, &probe), Ok(Some(_))) {
            return Vec::new();
        }
        ngrams
            .iter()
            .map(|ngram| Self::ft_ngram_key(&index.name, ngram, doc_key))
            .collect()
    }

    /// Compute fulltext index entries to add for a document insert (without writing to DB)
    /// Returns Vec<(key_bytes, value_bytes)> where value is typically doc_key
    pub(crate) fn compute_fulltext_entries_for_insert(
        &self,
        doc_key: &str,
        doc_value: &Value,
    ) -> Vec<(Vec<u8>, Vec<u8>)> {
        let indexes = self.get_all_fulltext_indexes();
        if indexes.is_empty() {
            return Vec::new();
        }

        let mut entries = Vec::new();
        let doc_key_bytes = doc_key.as_bytes().to_vec();

        for index in &indexes {
            for field in &index.fields {
                let field_value = extract_field_value(doc_value, field);
                if let Some(text) = field_value.as_str() {
                    for term in Self::ft_terms(index, text) {
                        let term_key = Self::ft_term_key(&index.name, &term, doc_key);
                        entries.push((term_key, doc_key_bytes.clone()));
                    }
                }
            }
        }

        entries
    }

    /// Compute fulltext index entries to remove for a document delete (without writing to DB)
    /// Returns Vec<key_bytes> for entries to delete
    pub(crate) fn compute_fulltext_entries_for_delete(
        &self,
        doc_key: &str,
        doc_value: &Value,
    ) -> Vec<Vec<u8>> {
        let indexes = self.get_all_fulltext_indexes();
        if indexes.is_empty() {
            return Vec::new();
        }

        let mut keys_to_remove = Vec::new();

        for index in &indexes {
            for field in &index.fields {
                let field_value = extract_field_value(doc_value, field);
                if let Some(text) = field_value.as_str() {
                    for term in Self::ft_terms(index, text) {
                        keys_to_remove.push(Self::ft_term_key(&index.name, &term, doc_key));
                    }
                    keys_to_remove.extend(self.legacy_ngram_keys(index, text, doc_key));
                }
            }
        }

        keys_to_remove
    }

    /// Fulltext changes for a document update: `(entries_to_add, keys_to_remove)`.
    ///
    /// Fields whose text is unchanged produce nothing (audit P5: an update
    /// that did not touch the indexed text used to delete and rewrite every
    /// term), and changed fields only add/remove the terms that differ.
    /// Callers must apply the removals before the additions.
    pub(crate) fn compute_fulltext_entries_for_update(
        &self,
        doc_key: &str,
        old_value: &Value,
        new_value: &Value,
    ) -> (Vec<KvPair>, Vec<Vec<u8>>) {
        let indexes = self.get_all_fulltext_indexes();
        if indexes.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let mut to_add = Vec::new();
        let mut to_remove = Vec::new();
        let doc_key_bytes = doc_key.as_bytes().to_vec();

        for index in &indexes {
            for field in &index.fields {
                let old_field = extract_field_value(old_value, field);
                let new_field = extract_field_value(new_value, field);
                let old_text = old_field.as_str();
                let new_text = new_field.as_str();
                if old_text == new_text {
                    continue;
                }

                let old_terms = old_text
                    .map(|t| Self::ft_terms(index, t))
                    .unwrap_or_default();
                let new_terms = new_text
                    .map(|t| Self::ft_terms(index, t))
                    .unwrap_or_default();

                for term in old_terms.difference(&new_terms) {
                    to_remove.push(Self::ft_term_key(&index.name, term, doc_key));
                }
                if let Some(text) = old_text {
                    to_remove.extend(self.legacy_ngram_keys(index, text, doc_key));
                }
                for term in new_terms.difference(&old_terms) {
                    to_add.push((
                        Self::ft_term_key(&index.name, term, doc_key),
                        doc_key_bytes.clone(),
                    ));
                }
            }
        }

        (to_add, to_remove)
    }
}
