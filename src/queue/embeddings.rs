//! Background auto-embedding worker.
//!
//! When a document is inserted/updated whose collection has a vector index with
//! `embedding_source` set but no concrete vector in the target field, the storage
//! layer records a cheap "pending embed" marker (no network on the write path,
//! see `Collection::mark_embed_pending`). This worker sweeps those markers,
//! generates embeddings via the configured LLM provider (async, batched), and
//! writes the vector back into the document — which persists it and updates the
//! HNSW index. This makes "just insert text" work on every write path (HTTP,
//! driver, Lua, bulk, replication) without ever blocking a write on the network.

use super::QueueWorker;
use crate::error::DbError;
use crate::server::llm_client::LLMClient;
use crate::storage::collection::vector::pending_embed_count;
use crate::storage::index::{extract_field_value, VectorIndexConfig};
use crate::storage::Collection;
use std::sync::atomic::{AtomicU64, Ordering};

/// Max documents embedded per (collection, index) per sweep — bounds a single
/// batch request and keeps one collection from starving the others.
const EMBED_BATCH: usize = 128;

/// Backoff (seconds) after a provider/config failure so we don't hammer a
/// down or misconfigured provider every worker tick.
const ERROR_BACKOFF_SECS: u64 = 60;

/// Unix-seconds before which embedding sweeps are skipped (set after an error).
static RETRY_AFTER: AtomicU64 = AtomicU64::new(0);

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl QueueWorker {
    /// One embedding sweep. Called from the worker loop alongside `check_jobs`.
    pub(crate) async fn check_embeddings(&self) {
        // Fast path: nothing pending anywhere → no enumeration at all.
        if pending_embed_count() == 0 {
            return;
        }
        // Respect backoff after a recent provider failure.
        if now_secs() < RETRY_AFTER.load(Ordering::Relaxed) {
            return;
        }

        for db_name in self.storage.list_databases() {
            let db = match self.storage.get_database(&db_name) {
                Ok(d) => d,
                Err(_) => continue,
            };
            for coll_name in db.list_collections() {
                let coll = match db.get_collection(&coll_name) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let configs = coll.get_all_vector_index_configs();
                for config in configs {
                    if config.embedding_source.is_none() {
                        continue;
                    }
                    let pending = coll.list_embed_pending(&config.name, EMBED_BATCH);
                    if pending.is_empty() {
                        continue;
                    }
                    if let Err(e) = self
                        .embed_pending_batch(&db_name, &coll, &config, &pending)
                        .await
                    {
                        // Provider/config problem — back off and stop this sweep.
                        // Markers remain and are retried after the backoff.
                        tracing::warn!(
                            "Auto-embed worker: {}/{} index '{}' failed: {} (backing off {}s)",
                            db_name,
                            coll_name,
                            config.name,
                            e,
                            ERROR_BACKOFF_SECS
                        );
                        RETRY_AFTER.store(now_secs() + ERROR_BACKOFF_SECS, Ordering::Relaxed);
                        return;
                    }
                }
            }
        }
    }

    /// Embed one batch of pending docs for a single index and persist the vectors.
    async fn embed_pending_batch(
        &self,
        db_name: &str,
        coll: &Collection,
        config: &VectorIndexConfig,
        doc_keys: &[String],
    ) -> Result<(), DbError> {
        let source_field = config.embedding_source.as_deref().unwrap_or_default();

        // Gather (doc_key, source_text), dropping stale markers for docs that were
        // deleted or no longer carry source text.
        let mut keys: Vec<String> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        for dk in doc_keys {
            let doc = match coll.get(dk) {
                Ok(d) => d,
                Err(_) => {
                    coll.clear_embed_pending(&config.name, dk);
                    continue;
                }
            };
            let value = doc.to_value();
            match extract_field_value(&value, source_field).as_str() {
                Some(t) if !t.trim().is_empty() => {
                    keys.push(dk.clone());
                    texts.push(t.to_string());
                }
                _ => coll.clear_embed_pending(&config.name, dk),
            }
        }
        if keys.is_empty() {
            return Ok(());
        }

        // Embeddings default to OpenAI; an index may override via embedding_provider.
        let provider = config
            .embedding_provider
            .clone()
            .unwrap_or_else(|| "openai".to_string());
        let client = LLMClient::from_storage(
            &self.storage,
            db_name,
            Some(&provider),
            config.embedding_model.clone(),
        )?;

        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let embeddings = client.embed_batch(&text_refs).await?;

        // Write each vector back into its document. `update()` re-runs
        // update_vector_indexes_on_upsert, which — now that the vector is present —
        // indexes the doc and clears the pending marker.
        for (dk, emb) in keys.iter().zip(embeddings) {
            if emb.len() != config.dimension {
                tracing::warn!(
                    "Auto-embed worker: dim mismatch for '{}' index '{}' (got {}, expected {}); dropping marker",
                    dk,
                    config.name,
                    emb.len(),
                    config.dimension
                );
                coll.clear_embed_pending(&config.name, dk);
                continue;
            }
            let doc = match coll.get(dk) {
                Ok(d) => d,
                Err(_) => {
                    coll.clear_embed_pending(&config.name, dk);
                    continue;
                }
            };
            let mut value = doc.to_value();
            if let Some(obj) = value.as_object_mut() {
                obj.insert(config.field.clone(), serde_json::json!(emb));
                if let Err(e) = coll.update(dk, value) {
                    // Leave the marker in place for a later retry.
                    tracing::warn!(
                        "Auto-embed worker: failed to persist vector for '{}': {}",
                        dk,
                        e
                    );
                }
            }
        }
        Ok(())
    }
}
