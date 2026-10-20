use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::index::{
    calculate_idf, extract_field_value, find_phrase, fuzzy_edits_for, generate_ngrams,
    levenshtein_distance, tokenize_folded, tokenize_spans, FulltextMatch, BM25_B, BM25_K1,
};
use crate::storage::kv::KvBatch;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

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

    /// Create a fulltext index (terms not accent-folded).
    pub fn create_fulltext_index(
        &self,
        name: String,
        fields: Vec<String>,
        min_length: Option<usize>,
    ) -> DbResult<()> {
        self.create_fulltext_index_with(name, fields, min_length, false)
    }

    /// Create a fulltext index; with `fold_accents`, terms are stored and
    /// searched accent-folded.
    pub fn create_fulltext_index_with(
        &self,
        name: String,
        fields: Vec<String>,
        min_length: Option<usize>,
        fold_accents: bool,
    ) -> DbResult<()> {
        let min_length = min_length.unwrap_or_else(default_min_length);

        if self.get_fulltext_index(&name).is_some() {
            return Err(DbError::InvalidDocument(format!(
                "Fulltext Index '{}' already exists",
                name
            )));
        }

        // Whatever the outcome below, from here on cached results may have
        // been computed against a half-built or half-removed index.
        let _invalidate = self.invalidate_cached_queries_on_exit();

        let index = FulltextIndex {
            name: name.clone(),
            fields: fields.clone(),
            min_length,
            fold_accents,
        };
        let index_bytes = serde_json::to_vec(&index)?;

        // Store metadata
        {
            let db = &self.db;
            let cf = self
                .ks
                .handle(&self.db)
                .expect("Column family should exist");
            db.put_ks(&cf, Self::ft_meta_key(&name), &index_bytes)
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to create fulltext index: {}", e))
                })?;
        }
        self.invalidate_index_meta();

        // Build index
        let docs = self.all();
        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        let mut batch = KvBatch::default();
        let mut count = 0;

        for doc in &docs {
            let doc_value = doc.to_value();
            for field in &fields {
                let field_value = extract_field_value(&doc_value, field);
                if let Some(text) = field_value.as_str() {
                    for term in &Self::ft_terms(&index, text) {
                        let term_key = Self::ft_term_key(&name, term, &doc.key);
                        batch.put_ks(&cf, term_key, doc.key.as_bytes());
                    }
                    // No `ft:` n-gram entries: nothing reads them (audit P5).
                    count += 1;
                }
            }

            if count > 1000 {
                db.write(batch).map_err(|e| {
                    DbError::InternalError(format!("Failed to build fulltext index: {}", e))
                })?;
                batch = KvBatch::default();
                count = 0;
            }
        }

        if count > 0 {
            db.write(batch).map_err(|e| {
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

        let _invalidate = self.invalidate_cached_queries_on_exit();

        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        // Delete metadata
        db.delete_ks(&cf, Self::ft_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop fulltext index: {}", e)))?;
        self.invalidate_index_meta();

        let mut batch = KvBatch::default();
        let mut count = 0;

        // Delete ngrams
        let prefix = format!("{}{}:", FT_PREFIX, name);
        let iter = db.prefix_iterator_ks(&cf, prefix.as_bytes());
        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(prefix.as_bytes()) {
                    batch.delete_ks(&cf, key);
                    count += 1;
                } else {
                    break;
                }
            }
            if count > 1000 {
                db.write(batch).map_err(|e| {
                    DbError::InternalError(format!("Failed to drop fulltext entries: {}", e))
                })?;
                batch = KvBatch::default();
                count = 0;
            }
        }

        // Delete terms
        let term_prefix = format!("{}{}:", FT_TERM_PREFIX, name);
        let iter = db.prefix_iterator_ks(&cf, term_prefix.as_bytes());
        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(term_prefix.as_bytes()) {
                    batch.delete_ks(&cf, key);
                    count += 1;
                } else {
                    break; // Fixed from original loop which implied break
                }
            }
            if count > 1000 {
                db.write(batch).map_err(|e| {
                    DbError::InternalError(format!("Failed to drop fulltext entries: {}", e))
                })?;
                batch = KvBatch::default();
                count = 0;
            }
        }

        if count > 0 {
            db.write(batch).map_err(|e| {
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
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");
        let mut batch = KvBatch::default();

        for index in indexes {
            for field in &index.fields {
                let field_value = extract_field_value(doc_value, field);
                if let Some(text) = field_value.as_str() {
                    for term in &Self::ft_terms(&index, text) {
                        let term_key = Self::ft_term_key(&index.name, term, doc_key);
                        batch.put_ks(&cf, term_key, doc_key.as_bytes());
                    }
                }
            }
        }

        db.write(batch)
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
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");
        let mut batch = KvBatch::default();

        for index in indexes {
            for field in &index.fields {
                let field_value = extract_field_value(doc_value, field);
                if let Some(text) = field_value.as_str() {
                    for term in &Self::ft_terms(&index, text) {
                        let term_key = Self::ft_term_key(&index.name, term, doc_key);
                        batch.delete_ks(&cf, term_key);
                    }

                    let ngrams = generate_ngrams(text, NGRAM_SIZE);
                    for ngram in &ngrams {
                        let ngram_key = Self::ft_ngram_key(&index.name, ngram, doc_key);
                        batch.delete_ks(&cf, ngram_key);
                    }
                }
            }
        }

        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to update fulltext index: {}", e)))
    }

    /// List fulltext indexes
    pub fn list_fulltext_indexes(&self) -> Vec<FulltextIndex> {
        self.get_all_fulltext_indexes()
    }

    /// Fulltext search ranked by BM25, exact terms only. See
    /// [`Self::fulltext_search_fuzzy`].
    pub fn fulltext_search(
        &self,
        query: &str,
        fields: Option<Vec<String>>,
        limit: usize,
    ) -> DbResult<Vec<FulltextMatch>> {
        self.fulltext_search_fuzzy(query, fields, limit, 0, None)
    }

    /// Fulltext search ranked by BM25.
    ///
    /// - **IDF** comes from the index: a term's document frequency is the
    ///   number of its postings (read up to [`MAX_POSTINGS_PER_TERM`]), over
    ///   the collection's document count. A term in most documents — a stop
    ///   word — weighs next to nothing, so no stop-word list is needed.
    /// - **tf** and the field length come from the document's field text.
    ///   The average length is that of the candidates scored, not of the
    ///   whole collection, which the index does not record.
    /// - A document's score is that of its best-scoring indexed field.
    ///
    /// Candidates are documents holding at least one query term exactly.
    /// With `max_distance > 0`, a word of a candidate within
    /// [`fuzzy_edits_for`] edits of a query term counts as half an
    /// occurrence of it.
    ///
    /// `phrase_slop`, when set, drops a field unless the query words occur
    /// in order inside it, with at most that many extra tokens between the
    /// first and the last (`0` means adjacent). A one-word query is not
    /// filtered. The phrase keeps every token of the query, repeats and
    /// words shorter than `min_length` included. A field that fails is not
    /// scored and does not enter the average length; the hit's spans are
    /// then that one occurrence.
    pub fn fulltext_search_fuzzy(
        &self,
        query: &str,
        fields: Option<Vec<String>>,
        limit: usize,
        max_distance: usize,
        phrase_slop: Option<usize>,
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

        // 2. Tokenize the query once per index: an index created with
        // `fold_accents` stores folded terms, so its query terms must be
        // folded too, and a plain index must keep seeing the raw ones. A
        // repeated query word counts once; a word shorter than the index's
        // `min_length` has no postings and cannot be scored.
        let query_terms: Vec<Vec<String>> = indexes
            .iter()
            .map(|index| {
                let mut seen = HashSet::new();
                tokenize_folded(query, index.fold_accents)
                    .into_iter()
                    .filter(|t| t.len() >= index.min_length && seen.insert(t.clone()))
                    .collect()
            })
            .collect();
        if query_terms.iter().all(|terms| terms.is_empty()) {
            // A phrase made only of words shorter than `min_length` lands
            // here too: those words have no postings, so nothing is a candidate.
            return Ok(Vec::new());
        }
        // The phrase is the query's tokens in order, repeats included and
        // with no `min_length` cut: "base de données" still requires "de".
        let phrase_terms: Vec<Vec<String>> = if phrase_slop.is_some() {
            indexes
                .iter()
                .map(|index| tokenize_folded(query, index.fold_accents))
                .collect()
        } else {
            Vec::new()
        };

        // 3. Read each term's postings: its document frequency, and the
        // candidates. A candidate is pre-ranked by the IDF of the terms it
        // holds, so one rare term outweighs several common ones.
        let total_docs = self.count();
        let mut idfs: Vec<HashMap<&str, f64>> = Vec::with_capacity(indexes.len());
        let mut candidate_weight: HashMap<String, f64> = HashMap::new();
        let db = &self.db;
        let Some(cf) = self.ks.handle(&self.db) else {
            return Ok(Vec::new()); // column family dropped mid-operation
        };

        // With a phrase, a document missing one of its words cannot match,
        // so it should not take a place among the candidates scored. Only
        // words that must match exactly can be required this way: a word
        // with fuzzy edits may be stood in for, and one shorter than
        // `min_length` has no postings. `None`: some index puts no such
        // constraint, so nothing is filtered.
        let mut admitted: Option<HashSet<String>> = phrase_slop.map(|_| HashSet::new());

        for (ix, (index, index_terms)) in indexes.iter().zip(&query_terms).enumerate() {
            let required: HashSet<&str> = match phrase_terms.get(ix) {
                Some(seq) if seq.len() >= 2 => seq
                    .iter()
                    .filter(|t| {
                        index_terms.contains(t)
                            && fuzzy_edits_for(t.chars().count(), max_distance) == 0
                    })
                    .map(String::as_str)
                    .collect(),
                _ => HashSet::new(),
            };
            let mut required_left = required.len();
            let mut required_hits: HashMap<String, usize> = HashMap::new();
            let mut index_idfs = HashMap::new();
            for term in index_terms {
                let prefix = format!("{}{}:{}:", FT_TERM_PREFIX, index.name, term);
                let mut doc_keys: Vec<String> = Vec::new();
                for (key, _) in db.prefix_iterator_ks(&cf, prefix.as_bytes()).flatten() {
                    if !key.starts_with(prefix.as_bytes())
                        || doc_keys.len() >= MAX_POSTINGS_PER_TERM
                    {
                        break;
                    }
                    // Key is "ft_term:<index>:<term>:<doc_key>". Terms are
                    // alphanumeric, so everything after the exact prefix is
                    // the doc key — which may itself contain ':' (audit D6;
                    // `split(':').last()` truncated such keys).
                    doc_keys.push(String::from_utf8_lossy(&key[prefix.len()..]).into_owned());
                }
                if doc_keys.is_empty() {
                    continue;
                }
                let idf = calculate_idf(total_docs, doc_keys.len());
                index_idfs.insert(term.as_str(), idf);
                // A truncated posting list cannot prove a document lacks the
                // word, so that word stops being required.
                let counts = required.contains(term.as_str());
                if counts && doc_keys.len() >= MAX_POSTINGS_PER_TERM {
                    required_left -= 1;
                }
                let counts = counts && doc_keys.len() < MAX_POSTINGS_PER_TERM;
                for doc_key in doc_keys {
                    if counts {
                        *required_hits.entry(doc_key.clone()).or_insert(0) += 1;
                    }
                    *candidate_weight.entry(doc_key).or_insert(0.0) += idf;
                }
            }
            idfs.push(index_idfs);
            if admitted.is_some() && required_left == 0 {
                admitted = None;
            } else if let Some(set) = admitted.as_mut() {
                // A required word without postings leaves this index
                // admitting nothing.
                set.extend(
                    required_hits
                        .into_iter()
                        .filter(|(_, n)| *n == required_left)
                        .map(|(k, _)| k),
                );
            }
        }
        if let Some(set) = &admitted {
            candidate_weight.retain(|doc_key, _| set.contains(doc_key));
        }

        // 4. Score only the most promising candidates (audit P5): scoring
        // loads the document and tokenizes it, so doing it for every posting
        // before applying `limit` scaled with the corpus.
        //
        // `max_scored` documents are scored. Without a phrase that is also
        // how many are loaded. With one, a candidate can hold every word and
        // still fail the order, so loading goes on, best weight first, until
        // `max_scored` documents pass or `MAX_SCORED_CANDIDATES` were read:
        // otherwise a hundred documents holding "base" and "données" apart
        // would fill the pool and hide the one that says "base de données".
        let max_scored = limit
            .saturating_mul(4)
            .clamp(MIN_SCORED_CANDIDATES, MAX_SCORED_CANDIDATES);
        let max_loaded = if phrase_slop.is_some() {
            MAX_SCORED_CANDIDATES
        } else {
            max_scored
        };
        let mut ranked: Vec<(String, f64)> = candidate_weight.into_iter().collect();
        if ranked.len() > max_loaded {
            ranked.select_nth_unstable_by(max_loaded - 1, |a, b| b.1.total_cmp(&a.1));
            ranked.truncate(max_loaded);
        }
        ranked.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        // 5. First pass: per candidate field, its length and how often each
        // query term occurs in it. BM25 needs the average length before any
        // score can be computed. Term strings come from `tokenize_folded`;
        // the span walk only supplies offsets, and only when it emits the
        // same terms, so a snippet cannot move a score. A field that fails
        // the phrase is left out of this average.
        struct FieldStats {
            name: String,
            len: usize,
            /// (index position, query term position, exact tf, fuzzy tf)
            hits: Vec<(usize, usize, usize, usize)>,
            spans: Vec<(usize, usize)>,
        }
        let mut stats: Vec<(String, Vec<FieldStats>)> = Vec::with_capacity(ranked.len());
        let (mut len_sum, mut len_count) = (0usize, 0usize);
        let mut scored = 0usize;
        for (doc_key, _) in ranked {
            if scored >= max_scored {
                break;
            }
            let Ok(doc) = self.get(&doc_key) else {
                continue;
            };
            let doc_value = doc.to_value();
            let mut fields_stats = Vec::new();
            for (ix, (index, index_terms)) in indexes.iter().zip(&query_terms).enumerate() {
                for field in &index.fields {
                    if fields.as_ref().is_some_and(|f| !f.contains(field)) {
                        continue;
                    }
                    let value = extract_field_value(&doc_value, field);
                    let Some(text) = value.as_str() else {
                        continue;
                    };
                    let doc_terms = tokenize_folded(text, index.fold_accents);
                    let located = tokenize_spans(text, index.fold_accents);
                    let spans_ok = located.len() == doc_terms.len()
                        && located
                            .iter()
                            .zip(&doc_terms)
                            .all(|(span, term)| span.term == *term);
                    // `None` means the phrase is off or vacuous (one word):
                    // spans stay every match. `Some` is the chosen occurrence.
                    // A field that fails is skipped, so it cannot win and it
                    // does not move the average length.
                    let phrase_tokens = if let Some(slop) = phrase_slop {
                        let seq = &phrase_terms[ix];
                        if seq.len() >= 2 {
                            match find_phrase(&doc_terms, seq, slop, max_distance) {
                                Some(hit) => Some(hit.tokens),
                                None => continue,
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    len_sum += doc_terms.len();
                    len_count += 1;
                    let mut hits = Vec::new();
                    let mut spans = Vec::new();
                    for (qx, q_term) in index_terms.iter().enumerate() {
                        let q_len = q_term.chars().count();
                        let edits = fuzzy_edits_for(q_len, max_distance);
                        let (mut exact, mut fuzzy) = (0, 0);
                        for (ti, d_term) in doc_terms.iter().enumerate() {
                            let matched_token = if d_term == q_term {
                                exact += 1;
                                true
                            } else if edits > 0
                                && q_len.abs_diff(d_term.chars().count()) <= edits
                                && levenshtein_distance(q_term, d_term) <= edits
                            {
                                fuzzy += 1;
                                true
                            } else {
                                false
                            };
                            if matched_token && spans_ok && phrase_tokens.is_none() {
                                let span = &located[ti];
                                spans.push((span.start, span.end));
                            }
                        }
                        if exact + fuzzy > 0 {
                            hits.push((ix, qx, exact, fuzzy));
                        }
                    }
                    if let Some(tokens) = &phrase_tokens {
                        if spans_ok {
                            for &ti in tokens {
                                let span = &located[ti];
                                spans.push((span.start, span.end));
                            }
                        }
                    }
                    spans.sort_unstable();
                    spans.dedup();
                    fields_stats.push(FieldStats {
                        name: field.clone(),
                        len: doc_terms.len(),
                        hits,
                        spans,
                    });
                }
            }
            if !fields_stats.is_empty() {
                scored += 1;
            }
            stats.push((doc_key, fields_stats));
        }
        let avg_len = if len_count > 0 {
            (len_sum as f64 / len_count as f64).max(1.0)
        } else {
            1.0
        };

        // 6. Second pass: BM25 of each field, best field wins. An equal
        // score keeps the first field, as it did before spans existed.
        struct BestField {
            score: f64,
            matched: Vec<String>,
            field: String,
            spans: Vec<(usize, usize)>,
        }
        let mut matches = Vec::new();
        for (doc_key, fields_stats) in stats {
            let mut best: Option<BestField> = None;
            for field in fields_stats {
                let norm = BM25_K1 * (1.0 - BM25_B + BM25_B * field.len as f64 / avg_len);
                let mut score = 0.0;
                let mut matched = Vec::new();
                for (ix, qx, exact, fuzzy) in field.hits {
                    let term = &query_terms[ix][qx];
                    // A term found only fuzzily has no postings of its own;
                    // weigh it as a rare one.
                    let idf = idfs[ix]
                        .get(term.as_str())
                        .copied()
                        .unwrap_or_else(|| calculate_idf(total_docs, 1));
                    let tf = exact as f64 + 0.5 * fuzzy as f64;
                    score += idf * tf * (BM25_K1 + 1.0) / (tf + norm);
                    matched.push(term.clone());
                }
                if score > 0.0 && best.as_ref().is_none_or(|b| score > b.score) {
                    best = Some(BestField {
                        score,
                        matched,
                        field: field.name,
                        spans: field.spans,
                    });
                }
            }
            if let Some(best) = best {
                matches.push(FulltextMatch {
                    doc_key,
                    score: best.score,
                    matched_terms: best.matched,
                    field: best.field,
                    spans: best.spans,
                });
            }
        }

        // 7. Sort and limit
        matches.sort_by(|a, b| b.score.total_cmp(&a.score));
        matches.truncate(limit);

        Ok(matches)
    }

    // ==================== Fulltext Index Entry Computation Helpers ====================

    /// Indexed terms of one field value, deduplicated. Every path that writes
    /// or deletes `ft_term:` keys goes through this, so an index's accent
    /// folding and `min_length` apply identically to all of them.
    pub(crate) fn ft_terms(
        index: &FulltextIndex,
        text: &str,
    ) -> std::collections::BTreeSet<String> {
        tokenize_folded(text, index.fold_accents)
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
        let Some(cf) = self.ks.handle(&self.db) else {
            return Vec::new();
        };
        let probe = Self::ft_ngram_key(&index.name, first, doc_key);
        if !matches!(self.db.get_pinned_ks(&cf, &probe), Ok(Some(_))) {
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
