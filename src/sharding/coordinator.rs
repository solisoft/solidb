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
    node_addresses: std::sync::Arc<std::sync::RwLock<Vec<String>>>,
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
            node_addresses: std::sync::Arc::new(std::sync::RwLock::new(node_addresses)),
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
            node_addresses: std::sync::Arc::new(std::sync::RwLock::new(node_addresses)),
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

    /// Check if this node is responsible for the shard
    pub fn is_local(&self, shard_id: u16) -> bool {
        let addresses = self.node_addresses.read().unwrap();
        ShardRouter::is_shard_local(shard_id, self.node_index, addresses.len())
    }

    /// Get address of node responsible for shard
    pub fn get_shard_address(&self, shard_id: u16) -> Option<String> {
        let addresses = self.node_addresses.read().unwrap();
        ShardRouter::shard_to_node(shard_id, &addresses).map(|s| s.to_string())
    }

    /// Get the current number of nodes in the cluster
    pub fn get_node_count(&self) -> usize {
        self.node_addresses.read().unwrap().len()
    }

    /// Add a new node to the cluster and trigger rebalancing for auto-sharded collections
    pub async fn add_node(&self, node_addr: &str) -> DbResult<()> {
        let should_rebalance = {
            let mut addresses = self.node_addresses.write().unwrap();
            if !addresses.contains(&node_addr.to_string()) {
                println!("Configuration Change: Adding node {} to cluster", node_addr);
                addresses.push(node_addr.to_string());
                true
            } else {
                false
            }
        };

        if should_rebalance {
            // Update health tracker if present
            if let Some(ref health) = self.health {
                health.add_node(node_addr);
            }
            // Trigger data migration for auto-sharded collections
            self.rebalance().await?;
        }
        Ok(())
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

        // Route to shard (use effective shard count for auto-sharding)
        let shard_id = ShardRouter::route(&key, shard_config.num_shards);

        // Determine replicas
        // Create scope for lock
        let replica_nodes = {
            let addresses = self.node_addresses.read().unwrap();
            ShardRouter::shard_to_nodes(
                shard_id,
                shard_config.replication_factor,
                &addresses,
            ).into_iter().map(|s| s.to_string()).collect::<Vec<String>>()
        };

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
        let my_addr = self.node_addresses.read().unwrap().get(self.node_index)
            .map(|s| s.clone());

        for node_addr in &replica_nodes {
            if Some(node_addr.to_string()) == my_addr {
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

        if let Some(doc) = primary_result {
            Ok(doc)
        } else {
            Err(DbError::InternalError(
                format!("All replicas failed: {}", errors.join(", "))
            ))
        }
    }

    /// Get the list of replica nodes for a given document key
    pub fn get_replicas(&self, key: &str, config: &CollectionShardConfig) -> Vec<String> {
        let shard_id = ShardRouter::route(key, config.num_shards);
        ShardRouter::shard_to_nodes(
            shard_id,
            config.replication_factor,
            &self.node_addresses.read().unwrap(),
        ).into_iter().map(|s| s.to_string()).collect()
    }

    /// Remove a node from the cluster and rebalance
    pub async fn remove_node(&self, node_addr: &str) -> DbResult<()> {
        let should_rebalance = {
            let mut addresses = self.node_addresses.write().unwrap();
            if let Some(pos) = addresses.iter().position(|x| x == node_addr) {
                println!("Configuration Change: Removing node {} from cluster", node_addr);
                addresses.remove(pos);
                true
            } else {
                false
            }
        };

        if should_rebalance {
            // Trigger data migration
            self.rebalance().await?;
        }
        Ok(())
    }

    /// Rebalance data across the new topology
    pub async fn rebalance(&self) -> DbResult<()> {
        println!("Starting cluster rebalancing...");
        
        // 1. Get all databases
        let databases = self.storage.list_databases();
        
        for db_name in databases {
            let db = self.storage.get_database(&db_name)?;
            let collections = db.list_collections();
            
            for coll_name in collections {
                let collection = db.get_collection(&coll_name)?;
                
                // Only rebalance sharded collections
                if let Some(config) = collection.get_shard_config() {
                    println!("Rebalancing collection: {}/{}", db_name, coll_name);
                    
                    // Iterate all documents
                    // efficient iteration needed. For now getting all IDs.
                    // This is heavy! In prod we'd stream.
                    let all_docs = collection.scan(None); 
                    
                    for doc in all_docs {
                        let key = doc.key.clone();
                        
                        // Calculate NEW replicas
                        let replicas = self.get_replicas(&key, &config);
                        
                        // Check if I am responsible for this doc (either primary or replica)
                        let my_addr_opt = self.node_addresses.read().unwrap().get(self.node_index).cloned();
                        
                        if let Some(my_addr) = my_addr_opt {
                            for target in replicas {
                                if target != my_addr {
                                    // I should ensure this target has the data.
                                    // Simple approach: Send blindly (idempotent upsert).
                                    // Optimization: Check if target has it? (Too slow).
                                    // Optimization: Only send if I WAS primary? 
                                    // With Mod-N shuffle, I might have just BECOME primary/replica.
                                    // Safer to just gossip: If I hold data, and target is in replica set, send it.
                                    
                                    // To avoid storm: Only send if I am the FIRST available node in the replica list?
                                    // (Primary responsibility).
                                    
                                    // For prototype: Just send.
                                    match self.forward_insert_to_node(
                                        &target, &db_name, &coll_name, &doc.to_value()
                                    ).await {
                                        Ok(_) => {}, // Success
                                        Err(e) => println!("Rebalance push error to {}: {}", target, e),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        println!("Rebalancing complete.");
        Ok(())
    }


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
        let shard_id = ShardRouter::route(&key, shard_config.num_shards);

        // Get all replica nodes for failover
        // Create scope for lock
        let replica_nodes = {
            let addresses = self.node_addresses.read().unwrap();
            ShardRouter::shard_to_nodes(
                shard_id,
                shard_config.replication_factor,
                &addresses,
            ).into_iter().map(|s| s.to_string()).collect::<Vec<String>>()
        };

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
        let my_addr = self.node_addresses.read().unwrap().get(self.node_index)
            .map(|s| s.clone());
        let mut last_error: Option<DbError> = None;

        for node_addr in &replica_nodes {
            // Skip unhealthy nodes
            if !self.is_node_healthy(node_addr) {
                continue;
            }

            if Some(node_addr.clone()) == my_addr {
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
        let shard_id = ShardRouter::route(&key, shard_config.num_shards);

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
        let shard_id = ShardRouter::route(&key, shard_config.num_shards);

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

            // 2. Start recovery and rebalance monitor
            let self_clone = self.clone();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(5));
                loop {
                    ticker.tick().await;
                    
                    // Check all known nodes
                    // We need a snapshot of nodes checking health
                    // Note: If we remove a node, iteration changes. 
                    // So we iterate a CLONE of addresses.
                    let nodes = self_clone.node_addresses.read().unwrap().clone();
                    
                    for node in nodes {
                        // RECOVERY: Sync if healthy and has pending ops
                        if self_clone.replication_queue.has_pending(&node) && self_clone.is_node_healthy(&node) {
                            self_clone.recover_node(&node).await;
                        }

                        // REBALANCING: Check if node is DEAD (e.g. not healthy)
                        // In real system we'd check `consecutive_failures` via health.nodes()
                        // Here we assume `!is_node_healthy` for X checks means dead?
                        // `NodeHealth` marks healthy=false after threshold.
                        // So if !healthy, we could assume Dead?
                        // But wait, "Unhealthy" vs "Dead".
                        // User said: "remove dead node & migrate data".
                        // If we remove quickly, we might flap. 
                        // Let's assume `is_healthy` false IS the signal (threshold reached).
                         
                        // DANGER: We can't distinguish "Transient" from "Dead" easily without more state.
                        // BUT `NodeHealth` uses `failure_threshold`. So if is_healthy is false, it met the threshold.
                        // We will act on it. WARNING: This effectively removes any node that flaps.
                        // Ideally we'd have a separate "DEAD" state.
                        // For this task, let's treat `!is_healthy` as trigger.
                        
                        if !self_clone.is_node_healthy(&node) {
                             // Check if we already removed it? 
                             // We are iterating `nodes` copy. If it's in list, it's active.
                             // Trigger removal.
                             let _ = self_clone.remove_node(&node).await;
                        }
                    }
                }
            });
        }
    }

    /// Scan all shards across all nodes (scatter-gather query)
    /// This method queries each shard and merges results for a complete dataset
    pub async fn scan_all_shards(
        &self,
        db_name: &str,
        coll_name: &str,
        config: &CollectionShardConfig,
    ) -> DbResult<Vec<Document>> {
        let mut all_documents = Vec::new();
        let nodes = self.node_addresses.read().unwrap();
        
        // Query each shard (one representative per shard)
        for shard_id in 0..config.num_shards {
            // Get primary node for this shard
            let node_addrs = ShardRouter::shard_to_nodes(
                shard_id,
                config.replication_factor,
                &nodes,
            );
            
            let node_addr = node_addrs.first()
                .ok_or_else(|| DbError::InternalError(format!("No node for shard {}", shard_id)))?;

            // Check if this is local or remote
            let my_addr = nodes.get(self.node_index).map(|s| s.as_str());
            
            if Some(*node_addr) == my_addr {
                // Local scan - query directly from storage
                if let Ok(db) = self.storage.get_database(db_name) {
                    if let Ok(collection) = db.get_collection(coll_name) {
                        // For local shard, we need to scan only documents belonging to this shard
                        // For now, scan all and filter (TODO: optimize with shard-local index)
                        let docs = collection.scan(None);
                        for doc in docs {
                            // Check if document belongs to this shard
                            let doc_shard = ShardRouter::route(&doc.key, config.num_shards);
                            if doc_shard == shard_id {
                                all_documents.push(doc);
                            }
                        }
                    }
                }
            } else {
                // Remote scan - query via HTTP
                let url = format!("{}/database/{}/shard-scan/{}/{}",
                    node_addr, db_name, coll_name, shard_id);
                
                if let Ok(response) = self.http_client
                    .get(&url)
                    .header("X-Shard-Direct", "true")
                    .send()
                    .await
                {
                    if response.status().is_success() {
                        if let Ok(docs) = response.json::<Vec<Document>>().await {
                            all_documents.extend(docs);
                        }
                    }
                }
            }
        }

        Ok(all_documents)
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
