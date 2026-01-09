# Sync Module

## Purpose
Replication and synchronization between cluster nodes. Ensures eventual consistency through operation log shipping and conflict resolution.

## Key Files

| File | Description |
|------|-------------|
| `worker.rs` | Replication worker - pulls changes from leader |
| `log.rs` | SyncLog - operation log management |
| `protocol.rs` | Replication protocol messages |
| `transport.rs` | Network transport for sync |
| `state.rs` | Sync state tracking |
| `blob_replication.rs` | Binary blob synchronization |
| `mod.rs` | Module exports |

## Architecture

### Replication Flow
```
Leader                      Follower
  │                            │
  │ Write Operation            │
  ├──────────────────────────► │
  │                            │ Append to Log
  │                            │
  │         ACK                │
  │ ◄────────────────────────┤
```

### SyncLog (log.rs)
Append-only operation log:
```rust
pub struct SyncLog {
    storage: Arc<StorageEngine>,
    sequence: AtomicU64,
    entries: RwLock<VecDeque<LogEntry>>,
}

pub struct LogEntry {
    sequence: u64,
    timestamp: HLCTimestamp,
    operation: Operation,
    database: String,
    collection: String,
}
```

### Operations Logged
```rust
pub enum Operation {
    Insert { key: String, document: Value },
    Update { key: String, document: Value },
    Delete { key: String },
    CreateCollection { name: String, config: Option<Value> },
    DropCollection { name: String },
}
```

### SyncWorker (worker.rs)
Background task that:
1. Connects to leader node
2. Requests entries after last known sequence
3. Applies operations locally
4. Updates sync position

## Consistency Model

**Eventual Consistency**:
- Writes go to leader
- Leader replicates to followers
- Followers eventually catch up
- Read-your-writes on leader

**Conflict Resolution**:
- Last-writer-wins based on HLC timestamp
- Document `_rev` for optimistic concurrency

## Blob Replication (blob_replication.rs)

Large binary files synced separately:
1. Detect new blobs on leader
2. Chunk transfer (1MB chunks)
3. Checksum verification
4. Atomic commit

## Common Tasks

### Checking Replication Lag
```
GET /_api/cluster/sync/status
```

Returns sequence numbers and lag per follower.

### Forcing Sync
Followers automatically sync, but can trigger:
```
POST /_api/cluster/sync/trigger
```

### Debugging Replication
1. Check `_system/_sync_log` for operation log
2. Compare sequence numbers across nodes
3. Check network connectivity between nodes

## Dependencies
- **Uses**: `cluster::HLC` for timestamps, `storage` for persistence
- **Used by**: `cluster::ClusterManager`, background workers

## Gotchas
- Log entries pruned after all followers acknowledge
- Network partitions queue writes until reconnection
- Blobs sync asynchronously (may lag behind documents)
- Sync position stored in `_system/_sync_state`
- Default batch size: 1000 entries per sync round
