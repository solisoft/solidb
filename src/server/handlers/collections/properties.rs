use axum::{
    extract::{Path, State},
    response::Json,
};
use serde::{Deserialize, Serialize};
use crate::error::DbError;
use super::super::system::AppState;

// ==================== Structs ====================

#[derive(Debug, Deserialize, serde::Serialize, Clone)]
pub struct UpdateCollectionPropertiesRequest {
    /// Collection type: "document", "edge", or "blob"
    #[serde(rename = "type")]
    pub type_: Option<String>,
    /// Number of shards (updating this triggers rebalance)
    #[serde(rename = "numShards", alias = "num_shards")]
    pub num_shards: Option<u16>,
    /// Replication factor (optional, default: 1 = no replicas)
    #[serde(rename = "replicationFactor", alias = "replication_factor")]
    pub replication_factor: Option<u16>,
    /// Whether to propagate this update to other nodes (default: true)
    #[serde(default)]
    pub propagate: Option<bool>,
    /// JSON Schema for validation (optional)
    #[serde(rename = "schema")]
    pub schema: Option<serde_json::Value>,
    /// Validation mode: "off", "strict", or "lenient"
    #[serde(rename = "validationMode")]
    pub validation_mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CollectionPropertiesResponse {
    pub name: String,
    pub status: String,
    #[serde(rename = "shardConfig")]
    pub shard_config: crate::sharding::coordinator::CollectionShardConfig,
}

// ==================== Handlers ====================

pub async fn update_collection_properties(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(payload): Json<UpdateCollectionPropertiesRequest>,
) -> Result<Json<CollectionPropertiesResponse>, DbError> {
    tracing::info!(
        "update_collection_properties called: db={}, coll={}, payload={:?}",
        db_name,
        coll_name,
        payload
    );

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Update collection type if specified
    if let Some(new_type) = &payload.type_ {
        collection.set_type(new_type)?;
        tracing::info!(
            "Updated collection type for {}/{} to {}",
            db_name,
            coll_name,
            new_type
        );
    }

    // Get existing config or create new one if not sharded yet
    let mut config = collection
        .get_shard_config()
        .unwrap_or_else(|| crate::sharding::coordinator::CollectionShardConfig::default());

    tracing::info!("Current config before update: {:?}", config);

    let old_num_shards = config.num_shards;
    let mut shard_count_changed = false;

    // Get healthy node count for capping shard/replica values
    let healthy_node_count = if let Some(ref coordinator) = state.shard_coordinator {
        let count = coordinator.get_node_addresses().len();
        tracing::info!("Coordinator reports {} nodes", count);
        count
    } else {
        tracing::info!("No coordinator, using 1 node");
        1
    };

    // Update num_shards if specified
    if let Some(mut num_shards) = payload.num_shards {
        if num_shards < 1 {
            return Err(DbError::BadRequest(
                "Number of shards must be >= 1".to_string(),
            ));
        }

        // Cap num_shards to the number of healthy nodes
        tracing::info!(
            "Shard update check: requested={}, available_nodes={}",
            num_shards,
            healthy_node_count
        );

        if num_shards as usize > healthy_node_count {
            tracing::warn!(
                "Requested {} shards but only {} nodes available, capping to {}",
                num_shards,
                healthy_node_count,
                healthy_node_count
            );
            num_shards = healthy_node_count as u16;
        }

        if num_shards != config.num_shards {
            tracing::info!(
                "Updating num_shards for {}.{} from {} to {}",
                db_name,
                coll_name,
                config.num_shards,
                num_shards
            );
            config.num_shards = num_shards;
            shard_count_changed = true;
        } else {
            tracing::info!("num_shards unchanged ({})", num_shards);
        }
    } else {
        tracing::warn!("Update payload missing num_shards. Valid keys: numShards, num_shards");
    }

    // Update replication_factor if specified
    if let Some(mut rf) = payload.replication_factor {
        if rf < 1 {
            return Err(DbError::BadRequest(
                "Replication factor must be >= 1".to_string(),
            ));
        }

        // Cap replication_factor to the number of healthy nodes
        if rf as usize > healthy_node_count {
            tracing::warn!(
                "Requested replication factor {} but only {} nodes available, capping to {}",
                rf,
                healthy_node_count,
                healthy_node_count
            );
            rf = healthy_node_count as u16;
        }

        config.replication_factor = rf;
    }

    tracing::info!("Saving config: {:?}", config);

    // Save updated config
    collection.set_shard_config(&config)?;

    tracing::info!("Config saved successfully");

    // Trigger rebalance if shard count changed
    if shard_count_changed {
        if let Some(ref coordinator) = state.shard_coordinator {
            tracing::info!(
                "Shard count changed from {} to {} for {}/{}, triggering rebalance",
                old_num_shards,
                config.num_shards,
                db_name,
                coll_name
            );
            // Spawn rebalance as background task to avoid blocking the response
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                if let Err(e) = coordinator.rebalance().await {
                    tracing::error!("Failed to trigger rebalance: {}", e);
                }
            });
        }
    }

    // Broadcast metadata update to other cluster nodes to ensure consistency
    // This prevents "split brain" where only the coordinator node knows the new config
    let propagate = payload.propagate.unwrap_or(true);

    if propagate {
        if let Some(ref manager) = state.cluster_manager {
            let my_node_id = manager.local_node_id();
            let secret = state.cluster_secret();
            let client = reqwest::Client::new();

            // Clone payload and set propagate = false
            let mut forward_payload = payload.clone();
            forward_payload.propagate = Some(false);

            for member in manager.state().get_all_members() {
                if member.node.id == my_node_id {
                    continue;
                }

                let address = &member.node.api_address;
                let url = format!(
                    "http://{}/_api/database/{}/collection/{}/properties",
                    address, db_name, coll_name
                );

                tracing::info!(
                    "Propagating config update to node {} ({})",
                    member.node.id,
                    address
                );

                // Spawn background task for propagation to avoid latency
                let client = client.clone();
                let payload = forward_payload.clone();
                let secret = secret.clone();
                let url = url.clone();

                tokio::spawn(async move {
                    match client
                        .put(&url)
                        .header("X-Cluster-Secret", &secret)
                        .header("X-Shard-Direct", "true") // Bylass auth check
                        .json(&payload)
                        .send()
                        .await
                    {
                        Ok(res) => {
                            if !res.status().is_success() {
                                tracing::warn!(
                                    "Failed to propagate config to {}: {}",
                                    url,
                                    res.status()
                                );
                            } else {
                                tracing::debug!("Successfully propagated config to {}", url);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to send propagation request to {}: {}", url, e);
                        }
                    }
                });
            }
        }
    }

    Ok(Json(CollectionPropertiesResponse {
        name: coll_name,
        status: if shard_count_changed {
            "updated_rebalancing".to_string()
        } else {
            "updated".to_string()
        },
        shard_config: config,
    }))
}
