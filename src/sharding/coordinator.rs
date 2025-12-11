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
    /// This node's HTTP address
    my_address: String,
    /// All node HTTP addresses (including self)
    node_addresses: std::sync::Arc<std::sync::RwLock<Vec<String>>>,
    /// Node health tracker for failover
    health: Option<super::health::NodeHealth>,
    /// Queue for failed operations to replay on recovery
    replication_queue: super::replication_queue::ReplicationQueue,
}

impl ShardCoordinator {
    /// Normalize address to handle localhost/0.0.0.0/127.0.0.1 aliasing
    fn normalize_address(addr: &str) -> String {
        let normalized = addr
            .replace("localhost:", "127.0.0.1:")
            .replace("0.0.0.0:", "127.0.0.1:");
        normalized
    }
    
    /// Create a new shard coordinator
    pub fn new(
        storage: Arc<StorageEngine>,
        my_address: String,
        node_addresses: Vec<String>,
    ) -> Self {
        // Create HTTP client with reasonable timeout
        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        let normalized_addresses: Vec<String> = node_addresses.into_iter()
            .map(|a| Self::normalize_address(&a))
            .collect();

        Self {
            storage,
            http_client,
            my_address: Self::normalize_address(&my_address),
            node_addresses: std::sync::Arc::new(std::sync::RwLock::new(normalized_addresses)),
            health: None,
            replication_queue: super::replication_queue::ReplicationQueue::new(),
        }
    }

    /// Create a new shard coordinator with health tracking enabled
    pub fn with_health_tracking(
        storage: Arc<StorageEngine>,
        my_address: String,
        node_addresses: Vec<String>,
        failure_threshold: u32,
    ) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        let normalized_addresses: Vec<String> = node_addresses.iter()
            .map(|a| Self::normalize_address(a))
            .collect();

        let health = super::health::NodeHealth::new(normalized_addresses.clone(), failure_threshold);

