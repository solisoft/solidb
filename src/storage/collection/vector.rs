use super::*;
use crate::error::{DbError, DbResult};
use crate::server::llm_client::LLMClient;
use crate::storage::index::{
    extract_field_value, VectorIndexConfig, VectorIndexStats, VectorQuantization,
};
use crate::storage::vector::{VectorIndex, VectorSearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Statistics about vector quantization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantizationStats {
    #[serde(rename = "type")]
    pub type_: VectorQuantization,
    /// Vectors held in quantized form.
    #[serde(default)]
    pub vectors_quantized: usize,
    pub original_size: usize,
    pub compressed_size: usize,
    pub compression_ratio: f32,
}
impl Collection {
    // ==================== Vector Index Operations ====================

    /// Get all vector index configurations
    pub fn get_all_vector_index_configs(&self) -> Vec<VectorIndexConfig> {
        // From the cached index definitions: every document write asks, and
        // this used to open an iterator each time. Empty when the column
        // family is gone (dropped mid-operation).
        self.index_meta()
            .map(|m| m.vector.clone())
            .unwrap_or_default()
    }

    /// Get vector index (loading it if necessary)
    pub fn get_vector_index(&self, name: &str) -> DbResult<Arc<super::vector::VectorIndex>> {
        // Try memory first
        if let Some(index) = self.vector_indexes.get(name) {
            return Ok(index.clone());
        }

        // Try load from disk
        self.load_vector_index(name)
    }

    /// Load vector index from disk
    pub(crate) fn load_vector_index(
        &self,
        name: &str,
    ) -> DbResult<Arc<super::vector::VectorIndex>> {
        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .ok_or(DbError::InternalError("Column family not found".into()))?;

        // Check metadata first
        let meta_key = Self::vec_meta_key(name);
        if db.get_ks(&cf, &meta_key)?.is_none() {
            return Err(DbError::InvalidDocument(format!(
                "Vector Index '{}' not found",
                name
            )));
        }

        // Load data
        let data_key = Self::vec_data_key(name);
        if let Some(bytes) = db.get_ks(&cf, &data_key)? {
            let loaded = match chunked_header(&bytes) {
                // Read piece by piece, never whole.
                Some((generation, count)) => {
                    let mut next = 0u32;
                    let reader = ChunkReader {
                        chunk: Vec::new(),
                        at: 0,
                        next: move || -> std::io::Result<Option<Vec<u8>>> {
                            if next >= count {
                                return Ok(None);
                            }
                            let key = vec_chunk_key(name, generation, next);
                            next += 1;
                            match db.get_ks(&cf, &key) {
                                Ok(Some(chunk)) => Ok(Some(chunk)),
                                Ok(None) => Err(std::io::Error::other("a piece is missing")),
                                Err(e) => Err(std::io::Error::other(e.to_string())),
                            }
                        },
                    };
                    super::vector::VectorIndex::deserialize_from(reader)
                }
                None => super::vector::VectorIndex::deserialize(&bytes),
            };
            match loaded {
                Ok(index) => {
                    let index_arc = Arc::new(index);
                    self.vector_indexes
                        .insert(name.to_string(), index_arc.clone());
                    Ok(index_arc)
                }
                Err(e) => Err(DbError::InternalError(format!(
                    "Failed to deserialize vector index: {}",
                    e
                ))),
            }
        } else {
            Err(DbError::InternalError("Vector index data missing".into()))
        }
    }

    /// Create a vector index
    pub fn create_vector_index(&self, config: VectorIndexConfig) -> DbResult<VectorIndexStats> {
        refuse_withdrawn_quantization(config.quantization)?;
        if config.multivector && config.embedding_source.is_some() {
            return Err(DbError::BadRequest(
                "a multivector index reads its vectors from the documents: an embedding \
                 provider gives one vector a text, not one a token"
                    .to_string(),
            ));
        }
        let name = config.name.clone();
        if self.get_vector_index(&name).is_ok() {
            return Err(DbError::InvalidDocument(format!(
                "Vector Index '{}' already exists",
                name
            )));
        }

        // Whatever the outcome below, from here on cached results may have
        // been computed against a half-built or half-removed index.
        let _invalidate = self.invalidate_cached_queries_on_exit();

        // Store metadata
        let config_bytes = serde_json::to_vec(&config)?;
        {
            let db = &self.db;
            let cf = self
                .ks
                .handle(&self.db)
                .expect("Column family should exist");
            let written = db
                .put_ks(&cf, Self::vec_meta_key(&name), &config_bytes)
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to create vector index: {}", e))
                });
            self.invalidate_index_meta();
            written?;
        }

        // Create in-memory index
        let index = VectorIndex::new(config.clone())?;

        // Build index from existing documents (with auto-embed support).
        // Collect texts first and use batched embedding (1 HTTP call instead of N serial)
        // for efficiency when building on large collections.
        //
        // The documents are read one at a time, and of each only the fields
        // the index reads: collecting the whole collection first held every
        // document decoded at once (6.6 GB for a million vectors of 128
        // dimensions, card #985).
        let mut direct_vectors: Vec<(String, Vec<f32>)> = Vec::new();
        let mut texts_to_embed: Vec<String> = Vec::new();
        let mut keys_for_embed: Vec<String> = Vec::new();
        let top = |path: &str| path.split('.').next().unwrap_or(path).to_string();
        let mut read_fields = vec![top(&config.field)];
        if let Some(source) = &config.embedding_source {
            read_fields.push(top(source));
        }
        read_fields.sort();
        read_fields.dedup();
        let (lower, upper) = super::crud::doc_key_range();
        self.for_each_doc_in(&lower, &upper, None, |key, bytes| {
            let Ok(doc_value) =
                crate::storage::serializer::deserialize_doc_projected(bytes, &read_fields)
            else {
                return Ok(true);
            };
            let key = || String::from_utf8_lossy(key).into_owned();
            let entries = self.index_entries(&config, &key(), &doc_value);
            if !entries.is_empty() {
                direct_vectors.extend(entries);
            } else if let Some(ref source_field) = config.embedding_source {
                if let Some(text) = extract_field_value(&doc_value, source_field).as_str() {
                    if !text.trim().is_empty() {
                        texts_to_embed.push(text.to_string());
                        keys_for_embed.push(key());
                    }
                }
            }
            Ok(true)
        })?;

        // Batched embedding generation (chunked to keep requests reasonable size).
        // This avoids the previous N serial per-document calls.
        let generated_embeddings: Vec<Vec<f32>> = if !texts_to_embed.is_empty() {
            let provider = config.embedding_provider.as_deref();
            let model = config.embedding_model.clone();
            match LLMClient::from_env(provider, model) {
                Ok(client) => {
                    let mut all_embs = vec![];
                    for chunk in texts_to_embed.chunks(200) {
                        let text_refs: Vec<&str> = chunk.iter().map(|s| s.as_str()).collect();
                        match client.embed_batch_blocking(&text_refs) {
                            Ok(embs) => all_embs.extend(embs),
                            Err(e) => {
                                tracing::warn!("Batch embedding chunk failed: {}", e);
                                // continue with partial
                            }
                        }
                    }
                    all_embs
                }
                Err(e) => {
                    tracing::warn!("Could not create embed client for index build: {}", e);
                    vec![]
                }
            }
        } else {
            vec![]
        };

        // Every vector in one batch: the graph is built once, on every CPU.
        let mut batch = direct_vectors;
        for (key, v) in keys_for_embed.into_iter().zip(generated_embeddings) {
            if v.len() == config.dimension {
                batch.push((key, v));
            } else if !v.is_empty() {
                tracing::warn!(
                    "Batch embed dim mismatch for key {} in index {}: got {} expected {}",
                    key,
                    config.name,
                    v.len(),
                    config.dimension
                );
            }
        }
        index.insert_many(batch);
        // Quantized from the config, at creation (card #996): every node
        // that creates the index builds the same one, where a later
        // quantize call reached only the node it was sent to.
        if config.quantization != VectorQuantization::None {
            index.quantize_as(config.quantization)?;
        }

        // Persist populated index
        self.persist_vector_indexes()?;

        let stats = index.stats();

        // Store in memory
        self.vector_indexes
            .insert(config.name.clone(), Arc::new(index));

        // Persist populated index
        self.persist_vector_indexes()?;

        Ok(stats)
    }

    /// Drop a vector index
    pub fn drop_vector_index(&self, name: &str) -> DbResult<()> {
        let _invalidate = self.invalidate_cached_queries_on_exit();

        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        // Remove from memory
        self.vector_indexes.remove(name);

        // Remove from disk
        let deleted = db
            .delete_ks(&cf, Self::vec_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to delete vector config: {}", e)));
        self.invalidate_index_meta();
        deleted?;
        db.delete_ks(&cf, Self::vec_data_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to delete vector data: {}", e)))?;
        let (from, to) = vec_chunk_range(name, 0, u64::MAX);
        let mut batch = crate::storage::kv::KvBatch::default();
        batch.delete_range_ks(&cf, from, to);
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to delete vector data: {}", e)))?;

        Ok(())
    }

    /// List vector indexes
    pub fn list_vector_indexes(&self) -> Vec<VectorIndexStats> {
        self.get_all_vector_index_configs()
            .into_iter()
            .map(|config| {
                let count = if let Some(idx) = self.vector_indexes.get(&config.name) {
                    idx.len()
                } else {
                    0
                };

                VectorIndexStats {
                    name: config.name,
                    field: config.field,
                    dimension: config.dimension,
                    metric: config.metric,
                    m: config.m,
                    ef_construction: config.ef_construction,
                    indexed_vectors: count,
                    quantization: config.quantization,
                    memory_bytes: 0,
                    compression_ratio: 1.0,
                }
            })
            .collect()
    }

    /// Search similar vectors
    pub fn vector_search(
        &self,
        name: &str,
        query: &[f32],
        k: usize,
        ef_search: Option<usize>,
    ) -> DbResult<Vec<VectorSearchResult>> {
        // Answered from the in-memory index, not from stored keys.
        self.note_range_read();
        let index = self.get_vector_index(name)?;

        let ef = ef_search.unwrap_or(100);
        if index.config().multivector {
            return self.multivector_search(&index, query, k, ef);
        }
        // A quantized graph ranks by its codes: a wider set is fetched and
        // re-scored against the exact vectors (card #996).
        if index.graph_is_quantized() {
            let fetch = k.saturating_mul(index.rescore_factor()).max(k);
            // The walk on codes is coarser: it is given room beyond the
            // candidates it returns.
            let found = index.search(query, fetch, ef.max(fetch.saturating_mul(2)))?;
            return Ok(self.rescored(&index, query, found, k));
        }
        index.search(query, k, ef)
    }

    /// `found` re-scored against the vectors stored in the documents,
    /// best first, the best `k` kept: a quantized graph's distances are
    /// approximate. A candidate deleted since it was indexed is dropped;
    /// one whose vector cannot be read keeps the graph's score.
    fn rescored(
        &self,
        index: &VectorIndex,
        query: &[f32],
        found: Vec<VectorSearchResult>,
        k: usize,
    ) -> Vec<VectorSearchResult> {
        let config = index.config();
        let top = config
            .field
            .split('.')
            .next()
            .unwrap_or(&config.field)
            .to_string();
        let fields = [top];
        let mut out: Vec<VectorSearchResult> = found
            .into_iter()
            .filter_map(|r| {
                let doc = self.get_value_projected(&r.doc_key, &fields).ok()?;
                let score = self
                    .extract_vector(&doc, &config.field, config.dimension)
                    .map_or(r.score, |v| index.exact_score(query, &v));
                Some(VectorSearchResult {
                    doc_key: r.doc_key,
                    score,
                })
            })
            .collect();
        out.sort_by(|a, b| index.better(a.score, b.score));
        out.truncate(k);
        out
    }

    /// Nearest-neighbour search with an optional equality metadata filter.
    ///
    /// With a graph, the filter is followed in the graph's walk
    /// ([`Self::filtered_walk`]): `ef` bounds the documents that pass, as
    /// pgvector's `hnsw.ef_search` does, and a selective filter widens the
    /// walk rather than starting it again (card #984: two searches and a
    /// read of each candidate per query put SoliDB 60% behind pgvector at
    /// eight clients).
    ///
    /// Without one (an index below its graph threshold): `k * overfetch`
    /// candidates are fetched, hydrated and filtered, and while fewer than `k`
    /// of them pass and the index holds more, the search is run again four
    /// times wider (its `ef` at least that wide), up to the whole index or
    /// `MAX_FILTERED_CANDIDATES`. A document is read and checked once across
    /// the rounds. Returns `(document, score)` pairs, best score first.
    ///
    /// One fixed round of `4 * k` kept about 4 of the 10 nearest for a filter
    /// that 10% of the documents pass, whatever `ef` (card #979). pgvector 0.8
    /// widens the same way (`hnsw.iterative_scan`).
    ///
    /// `filter` is a map of `field_path -> expected JSON value`; a document matches
    /// only if every entry equals the value at that (dotted) path. An empty filter
    /// matches everything (plain top-k search).
    pub fn vector_search_filtered(
        &self,
        name: &str,
        query: &[f32],
        k: usize,
        overfetch: usize,
        ef_search: Option<usize>,
        filter: &serde_json::Map<String, Value>,
    ) -> DbResult<Vec<(Value, f32)>> {
        /// The widest a filtered search goes before it answers with fewer
        /// than `k` rows.
        const MAX_FILTERED_CANDIDATES: usize = 10_000;
        if k == 0 {
            return Ok(Vec::new());
        }
        let held = self.get_vector_index(name)?.len();
        // Over-fetch so a selective filter can still yield up to k rows.
        let mut fetch = k.saturating_mul(overfetch.max(1)).max(k);
        // What each document read so far answered: the document when it
        // passes the filter, `None` when it fails it or is gone.
        // The top-level fields the filter's paths start with.
        let mut filtered_fields: Vec<String> = filter
            .keys()
            .map(|path| path.split('.').next().unwrap_or(path).to_string())
            .collect();
        filtered_fields.sort();
        filtered_fields.dedup();
        // A multivector graph's nodes are not documents: its candidates are
        // filtered below, document by document.
        let multivector = self.get_vector_index(name)?.config().multivector;
        if !filter.is_empty() && !multivector {
            if let Some(found) = self.filtered_walk(
                name,
                query,
                k,
                ef_search.unwrap_or_else(|| fetch.max(100)),
                MAX_FILTERED_CANDIDATES,
                filter,
                &filtered_fields,
            )? {
                return Ok(found);
            }
        }
        let mut checked: std::collections::HashMap<String, Option<Value>> =
            std::collections::HashMap::new();
        loop {
            let ef = ef_search.map(|e| e.max(fetch)).or(Some(fetch.max(100)));
            let candidates = self.vector_search(name, query, fetch, ef)?;
            let got = candidates.len();
            // Audit A2: `k` is caller-controlled; never reserve more than exists.
            let mut out: Vec<(Value, f32)> = Vec::with_capacity(k.min(got));
            for r in candidates {
                if out.len() >= k {
                    break;
                }
                let kept = checked.entry(r.doc_key.clone()).or_insert_with(|| {
                    // Only the fields the filter names are decoded to check
                    // it (an embedding is most of a document); the whole
                    // document is read for the candidates that pass.
                    let passes = self
                        .get_value_projected(&r.doc_key, &filtered_fields)
                        .is_ok_and(|v| Self::doc_matches_filter(&v, filter));
                    if !passes {
                        return None;
                    }
                    self.get(&r.doc_key).ok().map(|d| d.to_value()) // None: deleted since indexing
                });
                if let Some(doc) = kept {
                    out.push((doc.clone(), r.score));
                }
            }
            let exhausted = got < fetch || fetch >= held || fetch >= MAX_FILTERED_CANDIDATES;
            if out.len() >= k || exhausted || filter.is_empty() {
                return Ok(out);
            }
            fetch = fetch
                .saturating_mul(4)
                .min(held)
                .min(MAX_FILTERED_CANDIDATES);
        }
    }

    /// The filtered search in the graph's walk ([`VectorIndex::search_filtered`]):
    /// each node the walk would keep is checked on the fields the filter
    /// names, read once and kept between queries in the map traversals
    /// keep their projections in (`storage/kept.rs`), which every write of
    /// a document forgets. The whole document is read for the `k` kept.
    /// `None` when the index has no graph.
    #[allow(clippy::too_many_arguments)]
    fn filtered_walk(
        &self,
        name: &str,
        query: &[f32],
        k: usize,
        ef: usize,
        max_checks: usize,
        filter: &serde_json::Map<String, Value>,
        fields: &[String],
    ) -> DbResult<Option<Vec<(Value, f32)>>> {
        // Answered from the in-memory index and the kept projections.
        self.note_range_read();
        let index = self.get_vector_index(name)?;
        // A map of its own: a traversal's projection of the same fields
        // is not read the same way.
        let mut kept_as: Vec<String> = Vec::with_capacity(fields.len() + 1);
        kept_as.push("\u{0}vector filter".to_string());
        kept_as.extend_from_slice(fields);
        let kept = crate::storage::kept::vertices(self.ks.state(), Some(&kept_as));
        let mut passes = |key: &str| -> bool {
            if let Some(found) = kept.as_ref().and_then(|kept| kept.get(key)) {
                return Self::doc_matches_filter(&found, filter);
            }
            let seen = kept.as_ref().map(|kept| kept.epoch());
            let Ok(projected) = self.get_value_projected(key, fields) else {
                return false; // deleted since indexing
            };
            let ok = Self::doc_matches_filter(&projected, filter);
            if let Some(kept) = &kept {
                let size = 48 * fields.len().max(1);
                kept.put(
                    key,
                    &std::sync::Arc::new(projected),
                    size,
                    seen.unwrap_or_default(),
                );
            }
            ok
        };
        // A quantized graph: a wider set that passes, re-scored (#996).
        let quantized = index.graph_is_quantized();
        let fetch = if quantized {
            k.saturating_mul(index.rescore_factor()).max(k)
        } else {
            k
        };
        // The walk on codes is coarser: room beyond what it returns. An
        // index in f32 walks exactly as it did.
        let walk_ef = if quantized {
            ef.max(fetch.saturating_mul(2))
        } else {
            ef
        };
        let Some(found) = index.search_filtered(query, fetch, walk_ef, max_checks, &mut passes)?
        else {
            return Ok(None);
        };
        let found = if quantized {
            self.rescored(&index, query, found, k)
        } else {
            found
        };
        Ok(Some(
            found
                .into_iter()
                .filter_map(|r| {
                    self.get(&r.doc_key)
                        .ok()
                        .map(|doc| (doc.to_value(), r.score))
                })
                .collect(),
        ))
    }

    /// True when the document equals `filter` on every field (dotted paths allowed).
    fn doc_matches_filter(doc: &Value, filter: &serde_json::Map<String, Value>) -> bool {
        filter
            .iter()
            .all(|(field, expected)| extract_field_value(doc, field) == *expected)
    }

    /// Calculate similarity between a vector and documents
    pub fn vector_similarity(&self, name: &str, query: Vec<f32>) -> DbResult<Vec<(String, f32)>> {
        let index = self.get_vector_index(name)?;
        let results = index.search(&query, 100, 100)?;
        Ok(results.into_iter().map(|r| (r.doc_key, r.score)).collect())
    }

    /// Quantize a vector index
    pub fn quantize_vector_index(
        &self,
        name: &str,
        quantization: VectorQuantization,
    ) -> DbResult<QuantizationStats> {
        let index = self.get_vector_index(name)?;

        // `None` would be a no-op that reported success.
        if quantization == VectorQuantization::None {
            return Err(DbError::InvalidDocument(
                "quantization must be 'scalar'; use dequantize to remove it".to_string(),
            ));
        }
        refuse_withdrawn_quantization(quantization)?;
        index.quantize_as(quantization)?;
        self.set_vector_index_quantization(name, quantization)?;
        self.persist_vector_indexes()?;

        Ok(match index.quantization_stats() {
            Some(s) => QuantizationStats {
                type_: quantization,
                vectors_quantized: s.vector_count,
                original_size: s.full_memory_bytes,
                compressed_size: s.memory_bytes,
                compression_ratio: s.compression_ratio,
            },
            // An empty index has nothing to quantize yet.
            None => QuantizationStats {
                type_: quantization,
                vectors_quantized: 0,
                original_size: 0,
                compressed_size: 0,
                compression_ratio: 1.0,
            },
        })
    }

    /// Dequantize a vector index, returning to full-precision search.
    pub fn dequantize_vector_index(&self, name: &str) -> DbResult<()> {
        let index = self.get_vector_index(name)?;
        index.dequantize();
        self.set_vector_index_quantization(name, VectorQuantization::None)?;
        self.persist_vector_indexes()
    }

    fn set_vector_index_quantization(
        &self,
        name: &str,
        quantization: VectorQuantization,
    ) -> DbResult<()> {
        let mut configs = self.get_all_vector_index_configs();
        if let Some(config) = configs.iter_mut().find(|c| c.name == name) {
            config.quantization = quantization;
            let cf = self.ks.live(&self.db, &self.name)?;
            let config_bytes = serde_json::to_vec(config)?;
            let written = self
                .db
                .put_ks(&cf, Self::vec_meta_key(name), &config_bytes)
                .map_err(|e| DbError::InternalError(e.to_string()));
            self.invalidate_index_meta();
            written?;
        }
        Ok(())
    }

    /// Persist all in-memory vector indexes to disk
    pub fn persist_vector_indexes(&self) -> DbResult<()> {
        Self::persist_vector_index_map(&self.db, &self.ks, &self.name, &self.vector_indexes)
    }

    /// Persist every index in `indexes` into the keyspace `ks` of collection
    /// `name`.
    pub(crate) fn persist_vector_index_map(
        db: &crate::storage::RocksDb,
        ks: &Keyspace,
        name: &str,
        indexes: &dashmap::DashMap<String, Arc<VectorIndex>>,
    ) -> DbResult<()> {
        let cf = ks.live(db, name)?;

        // Snapshot the handles first: serializing a large index takes a
        // while, and must not pin a DashMap shard lock meanwhile.
        let snapshot: Vec<(String, Arc<VectorIndex>)> = indexes
            .iter()
            .map(|e| (e.key().clone(), Arc::clone(e.value())))
            .collect();
        // Each index is written in pieces under a new generation, then the
        // header that names it, then the older pieces are deleted: a crash
        // in between leaves the header on a whole generation. Written as one
        // value, a million vectors of 128 dimensions were one 800 MB buffer,
        // then as much in the memtable (card #986).
        let failed = |name: &str, e: String| {
            DbError::InternalError(format!("Failed to persist vector index {}: {}", name, e))
        };
        for (name, index_arc) in snapshot {
            let data_key = Self::vec_data_key(&name);
            let previous = db
                .get_ks(&cf, &data_key)
                .map_err(|e| failed(&name, e.to_string()))?
                .and_then(|bytes| chunked_header(&bytes))
                .map_or(0, |(generation, _)| generation);
            let generation = previous + 1;
            let mut count = 0u32;
            {
                let mut writer = ChunkWriter {
                    buf: Vec::with_capacity(VEC_CHUNK_BYTES),
                    put: |piece: &[u8]| -> std::io::Result<()> {
                        db.put_ks(&cf, vec_chunk_key(&name, generation, count), piece)
                            .map_err(|e| std::io::Error::other(e.to_string()))?;
                        count += 1;
                        Ok(())
                    },
                };
                index_arc.serialize_into(&mut writer)?;
                writer.finish().map_err(|e| failed(&name, e.to_string()))?;
            }
            let mut header = VEC_CHUNKED.to_vec();
            header.extend_from_slice(&generation.to_be_bytes());
            header.extend_from_slice(&count.to_be_bytes());
            db.put_ks(&cf, &data_key, &header)
                .map_err(|e| failed(&name, e.to_string()))?;
            let (from, to) = vec_chunk_range(&name, 0, generation);
            let mut batch = crate::storage::kv::KvBatch::default();
            batch.delete_range_ks(&cf, from, to);
            db.write(batch).map_err(|e| failed(&name, e.to_string()))?;
        }
        Ok(())
    }

    /// Update vector indexes on doc update/insert.
    ///
    /// This runs inside the synchronous storage write path (and under transaction
    /// commit), so it MUST NOT perform network I/O. It indexes a vector only when
    /// the document already carries one in the target field. When an index declares
    /// an `embedding_source` and the document has source text but no vector, it
    /// records a cheap "pending embed" marker for the async embedding worker
    /// (`crate::queue`) to fill in later — never blocking the write on the network.
    /// Auto-embedding may also be done up front in the async API layer
    /// (`inject_auto_embeddings_if_needed`).
    pub(crate) fn update_vector_indexes_on_upsert(&self, doc_key: &str, doc_value: &Value) {
        let configs = self.get_all_vector_index_configs();
        if configs.is_empty() {
            return;
        }

        // Ensure loaded
        for config in &configs {
            let _ = self.get_vector_index(&config.name);
        }

        for config in configs {
            // 1. A concrete vector present in the doc — index it directly.
            if let Some(index) = self.vector_indexes.get(&config.name) {
                if config.multivector {
                    // A document's vectors are replaced whole: it may hold
                    // fewer than before (card #1005).
                    let removed = Self::remove_doc_nodes(&index, doc_key);
                    let entries = self.index_entries(&config, doc_key, doc_value);
                    for (node, vector) in &entries {
                        let _ = index.insert(node, vector);
                    }
                    if removed || !entries.is_empty() {
                        self.mark_vec_dirty();
                    }
                    continue;
                }
                if let Some(vector) =
                    self.extract_vector(doc_value, &config.field, config.dimension)
                {
                    let _ = index.insert(doc_key, &vector);
                    // Mark for (throttled) persistence; the actual disk write is
                    // deferred so a bulk load doesn't re-serialize the whole
                    // index per batch.
                    self.mark_vec_dirty();
                    if config.embedding_source.is_some() {
                        // Vector is now present; drop any stale pending-embed marker.
                        self.clear_embed_pending(&config.name, doc_key);
                    }
                    continue;
                }
            }

            // 2. No vector. If this index auto-embeds from a text field, enqueue a
            //    pending marker (or clear it when the source text is gone).
            if let Some(ref source_field) = config.embedding_source {
                let has_text = extract_field_value(doc_value, source_field)
                    .as_str()
                    .map(|t| !t.trim().is_empty())
                    .unwrap_or(false);
                if has_text {
                    self.mark_embed_pending(&config.name, doc_key);
                } else {
                    self.clear_embed_pending(&config.name, doc_key);
                }
            }
        }
    }

    /// Update vector indexes on doc delete
    /// Whether a deleted document may have anything in a vector index, or
    /// an embedding pending for one.
    pub(crate) fn has_vector_indexes(&self) -> bool {
        !self.vector_indexes.is_empty() || !self.get_all_vector_index_configs().is_empty()
    }

    pub(crate) fn update_vector_indexes_on_delete(&self, doc_key: &str) {
        let mut changed = false;
        for entry in self.vector_indexes.iter() {
            if Self::remove_doc_nodes(entry.value(), doc_key) {
                changed = true;
            }
        }
        if changed {
            self.mark_vec_dirty();
        }
        // Drop any pending-embed markers for this doc across all auto-embed indexes.
        for config in self.get_all_vector_index_configs() {
            if config.embedding_source.is_some() {
                self.clear_embed_pending(&config.name, doc_key);
            }
        }
    }

    /// Whether a document UPDATE leaves every vector index's embedding
    /// unchanged. When true, the caller can skip the delete+reinsert into the
    /// vector index entirely — no HNSW churn and no dirty flag (hence no
    /// persist). This is the dominant case for an incremental graph sync, which
    /// rewrites node documents to refresh metadata / a content hash while their
    /// embeddings stay byte-identical. Compared on the extracted `f32` vectors,
    /// so it matches exactly what would otherwise be re-indexed.
    pub(crate) fn vector_index_unchanged(&self, old_value: &Value, new_value: &Value) -> bool {
        for config in self.get_all_vector_index_configs() {
            let old_vec = self.index_entries(&config, "", old_value);
            let new_vec = self.index_entries(&config, "", new_value);
            if old_vec != new_vec {
                return false;
            }
        }
        true
    }

    /// Helper to extract vector from document
    pub(crate) fn extract_vector(
        &self,
        doc_value: &Value,
        field: &str,
        dim: usize,
    ) -> Option<Vec<f32>> {
        let val = extract_field_value(doc_value, field);
        if let Some(arr) = val.as_array() {
            if arr.len() == dim {
                let vec: Option<Vec<f32>> =
                    arr.iter().map(|v| v.as_f64().map(|f| f as f32)).collect();
                return vec;
            }
        }
        None
    }

    /// The vectors of a multivector index's field: a non-empty array of
    /// arrays of `dim` numbers each (card #1005).
    pub(crate) fn extract_vectors(
        &self,
        doc_value: &Value,
        field: &str,
        dim: usize,
    ) -> Option<Vec<Vec<f32>>> {
        let rows = extract_field_value(doc_value, field);
        let rows = rows.as_array().filter(|rows| !rows.is_empty())?;
        rows.iter()
            .map(|row| {
                let row = row.as_array().filter(|row| row.len() == dim)?;
                row.iter().map(|v| v.as_f64().map(|f| f as f32)).collect()
            })
            .collect()
    }

    /// A document's vectors for index `config`, as `(graph node key,
    /// vector)`: one of key `doc_key`, or one a vector of a multivector
    /// field. Empty when the field holds none.
    fn index_entries(
        &self,
        config: &VectorIndexConfig,
        doc_key: &str,
        doc_value: &Value,
    ) -> Vec<(String, Vec<f32>)> {
        if config.multivector {
            return self
                .extract_vectors(doc_value, &config.field, config.dimension)
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .map(|(i, v)| (node_key(doc_key, i), v))
                .collect();
        }
        self.extract_vector(doc_value, &config.field, config.dimension)
            .map(|v| vec![(doc_key.to_string(), v)])
            .unwrap_or_default()
    }

    /// Remove a document's nodes from `index`. Whether any was there.
    fn remove_doc_nodes(index: &VectorIndex, doc_key: &str) -> bool {
        if !index.config().multivector {
            return matches!(index.remove(doc_key), Ok(true));
        }
        let mut i = 0;
        while matches!(index.remove(&node_key(doc_key, i)), Ok(true)) {
            i += 1;
        }
        i > 0
    }

    /// Late-interaction search (card #1005): `query` holds one or more
    /// query vectors end to end. Each finds its nearest nodes; the
    /// documents they belong to are scored exactly by MaxSim, the sum over
    /// the query vectors of the best score among the document's vectors
    /// (read from the document), and the best `k` returned.
    fn multivector_search(
        &self,
        index: &VectorIndex,
        query: &[f32],
        k: usize,
        ef: usize,
    ) -> DbResult<Vec<VectorSearchResult>> {
        let config = index.config();
        let dim = config.dimension;
        if query.is_empty() || !query.len().is_multiple_of(dim) {
            return Err(DbError::BadRequest(format!(
                "a multivector search takes query vectors of {dim} dimensions"
            )));
        }
        if k == 0 {
            return Ok(Vec::new());
        }
        // Candidates: what each query vector's nearest nodes belong to. A
        // document matching on a few tokens is found by those tokens.
        let fetch = k.saturating_mul(MULTIVECTOR_CANDIDATES).max(32);
        let mut candidates: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for q in query.chunks_exact(dim) {
            let walk_ef = ef.max(fetch);
            for r in index.search(q, fetch, walk_ef)? {
                let doc = doc_of_node(&r.doc_key);
                if seen.insert(doc.to_string()) {
                    candidates.push(doc.to_string());
                }
            }
        }
        let top = config
            .field
            .split('.')
            .next()
            .unwrap_or(&config.field)
            .to_string();
        let fields = [top];
        let mut out: Vec<VectorSearchResult> = candidates
            .into_iter()
            .filter_map(|doc_key| {
                // Deleted since it was indexed: dropped.
                let doc = self.get_value_projected(&doc_key, &fields).ok()?;
                let vectors = self.extract_vectors(&doc, &config.field, dim)?;
                let score = query
                    .chunks_exact(dim)
                    .map(|q| {
                        best_score(index, vectors.iter().map(|v| index.exact_score(q, v)))
                            .unwrap_or(0.0)
                    })
                    .sum();
                Some(VectorSearchResult { doc_key, score })
            })
            .collect();
        out.sort_by(|a, b| index.better(a.score, b.score));
        out.truncate(k);
        Ok(out)
    }

    // ==================== Auto-embedding pending queue ====================
    //
    // When an auto-embed document lands without a vector, the write path records a
    // cheap marker key in the collection's own column family (same idiom as TTL /
    // vector-metadata entries). The async embedding worker (`crate::queue`) drains
    // these, generates embeddings off the write path, and writes them back.

    fn embed_pending_key(index: &str, doc_key: &str) -> String {
        format!("{}{}:{}", EMBED_PENDING_PREFIX, index, doc_key)
    }

    fn embed_pending_index_prefix(index: &str) -> String {
        format!("{}{}:", EMBED_PENDING_PREFIX, index)
    }

    /// Record that `doc_key` needs an embedding generated for vector index `index`.
    /// Cheap and network-free — consumed later by the async embedding worker.
    pub(crate) fn mark_embed_pending(&self, index: &str, doc_key: &str) {
        let Some(cf) = self.ks.handle(&self.db) else {
            return;
        };
        let key = Self::embed_pending_key(index, doc_key);
        // Only count genuinely new markers so the process-wide gauge stays meaningful.
        if matches!(self.db.get_ks(&cf, key.as_bytes()), Ok(Some(_))) {
            return;
        }
        if self.db.put_ks(&cf, key.as_bytes(), b"").is_ok() {
            PENDING_EMBED_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Remove a pending-embed marker (if present).
    pub(crate) fn clear_embed_pending(&self, index: &str, doc_key: &str) {
        let Some(cf) = self.ks.handle(&self.db) else {
            return;
        };
        let key = Self::embed_pending_key(index, doc_key);
        if matches!(self.db.get_ks(&cf, key.as_bytes()), Ok(Some(_)))
            && self.db.delete_ks(&cf, key.as_bytes()).is_ok()
        {
            PENDING_EMBED_COUNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Doc keys awaiting embedding for a given vector index (bounded by `limit`).
    pub fn list_embed_pending(&self, index: &str, limit: usize) -> Vec<String> {
        let Some(cf) = self.ks.handle(&self.db) else {
            return Vec::new();
        };
        let prefix = Self::embed_pending_index_prefix(index);
        let mut out = Vec::new();
        for item in self.db.prefix_iterator_ks(&cf, prefix.as_bytes()) {
            if out.len() >= limit {
                break;
            }
            let Ok((k, _)) = item else {
                break;
            };
            if !k.starts_with(prefix.as_bytes()) {
                break;
            }
            out.push(String::from_utf8_lossy(&k[prefix.len()..]).into_owned());
        }
        out
    }

    /// Total documents awaiting embedding in this collection (all auto-embed indexes).
    pub fn count_embed_pending(&self) -> usize {
        let Some(cf) = self.ks.handle(&self.db) else {
            return 0;
        };
        let prefix = EMBED_PENDING_PREFIX.as_bytes();
        let mut n = 0;
        for item in self.db.prefix_iterator_ks(&cf, prefix) {
            let Ok((k, _)) = item else {
                break;
            };
            if !k.starts_with(prefix) {
                break;
            }
            n += 1;
        }
        n
    }
}

/// Process-wide gauge of documents awaiting auto-embedding. The background
/// embedding worker reads this as a cheap "is there any work?" gate before
/// scanning collections.
static PENDING_EMBED_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Number of documents currently awaiting auto-embedding, process-wide (approximate).
pub fn pending_embed_count() -> u64 {
    PENDING_EMBED_COUNT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Retire `n` from the process-wide pending gauge, saturating at zero.
///
/// The gauge is advisory. `mark_embed_pending` counts a marker up and
/// `clear_embed_pending` counts it back down, but a marker can also vanish
/// with its column family when a collection or database is dropped, and that
/// path holds no `Collection` to decrement through. A test suite that creates
/// and drops databases therefore leaves the gauge permanently above zero,
/// which keeps `check_embeddings` past its fast path — enumerating every
/// collection in the instance on every worker tick, forever. The sweep calls
/// this once a complete pass has proved no markers exist.
pub(crate) fn release_pending_embed(n: u64) {
    if n == 0 {
        return;
    }
    super::saturating_sub_u64(&PENDING_EMBED_COUNT, n);
}

/// What `vec_data:<name>` starts with when the index is saved in pieces:
/// then the generation (u64) and the number of pieces (u32), big-endian.
/// An index saved whole (2.14.x and earlier) starts otherwise.
const VEC_CHUNKED: &[u8] = b"\xffSVCHUNK";
/// The size of a saved piece.
const VEC_CHUNK_BYTES: usize = 16 << 20;

fn chunked_header(bytes: &[u8]) -> Option<(u64, u32)> {
    let rest = bytes.strip_prefix(VEC_CHUNKED)?;
    let generation = u64::from_be_bytes(rest.get(..8)?.try_into().ok()?);
    let count = u32::from_be_bytes(rest.get(8..12)?.try_into().ok()?);
    Some((generation, count))
}

/// The keys of `name`'s pieces of generations `[from, to)`. The name's
/// length comes first, so that no name's keys run into another's.
fn vec_chunk_range(name: &str, from: u64, to: u64) -> (Vec<u8>, Vec<u8>) {
    let base = format!("{}{}:{}:", VEC_CHUNK_PREFIX, name.len(), name);
    (
        format!("{base}{from:016x}").into_bytes(),
        if to == u64::MAX {
            format!("{base}g").into_bytes()
        } else {
            format!("{base}{to:016x}").into_bytes()
        },
    )
}

fn vec_chunk_key(name: &str, generation: u64, n: u32) -> Vec<u8> {
    format!(
        "{}{}:{}:{generation:016x}:{n:08x}",
        VEC_CHUNK_PREFIX,
        name.len(),
        name
    )
    .into_bytes()
}

/// Hands what it is written to `put` in pieces of `VEC_CHUNK_BYTES`.
struct ChunkWriter<F: FnMut(&[u8]) -> std::io::Result<()>> {
    buf: Vec<u8>,
    put: F,
}

impl<F: FnMut(&[u8]) -> std::io::Result<()>> ChunkWriter<F> {
    fn finish(mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            (self.put)(&self.buf)?;
        }
        Ok(())
    }
}

impl<F: FnMut(&[u8]) -> std::io::Result<()>> std::io::Write for ChunkWriter<F> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let room = VEC_CHUNK_BYTES - self.buf.len();
        let taken = data.len().min(room);
        self.buf.extend_from_slice(&data[..taken]);
        if self.buf.len() == VEC_CHUNK_BYTES {
            (self.put)(&self.buf)?;
            self.buf.clear();
        }
        Ok(taken)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Reads the pieces `next` gives, one after the other.
struct ChunkReader<F: FnMut() -> std::io::Result<Option<Vec<u8>>>> {
    chunk: Vec<u8>,
    at: usize,
    next: F,
}

impl<F: FnMut() -> std::io::Result<Option<Vec<u8>>>> std::io::Read for ChunkReader<F> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.at == self.chunk.len() {
            match (self.next)()? {
                Some(chunk) => {
                    self.chunk = chunk;
                    self.at = 0;
                }
                None => return Ok(0),
            }
        }
        let n = out.len().min(self.chunk.len() - self.at);
        out[..n].copy_from_slice(&self.chunk[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// Binary quantization is withdrawn (card #1008): one bit a dimension does
/// not hold recall on a large collection, even on residuals from coarse
/// centroids. Measured on clustered data, recall@10 with 20 candidates a
/// result: 0.385 at 1M vectors (128 dims) before residuals, 0.85-0.87 at
/// 100k after (128 and 768 dims), where PQ gives 0.99. The code stays,
/// unreachable through the API, for a later version.
fn refuse_withdrawn_quantization(quantization: VectorQuantization) -> DbResult<()> {
    let why = match quantization {
        VectorQuantization::Binary => {
            "binary quantization is not available: it loses too many of the nearest \
             neighbours on large collections"
        }
        // Card #1007: at 1M x 768 the server held 8.10 GiB against 8.38 for
        // scalar (the documents keep their own copy of each vector) and
        // answered a third of scalar's searches.
        VectorQuantization::Pq => {
            "product quantization is not available: it saves little more memory than \
             scalar, since the documents keep their own vectors, at a third of its speed"
        }
        _ => return Ok(()),
    };
    Err(DbError::BadRequest(format!(
        "{why}; use 'scalar' (4x less vector memory)"
    )))
}

/// Candidates a query vector of a multivector search fetches, per result
/// asked for (card #1005): enough that a document strong on a few query
/// tokens is among some token's nearest nodes.
const MULTIVECTOR_CANDIDATES: usize = 8;

/// A multivector index's graph node for vector `i` of document `doc_key`
/// (card #1005): `<key>\u{1f}<i>`. A document's nodes are numbered from 0
/// with no gap, so they are removed by counting until one is missing; the
/// last separator gives the document back, whatever its key holds.
fn node_key(doc_key: &str, i: usize) -> String {
    format!("{doc_key}\u{1f}{i}")
}

fn doc_of_node(node: &str) -> &str {
    node.rsplit_once('\u{1f}').map_or(node, |(doc, _)| doc)
}

/// The best of `scores` for `index` (highest similarity, or nearest).
fn best_score(index: &VectorIndex, scores: impl Iterator<Item = f32>) -> Option<f32> {
    scores.min_by(|a, b| index.better(*a, *b))
}

/// Query texts already embedded (card #1003): (database, provider, model,
/// dimension, text) -> vector. A search typed again does not pay the
/// provider again. Bounded: emptied when it reaches its size.
fn query_embeddings() -> &'static parking_lot::Mutex<std::collections::HashMap<String, Vec<f32>>> {
    static CACHE: std::sync::OnceLock<
        parking_lot::Mutex<std::collections::HashMap<String, Vec<f32>>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| parking_lot::Mutex::new(std::collections::HashMap::new()))
}
const QUERY_EMBEDDINGS_MAX: usize = 1_024;

/// The vector of a search by text (card #1003): `text` embedded with the
/// provider and model that embed the index's documents (`embedding_provider`,
/// OpenAI when unset; `embedding_model`), so the two are comparable. An
/// index that embeds nothing (no `embedding_source` nor provider) refuses:
/// its vectors come from elsewhere and a text cannot be put beside them.
pub fn embed_query_text(
    storage: &crate::storage::StorageEngine,
    db_name: &str,
    config: &VectorIndexConfig,
    text: &str,
) -> DbResult<Vec<f32>> {
    if config.multivector {
        return Err(DbError::BadRequest(format!(
            "vector index '{}' is multivector: search it with its query vectors \
             (one a token), as the model that made the documents' gives them",
            config.name
        )));
    }
    if config.embedding_source.is_none() && config.embedding_provider.is_none() {
        return Err(DbError::BadRequest(format!(
            "vector index '{}' has no embedding source: search it with a vector, \
             or create it with embedding_source so a text can be embedded",
            config.name
        )));
    }
    let provider = config
        .embedding_provider
        .clone()
        .unwrap_or_else(|| "openai".to_string());
    let key = format!(
        "{db_name}\u{0}{provider}\u{0}{}\u{0}{}\u{0}{text}",
        config.embedding_model.as_deref().unwrap_or(""),
        config.dimension
    );
    if let Some(v) = query_embeddings().lock().get(&key) {
        return Ok(v.clone());
    }
    let client = LLMClient::from_storage(
        storage,
        db_name,
        Some(&provider),
        config.embedding_model.clone(),
    )?;
    let vector = client.embed_blocking(text)?;
    if vector.len() != config.dimension {
        return Err(DbError::BadRequest(format!(
            "the embedding of the query has {} dimensions, vector index '{}' has {}",
            vector.len(),
            config.name,
            config.dimension
        )));
    }
    let mut cache = query_embeddings().lock();
    if cache.len() >= QUERY_EMBEDDINGS_MAX {
        cache.clear();
    }
    cache.insert(key, vector.clone());
    Ok(vector)
}

#[cfg(test)]
mod truncate_tests {
    use crate::storage::StorageEngine;
    use serde_json::json;

    #[test]
    fn truncate_removes_pending_embed_markers() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
        engine.create_database("d".to_string()).unwrap();
        let db = engine.get_database("d").unwrap();
        db.create_collection("c".to_string(), None).unwrap();
        let coll = db.get_collection("c").unwrap();
        coll.insert(json!({"_key": "a"})).unwrap();

        coll.mark_embed_pending("v", "a");
        assert_eq!(coll.count_embed_pending(), 1, "control: marker written");

        coll.truncate().unwrap();
        assert_eq!(coll.count_embed_pending(), 0);
    }

    /// Card #986: an index is saved in pieces under a new generation each
    /// time, the older pieces deleted, and read back piece by piece; a
    /// dropped index leaves no piece.
    #[test]
    fn a_vector_index_is_saved_in_pieces() {
        use super::{chunked_header, vec_chunk_key};
        use crate::storage::keyspace::KsDbExt;
        use crate::storage::VectorIndexConfig;
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
        engine.create_database("d".to_string()).unwrap();
        let db = engine.get_database("d").unwrap();
        db.create_collection("c".to_string(), None).unwrap();
        let coll = db.get_collection("c").unwrap();
        // 11,000 vectors of 400 floats: 17.6 MB of arena, two pieces.
        let docs: Vec<_> = (0..11_000)
            .map(|i| {
                let v: Vec<f64> = (0..400)
                    .map(|j| (i as f64 * 0.37 + j as f64 * 1.3).sin())
                    .collect();
                json!({"_key": format!("k{i}"), "emb": v})
            })
            .collect();
        coll.insert_batch(docs).unwrap();
        coll.create_vector_index(VectorIndexConfig::new(
            "v".to_string(),
            "emb".to_string(),
            400,
        ))
        .unwrap();
        let cf = coll.ks.handle(&coll.db).unwrap();
        let header = |coll: &super::Collection| {
            let bytes = coll
                .db
                .get_ks(&cf, super::Collection::vec_data_key("v"))
                .unwrap()
                .unwrap();
            chunked_header(&bytes).unwrap()
        };
        let (first, pieces) = header(&coll);
        assert!(pieces >= 2, "{pieces} pieces");
        let query: Vec<f32> = (0..400)
            .map(|j| (1234.0f64 * 0.37 + j as f64 * 1.3).sin() as f32)
            .collect();
        let top = |coll: &super::Collection| -> Vec<String> {
            coll.vector_search("v", &query, 5, Some(64))
                .unwrap()
                .into_iter()
                .map(|r| r.doc_key)
                .collect()
        };
        let before = top(&coll);
        assert_eq!(before.len(), 5);

        coll.persist_vector_indexes().unwrap();
        let (second, _) = header(&coll);
        assert_eq!(second, first + 1);
        assert!(coll
            .db
            .get_ks(&cf, vec_chunk_key("v", first, 0))
            .unwrap()
            .is_none());
        assert!(coll
            .db
            .get_ks(&cf, vec_chunk_key("v", second, 0))
            .unwrap()
            .is_some());

        // Read back from the pieces.
        coll.vector_indexes.clear();
        assert_eq!(coll.get_vector_index("v").unwrap().len(), 11_000);
        assert_eq!(top(&coll), before);

        coll.drop_vector_index("v").unwrap();
        assert!(coll
            .db
            .get_ks(&cf, vec_chunk_key("v", second, 0))
            .unwrap()
            .is_none());
        assert!(coll
            .db
            .get_ks(&cf, vec_chunk_key("v", second, 1))
            .unwrap()
            .is_none());
    }
}
