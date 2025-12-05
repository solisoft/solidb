//! Shard coordinator for routing document operations to correct nodes

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

use super::router::ShardRouter;
use crate::error::{DbError, DbResult};
use crate::storage::{Document, StorageEngine};

/// Collection sharding configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionShardConfig {
    /// Number of shards
    pub num_shards: u16,
    /// Field to use for sharding (default: "_key")
    pub shard_key: String,
    /// Replication factor: 1 = no replicas, 2 = 1 replica per shard, etc.
    #[serde(default = "default_replication_factor")]
    pub replication_factor: u16,
}

fn default_replication_factor() -> u16 {
    1
}

impl Default for CollectionShardConfig {
    fn default() -> Self {
        Self {
            num_shards: 3,
            shard_key: "_key".to_string(),
            replication_factor: 1,
        }
    }
}

/// Coordinates shard-aware document operations
/// Coordinates shard-aware document operations
#[derive(Clone)]
pub struct ShardCoordinator {
    storage: Arc<StorageEngine>,
    http_client: Client,
    /// This node's index in the cluster (0-based)
    node_index: usize,
    /// All node HTTP addresses (including self)
    node_addresses: Vec<String>,
    /// Node health tracker for failover
    health: Option<super::health::NodeHealth>,
    /// Queue for failed operations to replay on recovery
    replication_queue: super::replication_queue::ReplicationQueue,
}