        Self {
            storage,
            http_client,
            my_address: Self::normalize_address(&my_address),
            node_addresses: std::sync::Arc::new(std::sync::RwLock::new(normalized_addresses)),
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
        let my_index = addresses.iter().position(|a| a == &self.my_address).unwrap_or(0);
        ShardRouter::is_shard_local(shard_id, my_index, addresses.len())
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

    /// Get the list of all node addresses in the cluster
    pub fn get_node_addresses(&self) -> Vec<String> {
        self.node_addresses.read().unwrap().clone()
    }

    /// Get this node's address
    pub fn my_address(&self) -> String {
        self.my_address.clone()
    }

    /// Get this node's index in the cluster
    pub fn get_node_index(&self) -> usize {
        self.node_addresses.read().unwrap()
            .iter()
            .position(|a| a == &self.my_address)
            .unwrap_or(0)
    }

    /// Get usage of the async HTTP client
    pub fn get_http_client(&self) -> &Client {
        &self.http_client
    }

    /// Add a new node to the cluster and trigger rebalancing for auto-sharded collections
    pub async fn add_node(&self, node_addr: &str) -> DbResult<()> {
        let should_rebalance = {
            let mut addresses = self.node_addresses.write().unwrap();
            let normalized = Self::normalize_address(node_addr);
            if !addresses.contains(&normalized) {
                addresses.push(normalized.clone());
                // Sort addresses to ensure consistent ring across cluster
                addresses.sort();
                tracing::info!("Added node {} to cluster (total: {})", normalized, addresses.len());
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

        tracing::info!(
            "[SHARD] key={}, shard={}, replicas={:?}, my_index={}",
            key, shard_id, replica_nodes, self.get_node_index()
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
        let my_addr = Some(self.my_address.clone());

        for node_addr in &replica_nodes {
            // Skip unhealthy remote nodes to avoid unnecessary timeouts
            // Always try local writes regardless of health status
            let is_local = Some(node_addr.to_string()) == my_addr;
            if !is_local && !self.is_node_healthy(node_addr) {
                tracing::debug!("[SHARD] Skipping unhealthy node {} for insert", node_addr);
                continue;
            }

            if is_local {
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

    /// Batch insert documents with sharding - groups by target node for efficiency
    /// Returns (success_count, error_count)
    pub async fn insert_batch(
        &self,
        db_name: &str,
        coll_name: &str,
        shard_config: &CollectionShardConfig,
        docs: Vec<Value>,
    ) -> DbResult<(usize, usize)> {
        if docs.is_empty() {
            return Ok((0, 0));
        }

        // Group documents by target nodes
        let mut node_docs: std::collections::HashMap<String, Vec<Value>> = std::collections::HashMap::new();
        let addresses = self.node_addresses.read().unwrap().clone();
        
        for mut doc in docs {
            // Get or generate key
            let key = if let Some(k) = doc.get("_key").and_then(|v| v.as_str()) {
                k.to_string()
            } else {
                let key = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
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
                &addresses,
            );

            // Add document to all replica nodes' queues
            for node in replica_nodes {
                node_docs.entry(node.to_string())
                    .or_insert_with(Vec::new)
                    .push(doc.clone());
            }
        }

        let mut success_count = 0usize;
        let mut error_count = 0usize;
        let my_addr = self.my_address.clone();
        let admin_pass = std::env::var("SOLIDB_ADMIN_PASSWORD").unwrap_or_else(|_| "admin".to_string());

        // Process each node's batch
        for (node_addr, docs) in node_docs {
            let batch_size = docs.len();
            
            if node_addr == my_addr {
                // Local batch insert
                match self.storage.get_database(db_name)
                    .and_then(|db| db.get_collection(coll_name))
                {
                    Ok(coll) => {
                        match coll.insert_batch(docs) {
                            Ok(inserted) => {
                                if let Err(e) = coll.index_documents(&inserted) {
                                    tracing::error!("Failed to index batch: {}", e);
                                }
                                success_count += inserted.len();
                            }
                            Err(e) => {
                                tracing::error!("[BATCH] Local batch insert failed: {}", e);
                                error_count += batch_size;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("[BATCH] Failed to get collection: {}", e);
                        error_count += batch_size;
                    }
                }
            } else {
                // Skip unhealthy nodes
                if !self.is_node_healthy(&node_addr) {
                    tracing::debug!("[BATCH] Skipping unhealthy node {}", node_addr);
                    continue; // Don't count as error - replica will sync later
                }

                // Remote batch insert via import endpoint
                let url = format!(
                    "http://{}/_api/database/{}/collection/{}/import",
                    node_addr, db_name, coll_name
                );

                // Convert docs to JSONL format
                let jsonl: String = docs.iter()
                    .filter_map(|d| serde_json::to_string(d).ok())
                    .collect::<Vec<_>>()
                    .join("\n");

                let client = reqwest::Client::new();
                let form = reqwest::multipart::Form::new()
                    .part("file", reqwest::multipart::Part::text(jsonl).file_name("batch.jsonl"));

                match client
                    .post(&url)
                    .basic_auth("admin", Some(&admin_pass))
                    .header("X-Shard-Direct", "true") // Mark as shard-directed to avoid re-routing
                    .multipart(form)
                    .timeout(std::time::Duration::from_secs(30))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        // Parse response to get actual counts
                        if let Ok(result) = resp.json::<serde_json::Value>().await {
                            let imported = result.get("imported").and_then(|v| v.as_u64()).unwrap_or(0);
                            let failed = result.get("failed").and_then(|v| v.as_u64()).unwrap_or(0);
                            success_count += imported as usize;
                            error_count += failed as usize;
                            self.mark_node_success(&node_addr);
                        } else {
                            success_count += batch_size;
                            self.mark_node_success(&node_addr);
                        }
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        tracing::error!("[BATCH] Remote batch failed {}: {} - {}", node_addr, status, body);
                        error_count += batch_size;
                        self.mark_node_failure(&node_addr);
                    }
                    Err(e) => {
                        tracing::error!("[BATCH] Remote batch failed {}: {}", node_addr, e);
                        error_count += batch_size;
                        self.mark_node_failure(&node_addr);
                    }
                }
            }
        }

        Ok((success_count, error_count))
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

    /// Get all unique nodes that hold shards for a collection (including replicas)
    pub fn get_collection_nodes(&self, config: &CollectionShardConfig) -> Vec<String> {
        let mut nodes = std::collections::HashSet::new();
        let addresses = self.node_addresses.read().unwrap();
        
        for shard_id in 0..config.num_shards {
            let replica_nodes = ShardRouter::shard_to_nodes(
                shard_id,
                config.replication_factor,
                &addresses,
            );
            for node in replica_nodes {
                nodes.insert(node.to_string());
            }
        }
        
        nodes.into_iter().collect()
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
            // 1. Remove from _system._config to prevent re-discovery
            let db = self.storage.get_database("_system")?;
            if let Ok(collection) = db.get_collection("_config") {
                 // Try to remove from persistent config
                 // We need to support various formats (API vs Repl address)
                 // But _sys._config stores REPLICATION addresses (e.g. 900x)
                 // node_addr is API address (e.g. 800x).
                 // We need to infer the replication address or just delete by matching host?
                 // Simple hack: Load doc, filter out any peer that maps to this API address?
                 // Or just assume single node per host in test?
                 // Better: We calculated offset in refresh_nodes. We can reverse it?
                 // But we don't store offset easily.
                 
                 // Let's try to find it in the doc by matching.
                 if let Ok(mut doc) = collection.get("cluster_peers") {
                     if let Some(peers_arr) = doc.data.get("peers").and_then(|v| v.as_array()).cloned() {
                         // Filter out the removed node
                         // We need to be careful with port mapping.
                         // But wait, if we remove it from memory, we should remove it from storage.
                         // Let's rely on string matching or try to locate it.
                         
                         // The storage has 127.0.0.1:9001. We have 127.0.0.1:8001.
                         // We need to match the HOST and port-offset.
                         // But we don't have the offset handy here? 
                         // Check refresh_nodes_from_storage logic again (it derives offset).
                         // We can re-derive it.
                         
                         let my_api_port = self.my_address.split(':').last()
                             .and_then(|p| p.parse::<u16>().ok())
                             .unwrap_or(0);
                             
                         let config = self.storage.cluster_config();
                         let offset = if let Some(conf) = config {
                             (conf.replication_port as i32) - (my_api_port as i32)
                         } else {
                             1000 // Default assumption
                         };
                         
                         let new_peers: Vec<Value> = peers_arr.iter().filter(|p| {
                             if let Some(p_str) = p.as_str() {
                                 // Convert replication addr p_str to API addr
                                 if let Some(port_start) = p_str.rfind(':') {
                                     let host = &p_str[..port_start];
                                     if let Ok(repl_port) = p_str[port_start+1..].parse::<u16>() {
                                         let api_port = (repl_port as i32 - offset) as u16;
                                         let api_addr = format!("{}:{}", host, api_port);
                                         let normalized = Self::normalize_address(&api_addr);
                                         
                                         // Compare with removed node_addr
                                          return normalized != node_addr && api_addr != node_addr;
                                     }
                                 }
                             }
                             true // Keep if parsing fails
                         }).cloned().collect();
                         
                         if new_peers.len() != peers_arr.len() {
                             if let Value::Object(ref mut map) = doc.data {
                                 map.insert("peers".to_string(), Value::Array(new_peers));
                             }
                             if let Err(e) = collection.update("cluster_peers", doc.data) {
                                  tracing::error!("Failed to persist node removal to _config: {}", e);
                             } else {
                                  tracing::info!("Persisted removal of {} from _system._config", node_addr);
                             }
                         }
                     }
                 }
            }
            
            // 2. Trigger data migration
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
                        let my_addr_opt = Some(self.my_address.clone());
                        
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
                                    
                                    // For prototype: Just send (using upsert for idempotency).
                                    match self.forward_upsert_to_node(
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

        // Get admin password from env var for inter-node auth
        let admin_pass = std::env::var("SOLIDB_ADMIN_PASSWORD").unwrap_or_else(|_| "admin".to_string());

        let response = self.http_client
            .post(&url)
            .header("X-Shard-Direct", "true")
            .basic_auth("admin", Some(&admin_pass))
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

    /// Forward upsert (insert or update) to a specific node - idempotent for rebalancing
    async fn forward_upsert_to_node(
        &self,
        node_addr: &str,
        db_name: &str,
        coll_name: &str,
        doc: &Value,
    ) -> DbResult<Document> {
        // Extract _key from document for PUT endpoint
        let key = doc.get("_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DbError::InternalError("Document missing _key".to_string()))?;

        let url = format!(
            "http://{}/_api/database/{}/document/{}/{}?upsert=true",
            node_addr, db_name, coll_name, key
        );

        // Get admin password from env var for inter-node auth
        let admin_pass = std::env::var("SOLIDB_ADMIN_PASSWORD").unwrap_or_else(|_| "admin".to_string());

        let response = self.http_client
            .put(&url)
            .header("X-Shard-Direct", "true")
            .basic_auth("admin", Some(&admin_pass))
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
                error["error"].as_str().unwrap_or("Remote upsert failed").to_string()
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

        // Get admin password from env var for inter-node auth
        let admin_pass = std::env::var("SOLIDB_ADMIN_PASSWORD").unwrap_or_else(|_| "admin".to_string());

        let response = self.http_client
            .post(&url)
            .header("X-Shard-Direct", "true") // Prevent re-routing
            .basic_auth("admin", Some(&admin_pass))
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
        let my_addr = Some(self.my_address.clone());
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

        // Get admin password from env var for inter-node auth
        let admin_pass = std::env::var("SOLIDB_ADMIN_PASSWORD").unwrap_or_else(|_| "admin".to_string());

        let response = self.http_client
            .get(&url)
            .header("X-Shard-Direct", "true")
            .basic_auth("admin", Some(&admin_pass))
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

        // Get admin password from env var for inter-node auth
        let admin_pass = std::env::var("SOLIDB_ADMIN_PASSWORD").unwrap_or_else(|_| "admin".to_string());

        let response = self.http_client
            .get(&url)
            .header("X-Shard-Direct", "true")
            .basic_auth("admin", Some(&admin_pass))
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

        // Get admin password from env var for inter-node auth
        let admin_pass = std::env::var("SOLIDB_ADMIN_PASSWORD").unwrap_or_else(|_| "admin".to_string());

        let response = self.http_client
            .put(&url)
            .header("X-Shard-Direct", "true")
            .basic_auth("admin", Some(&admin_pass))
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

        // Get admin password from env var for inter-node auth
        let admin_pass = std::env::var("SOLIDB_ADMIN_PASSWORD").unwrap_or_else(|_| "admin".to_string());

        let response = self.http_client
            .delete(&url)
            .header("X-Shard-Direct", "true")
            .basic_auth("admin", Some(&admin_pass))
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
        _db_name: &str,
        _coll_name: &str,
        shard_config: &CollectionShardConfig,
        local_fn: F,
    ) -> DbResult<Vec<T>>
    where
        F: Fn() -> DbResult<Vec<T>>,
        T: Send + 'static,
    {
        // For simplicity in v1: execute locally only
        // Full scatter-gather would query all nodes in parallel
        // TODO: Implement full scatter-gather for SDBQL queries
        
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
            // 1. Start health checker (runs in background, don't await)
            let health_clone = health.clone();
            tokio::spawn(async move {
                // This spawns the health check loop and immediately returns a JoinHandle
                // We ignore the handle - the loop runs forever in a separate task
                health_clone.start_health_checker(Duration::from_secs(2));
            });

            // 2. Start recovery, rebalance monitor, and node list refresh
            let self_clone = self.clone();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(5));
                loop {
                    ticker.tick().await;

                    // REFRESH NODES: Sync node list from _system._config
                    if let Err(e) = self_clone.refresh_nodes_from_storage() {
                        // This is expected on first startup before any sharded collections are created
                        tracing::debug!("Failed to refresh node list from storage: {}", e);
                    }
                    
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
                        
                        // DISABLED: Aggressive node removal was destroying clusters during warmup.
                        // Nodes marked unhealthy before a single health check passed were being removed.
                        // TODO: Implement proper dead-node detection with longer grace period.
                        // Only remove nodes that:
                        // 1. Are not ourselves (prevent suicide)
                        // 2. Were previously healthy (prevents removing during startup)
                        // 3. Are currently unhealthy (have failed threshold checks)
                        if node != self_clone.my_address {
                            if let Some(ref health) = self_clone.health {
                                let was_healthy = health.was_ever_healthy(&node);
                                let is_unhealthy = !self_clone.is_node_healthy(&node);
                                
                                if was_healthy && is_unhealthy {
                                    tracing::warn!("Node {} failed health checks, initiating removal", node);
                                    let _ = self_clone.remove_node(&node).await;
                                }
                            }
                        }
                    }
                }
            });
        }
    }

    /// Refresh node list from the persistent _system._config collection
    /// This ensures all nodes have a full view of the cluster, even if they started with a partial peer list
    fn refresh_nodes_from_storage(&self) -> DbResult<()> {
        let db = self.storage.get_database("_system")?;
        let collection = db.get_collection("_config")
             .map_err(|_| DbError::CollectionNotFound("_config".to_string()))?; // Quiet fail if not exists
             
        if let Ok(doc) = collection.get("cluster_peers") {
            if let Some(peers_arr) = doc.data.get("peers").and_then(|v| v.as_array()) {
                let mut new_addresses = Vec::new();
                
                // Get config for port offset calculation
                // Note: We need to derive API ports from replication addresses
                // We can infer the offset from our own configuration
                let config = self.storage.cluster_config().ok_or(DbError::InternalError("No cluster config".to_string()))?;
                
                // Parse my API port from my_address
                let my_api_port = self.my_address.split(':').last()
                    .and_then(|p| p.parse::<u16>().ok())
                    .ok_or(DbError::InternalError("Invalid my_address format".to_string()))?;
                
                // Calculate offset: offset = repl_port - api_port
                // e.g. repl=7745, api=6745 -> offset=1000
                let port_offset = (config.replication_port as i32) - (my_api_port as i32);
                
                for peer_val in peers_arr {
                    if let Some(peer_addr) = peer_val.as_str() {
                         // Convert replication address to API address
                         if let Some(port_start) = peer_addr.rfind(':') {
                             let host = &peer_addr[..port_start];
                             
                             if let Ok(repl_port) = peer_addr[port_start+1..].parse::<u16>() {
                                 let api_port = (repl_port as i32 - port_offset) as u16;
                                 let api_addr = format!("{}:{}", host, api_port);
                                 // Normalize the address before adding
                                 let normalized = Self::normalize_address(&api_addr);
                                 new_addresses.push(normalized.clone());
                                 tracing::debug!("Found peer from storage: {} -> API {}", peer_addr, normalized);
                             }
                         }
                    }
                }
                
                // Add self if missing (should be there, but safety first)
                if !new_addresses.contains(&self.my_address) {
                    new_addresses.push(self.my_address.clone());
                }
                
                new_addresses.sort();
                
                // Update if changed
                let mut current = self.node_addresses.write().unwrap();
                if *current != new_addresses {
                    tracing::info!("Updating cluster node list: {:?} -> {:?}", *current, new_addresses);
                    *current = new_addresses;
                    
                    // Update health tracker if present
                    if let Some(ref health) = self.health {
                        // TODO: efficiently update health tracker nodes
                        // For now we rely on add_node calls... but we are doing full refresh here.
                        // Actually NodeHealth doesn't support bulk replace easily.
                        // But we can just iterate and add any new ones.
                        // Removing old ones is trickier with current API.
                        // Since new nodes are main case, let's just add new ones.
                        for addr in current.iter() {
                             health.add_node(addr);
                        }
                    }
                }
            }
        }
        
        Ok(())
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
            let my_addr = Some(self.my_address.as_str());
            
            tracing::debug!("[SCAN] Shard {}: node={}, me={:?}", shard_id, node_addr, my_addr);
            
            if Some(*node_addr) == my_addr {
                // Local scan - query directly from storage
                tracing::debug!("[SCAN] Shard {} is LOCAL", shard_id);
                if let Ok(db) = self.storage.get_database(db_name) {
                    if let Ok(collection) = db.get_collection(coll_name) {
                        // For local shard, we need to scan only documents belonging to this shard
                        // For now, scan all and filter (TODO: optimize with shard-local index)
                        let docs = collection.scan(None);
                        let mut count = 0;
                        for doc in docs {
                            // Check if document belongs to this shard
                            let doc_shard = ShardRouter::route(&doc.key, config.num_shards);
                            if doc_shard == shard_id {
                                all_documents.push(doc);
                                count += 1;
                            }
                        }
                        tracing::debug!("[SCAN] Shard {} local scan found {} docs", shard_id, count);
                    }
                }
            } else {
                // Remote scan - query via HTTP
                let url = format!("http://{}/database/{}/shard-scan/{}/{}",
                    node_addr, db_name, coll_name, shard_id);
                
                tracing::debug!("[SCAN] Shard {} is REMOTE: {}", shard_id, url);
                
                // Get admin password from env var for inter-node auth
                let admin_pass = std::env::var("SOLIDB_ADMIN_PASSWORD").unwrap_or_else(|_| "admin".to_string());
                
                match self.http_client
                    .get(&url)
                    .header("X-Shard-Direct", "true")
                    .basic_auth("admin", Some(&admin_pass))
                    .send()
                    .await
                {
                    Ok(response) => {
                         if response.status().is_success() {
                            if let Ok(docs) = response.json::<Vec<Document>>().await {
                                tracing::debug!("[SCAN] Remote scan got {} docs", docs.len());
                                all_documents.extend(docs);
                            } else {
                                tracing::warn!("[SCAN] Failed to parse remote docs");
                            }
                        } else {
                            tracing::warn!("[SCAN] Remote scan failed: status {}", response.status());
                        }
                    },
                    Err(e) => {
                         tracing::warn!("[SCAN] Remote scan error: {}", e);
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
            .field("my_address", &self.my_address)
            .field("node_addresses", &self.node_addresses)
            .finish()
    }

}
