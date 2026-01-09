# Sharding Module

## Purpose
Horizontal partitioning of data across multiple nodes. Distributes documents based on consistent hashing of keys.

## Key Files

| File | Description |
|------|-------------|
| `coordinator.rs` | Central shard management, orchestrates operations |
| `migration.rs` | Shard data migration between nodes |
| `distribution.rs` | Shard placement and rebalancing logic |
| `router.rs` | Routes requests to correct shard/node |
| `mod.rs` | Module exports and types |

## Architecture

### Shard Distribution
```
Document Key → hash(key) % num_shards → Shard ID → Node(s)
```

### Key Components

#### ShardCoordinator (coordinator.rs)
Central coordinator for shard operations:
```rust
pub struct ShardCoordinator {
    storage: Arc<StorageEngine>,
    shard_table: Arc<RwLock<ShardTable>>,
    node_id: String,
    cluster_manager: Option<Arc<ClusterManager>>,
}
```

#### ShardTable
Maps shards to nodes:
```rust
pub struct ShardTable {
    num_shards: u32,
    replication_factor: u32,
    shards: HashMap<ShardId, ShardInfo>,
}
```

#### ShardInfo
```rust
pub struct ShardInfo {
    id: ShardId,
    leader: NodeId,
    replicas: Vec<NodeId>,
    status: ShardStatus,
}
```

### Shard Statuses
- `Active` - Normal operation
- `Migrating` - Data being moved
- `Syncing` - Catching up with leader
- `Offline` - Temporarily unavailable

## Migration Process (migration.rs)

1. **Prepare**: Lock shard for writes
2. **Copy**: Stream documents to target node
3. **Sync**: Apply queued writes
4. **Switch**: Update shard table
5. **Cleanup**: Remove from old node

## Common Tasks

### Adding Sharding to a Collection
```rust
coordinator.enable_sharding("mydb", "mycollection", ShardingConfig {
    num_shards: 8,
    replication_factor: 2,
    shard_key: vec!["_key"],
})?;
```

### Manual Rebalance
Trigger shard redistribution after adding/removing nodes.

### Debugging Shard Issues
1. Check `_system/_sharding` collection for shard table
2. Verify node health via cluster status
3. Check migration status if rebalancing

## Dependencies
- **Uses**: `cluster::ClusterManager` for node coordination
- **Used by**: `server::handlers` for routing, `storage` for distributed ops

## Gotchas
- `coordinator.rs` is large (3,447 lines) - search for specific operations
- Shard key fields must exist in all documents
- Rebalancing can be I/O intensive - schedule during low traffic
- Cross-shard transactions not supported (use same shard key)
- Default: 16 shards, replication factor 1