impl ShardCoordinator {
    /// Create a new shard coordinator
    pub fn new(
        storage: Arc<StorageEngine>,
        node_index: usize,
        node_addresses: Vec<String>,
    ) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            storage,
            http_client,
            node_index,
            node_addresses,
            health: None,
            replication_queue: super::replication_queue::ReplicationQueue::new(),
        }
    }

    /// Create a new shard coordinator with health tracking enabled
    pub fn with_health_tracking(
        storage: Arc<StorageEngine>,
        node_index: usize,
        node_addresses: Vec<String>,
        failure_threshold: u32,
    ) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        let health = super::health::NodeHealth::new(node_addresses.clone(), failure_threshold);

        Self {
            storage,
            http_client,
            node_index,
            node_addresses,
            health: Some(health),
            replication_queue: super::replication_queue::ReplicationQueue::new(),
        }
    }

    /// Get health tracker (for starting background health checker)
    pub fn health_tracker(&self) -> Option<&super::health::NodeHealth> {
        self.health.as_ref()
    }

    /// Check if a node is healthy (always true if no health tracking)
    fn is_node_healthy(&self, node_addr: &str) -> bool {
        self.health.as_ref().map(|h| h.is_healthy(node_addr)).unwrap_or(true)
    }

    /// Mark a node as succeeded
    fn mark_node_success(&self, node_addr: &str) {
        if let Some(ref health) = self.health {
            health.mark_success(node_addr);
        }
    }

    /// Mark a node as failed
    fn mark_node_failure(&self, node_addr: &str) {
        if let Some(ref health) = self.health {
            health.mark_failure(node_addr);
        }
    }

    /// Get document key from value (for routing)
    fn get_shard_key(doc: &Value, shard_key: &str) -> Option<String> {
        if shard_key == "_key" {
            doc.get("_key").and_then(|v| v.as_str()).map(|s| s.to_string())
        } else {
            doc.get(shard_key).and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
        }
    }

    /// Check if shard is local to this node
    fn is_local(&self, shard_id: u16) -> bool {
        ShardRouter::is_shard_local(shard_id, self.node_index, self.node_addresses.len())
    }

    /// Get the HTTP address for a shard
    fn get_shard_address(&self, shard_id: u16) -> Option<&str> {
        ShardRouter::shard_to_node(shard_id, &self.node_addresses)
    }

    /// Insert document, routing to correct shard with replication
    pub async fn insert(
        &self,
        db_name: &str,
        coll_name: &str,
        shard_config: &CollectionShardConfig,
        mut doc: Value,
    ) -> DbResult<Document> {
        // Generate key if not present
        let key = if let Some(key) = Self::get_shard_key(&doc, &shard_config.shard_key) {
            key
        } else {
            // Generate UUIDv7 key
            let key = uuid7::uuid7().to_string();
            if let Value::Object(ref mut obj) = doc {
                obj.insert("_key".to_string(), Value::String(key.clone()));
            }
            key
        };

        // Route to shard
        let shard_id = ShardRouter::route(&key, shard_config.num_shards);

        // Get all replica nodes for this shard
        let replica_nodes = ShardRouter::shard_to_nodes(
            shard_id,
            shard_config.replication_factor,
            &self.node_addresses,
        );

        // If RF=1 or single node, use original logic
        if replica_nodes.len() <= 1 {
            if self.is_local(shard_id) {
                let collection = self.storage
                    .get_database(db_name)?
                    .get_collection(coll_name)?;
                return collection.insert(doc);
            } else {
                return self.forward_insert(db_name, coll_name, shard_id, doc).await;
            }
        }

        // Replicated write: write to all replicas
        let mut primary_result: Option<Document> = None;
        let mut errors: Vec<String> = Vec::new();
        let my_addr = self.node_addresses.get(self.node_index)
            .map(|s| s.as_str());

        for node_addr in &replica_nodes {
            if Some(*node_addr) == my_addr {
                // Local insert
                match self.storage.get_database(db_name)
                    .and_then(|db| db.get_collection(coll_name))
                    .and_then(|coll| coll.insert(doc.clone()))
                {
                    Ok(doc) => {
                        if primary_result.is_none() {
                            primary_result = Some(doc);
                        }
                        self.mark_node_success(node_addr);
                    }
                    Err(e) => {
                        errors.push(format!("{}: {}", node_addr, e));
                        self.mark_node_failure(node_addr);
                    }
                }
            } else {
                // Remote insert
                match self.forward_insert_to_node(node_addr, db_name, coll_name, &doc).await {
                    Ok(doc) => {
                        if primary_result.is_none() {
                            primary_result = Some(doc);
                        }
                        self.mark_node_success(node_addr);
                    }
                    Err(e) => {
                        errors.push(format!("{}: {}", node_addr, e));
                        self.mark_node_failure(node_addr);
                        
                        // Queue failed write for recovery
                        self.replication_queue.push(
                            node_addr,
                            super::replication_queue::FailedOperation {
                                db_name: db_name.to_string(),
                                collection: coll_name.to_string(),
                                doc: doc.clone(),
                                timestamp: std::time::Instant::now(),
                            }
                        );
                    }
                }
            }
        }

        // Return success if at least one replica succeeded
        if let Some(doc) = primary_result {
            Ok(doc)
        } else {
            Err(DbError::InternalError(
                format!("All replicas failed: {}", errors.join(", "))
            ))
        }
    }

    /// Forward insert to a specific node
    async fn forward_insert_to_node(
        &self,
        node_addr: &str,
        db_name: &str,
        coll_name: &str,
        doc: &Value,
    ) -> DbResult<Document> {
        let url = format!(
            "http://{}/_api/database/{}/document/{}",
            node_addr, db_name, coll_name
        );

        let response = self.http_client
            .post(&url)
            .header("X-Shard-Direct", "true")
            .json(doc)
            .send()
            .await
            .map_err(|e| DbError::InternalError(format!("Forward failed: {}", e)))?;

        if response.status().is_success() {
            let result: Document = response.json().await
                .map_err(|e| DbError::InternalError(format!("Parse response: {}", e)))?;
            Ok(result)
        } else {
            let error: Value = response.json().await.unwrap_or(Value::Null);
            Err(DbError::InternalError(
                error["error"].as_str().unwrap_or("Remote insert failed").to_string()
            ))
        }
    }

    /// Forward insert to remote node
    async fn forward_insert(
        &self,
        db_name: &str,
        coll_name: &str,
        shard_id: u16,
        doc: Value,
    ) -> DbResult<Document> {
        let addr = self.get_shard_address(shard_id)
            .ok_or_else(|| DbError::InternalError("No node for shard".to_string()))?;

        let url = format!(
            "http://{}/_api/database/{}/document/{}",
            addr, db_name, coll_name
        );

        let response = self.http_client
            .post(&url)
            .header("X-Shard-Direct", "true") // Prevent re-routing
            .json(&doc)
            .send()
            .await
            .map_err(|e| DbError::InternalError(format!("Forward failed: {}", e)))?;

        if response.status().is_success() {
            let result: Document = response.json().await
                .map_err(|e| DbError::InternalError(format!("Parse response: {}", e)))?;
            Ok(result)
        } else {
            let error: Value = response.json().await.unwrap_or(Value::Null);
            Err(DbError::InternalError(
                error["error"].as_str().unwrap_or("Remote insert failed").to_string()
            ))
        }
    }

    /// Get document from shard with read failover
    pub async fn get(
        &self,
        db_name: &str,
        coll_name: &str,
        shard_config: &CollectionShardConfig,
        key: &str,
    ) -> DbResult<Document> {
        let shard_id = ShardRouter::route(key, shard_config.num_shards);

        // Get all replica nodes for failover
        let replica_nodes = ShardRouter::shard_to_nodes(
            shard_id,
            shard_config.replication_factor,
            &self.node_addresses,
        );

        // If RF=1 or no health tracking, use simple logic
        if replica_nodes.len() <= 1 || self.health.is_none() {
            if self.is_local(shard_id) {
                let collection = self.storage
                    .get_database(db_name)?
                    .get_collection(coll_name)?;
                return collection.get(key);
            } else {
                return self.forward_get(db_name, coll_name, shard_id, key).await;
            }
        }

        // Try each replica in order until one succeeds (failover)
        let my_addr = self.node_addresses.get(self.node_index)
            .map(|s| s.as_str());
        let mut last_error: Option<DbError> = None;

        for node_addr in &replica_nodes {
            // Skip unhealthy nodes
            if !self.is_node_healthy(node_addr) {
                continue;
            }

            if Some(*node_addr) == my_addr {
                // Try local get
                match self.storage.get_database(db_name)
                    .and_then(|db| db.get_collection(coll_name))
                    .and_then(|coll| coll.get(key))
                {
                    Ok(doc) => {
                        self.mark_node_success(node_addr);
                        return Ok(doc);
                    }
                    Err(e) => {
                        self.mark_node_failure(node_addr);
                        last_error = Some(e);
                    }
                }
            } else {
                // Try remote get
                match self.forward_get_to_node(node_addr, db_name, coll_name, key).await {
                    Ok(doc) => {
                        self.mark_node_success(node_addr);
                        return Ok(doc);
                    }
                    Err(e) => {
                        self.mark_node_failure(node_addr);
                        last_error = Some(e);
                    }
                }
            }
        }

        // All replicas failed
        Err(last_error.unwrap_or_else(|| 
            DbError::InternalError("All replicas unavailable".to_string())
        ))
    }

    /// Forward get to a specific node
    async fn forward_get_to_node(
        &self,
        node_addr: &str,
        db_name: &str,
        coll_name: &str,
        key: &str,
    ) -> DbResult<Document> {
        let url = format!(
            "http://{}/_api/database/{}/document/{}/{}",
            node_addr, db_name, coll_name, key
        );

        let response = self.http_client
            .get(&url)
            .header("X-Shard-Direct", "true")
            .send()
            .await
            .map_err(|e| DbError::InternalError(format!("Forward failed: {}", e)))?;

        if response.status().is_success() {
            let result: Document = response.json().await
                .map_err(|e| DbError::InternalError(format!("Parse response: {}", e)))?;
            Ok(result)
        } else if response.status().as_u16() == 404 {
            Err(DbError::DocumentNotFound(key.to_string()))
        } else {
            let error: Value = response.json().await.unwrap_or(Value::Null);
            Err(DbError::InternalError(
                error["error"].as_str().unwrap_or("Remote get failed").to_string()
            ))
        }
    }

    /// Forward get to remote node
    async fn forward_get(
        &self,
        db_name: &str,
        coll_name: &str,
        shard_id: u16,
        key: &str,
    ) -> DbResult<Document> {
        let addr = self.get_shard_address(shard_id)
            .ok_or_else(|| DbError::InternalError("No node for shard".to_string()))?;

        let url = format!(
            "http://{}/_api/database/{}/document/{}/{}",
            addr, db_name, coll_name, key
        );

        let response = self.http_client
            .get(&url)
            .header("X-Shard-Direct", "true")
            .send()
            .await
            .map_err(|e| DbError::InternalError(format!("Forward failed: {}", e)))?;

        if response.status().is_success() {
            let result: Document = response.json().await
                .map_err(|e| DbError::InternalError(format!("Parse response: {}", e)))?;
            Ok(result)
        } else if response.status().as_u16() == 404 {
            Err(DbError::DocumentNotFound(key.to_string()))
        } else {
            let error: Value = response.json().await.unwrap_or(Value::Null);
            Err(DbError::InternalError(
                error["error"].as_str().unwrap_or("Remote get failed").to_string()
            ))
        }
    }

    /// Update document in correct shard
    pub async fn update(
        &self,
        db_name: &str,
        coll_name: &str,
        shard_config: &CollectionShardConfig,
        key: &str,
        changes: Value,
    ) -> DbResult<Document> {
        let shard_id = ShardRouter::route(key, shard_config.num_shards);

        if self.is_local(shard_id) {
            let collection = self.storage
                .get_database(db_name)?
                .get_collection(coll_name)?;
            collection.update(key, changes)
        } else {
            self.forward_update(db_name, coll_name, shard_id, key, changes).await
        }
    }

    /// Forward update to remote node
    async fn forward_update(
        &self,
        db_name: &str,
        coll_name: &str,
        shard_id: u16,
        key: &str,
        changes: Value,
    ) -> DbResult<Document> {
        let addr = self.get_shard_address(shard_id)
            .ok_or_else(|| DbError::InternalError("No node for shard".to_string()))?;

        let url = format!(
            "http://{}/_api/database/{}/document/{}/{}",
            addr, db_name, coll_name, key
        );

        let response = self.http_client
            .put(&url)
            .header("X-Shard-Direct", "true")
            .json(&changes)
            .send()
            .await
            .map_err(|e| DbError::InternalError(format!("Forward failed: {}", e)))?;

        if response.status().is_success() {
            let result: Document = response.json().await
                .map_err(|e| DbError::InternalError(format!("Parse response: {}", e)))?;
            Ok(result)
        } else {
            let error: Value = response.json().await.unwrap_or(Value::Null);
            Err(DbError::InternalError(
                error["error"].as_str().unwrap_or("Remote update failed").to_string()
            ))
        }
    }

    /// Delete document from correct shard
    pub async fn delete(
        &self,
        db_name: &str,
        coll_name: &str,
        shard_config: &CollectionShardConfig,
        key: &str,
    ) -> DbResult<()> {
        let shard_id = ShardRouter::route(key, shard_config.num_shards);

        if self.is_local(shard_id) {
            let collection = self.storage
                .get_database(db_name)?
                .get_collection(coll_name)?;
            collection.delete(key)
        } else {
            self.forward_delete(db_name, coll_name, shard_id, key).await
        }
    }

    /// Forward delete to remote node
    async fn forward_delete(
        &self,
        db_name: &str,
        coll_name: &str,
        shard_id: u16,
        key: &str,
    ) -> DbResult<()> {
        let addr = self.get_shard_address(shard_id)
            .ok_or_else(|| DbError::InternalError("No node for shard".to_string()))?;

        let url = format!(
            "http://{}/_api/database/{}/document/{}/{}",
            addr, db_name, coll_name, key
        );

        let response = self.http_client
            .delete(&url)
            .header("X-Shard-Direct", "true")
            .send()
            .await
            .map_err(|e| DbError::InternalError(format!("Forward failed: {}", e)))?;

        if response.status().is_success() || response.status().as_u16() == 204 {
            Ok(())
        } else {
            let error: Value = response.json().await.unwrap_or(Value::Null);
            Err(DbError::InternalError(
                error["error"].as_str().unwrap_or("Remote delete failed").to_string()
            ))
        }
    }

    /// Scatter-gather query across all shards (for non-indexed scans)
    pub async fn scatter_gather<F, T>(
        &self,
        db_name: &str,
        coll_name: &str,
        shard_config: &CollectionShardConfig,
        local_fn: F,
    ) -> DbResult<Vec<T>>
    where
        F: Fn() -> DbResult<Vec<T>>,
        T: Send + 'static,
    {
        // For simplicity in v1: execute locally only
        // Full scatter-gather would query all nodes in parallel
        // TODO: Implement full scatter-gather for AQL queries
        
        // Check which shards are local and execute
        let mut results = Vec::new();
        
        for shard_id in 0..shard_config.num_shards {
            if self.is_local(shard_id) {
                let local_results = local_fn()?;
                results.extend(local_results);
            }
        }
        
        Ok(results)
    }

    /// Recover a node by replaying queued operations
    /// Returns number of operations replayed
    pub async fn recover_node(&self, node_addr: &str) -> usize {
        let ops = self.replication_queue.pop_all(node_addr);
        if ops.is_empty() {
            return 0;
        }

        let total = ops.len();
        tracing::info!("Recovering node {}: replaying {} operations", node_addr, total);

        for op in ops {
            // Replay insert
            if let Err(e) = self.forward_insert_to_node(
                node_addr,
                &op.db_name,
                &op.collection,
                &op.doc
            ).await {
                tracing::error!("Failed to replay op to {}: {}", node_addr, e);
                // Re-queue at back of queue to avoid data loss
                self.replication_queue.push(node_addr, op);
            }
        }
        
        total
    }

    /// Start background tasks (health check + recovery monitor)
    pub fn start_background_tasks(self: Arc<Self>) {
        if let Some(health) = &self.health {
            // 1. Start health checker
            let health_clone = health.clone();
            tokio::spawn(async move {
                let _ = health_clone.start_health_checker(Duration::from_secs(5)).await;
            });

            // 2. Start recovery monitor
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(5));
                loop {
                    ticker.tick().await;
                    
                    // Check all nodes that have pending ops
                    let nodes = self.node_addresses.clone();
                    for node in nodes {
                        if self.replication_queue.has_pending(&node) && self.is_node_healthy(&node) {
                            // Node has pending ops and is healthy -> Synchronize
                            self.recover_node(&node).await;
                        }
                    }
                }
            });
        }
    }
}

impl std::fmt::Debug for ShardCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShardCoordinator")
            .field("node_index", &self.node_index)
            .field("node_addresses", &self.node_addresses)
            .finish()
    }
}
