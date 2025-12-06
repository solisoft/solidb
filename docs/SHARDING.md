# Sharding and Replication Specification

## 1. Overview
SoliDB implements a shared-nothing sharding architecture with master-master replication capabilities. Data is distributed across multiple nodes to provide horizontal scaling and high availability.

## 2. Architecture

### 2.1 Components
- **ShardRouter**: Deterministic routing logic using consistent hashing concepts.
- **ShardCoordinator**: Manages distributed operations, failover, and recovery.
- **NodeHealth**: Background monitor for node availability.
- **ReplicationQueue**: Durable (in-memory) queue for storing writes destined for offline nodes.

### 2.2 Data Distribution
- **Sharding Key**: Defaults to `_key`, but can be any top-level string/number field.
- **Routing Algorithm**: `ShardID = hash(Key) % NumShards`.
- **Node Mapping**: `Node = (ShardID + ReplicaIndex) % NumNodes`.
- **Placement**: Round-robin shard distribution ensures balanced load.

## 3. Replication Implementation

### 3.1 Write Path (`insert` / `update` / `delete`)
1.  **Routing**: The coordinator determines the `ShardID` and the list of `ReplicaNodes`.
2.  **Replication**: The write is sent to **ALL** replica nodes in parallel.
3.  **Consistency**:
    - Returns Success if **at least one** replica acknowledges the write (Eventual Consistency priority).
    - Returns Error only if ALL replicas fail.
4.  **Failure Handling**:
    - If a replica is down, the operation is queued in the `ReplicationQueue`.

### 3.2 Read Path (`get`)
1.  **Routing**: Determines `ShardID` and `ReplicaNodes`.
2.  **Failover**:
    - Tries the **Primary** node first.
    - If Primary is down (determined by `NodeHealth`), transparently tries the **Secondary**, then **Tertiary**, etc.
    - Returns the first successful response.

## 4. Availability & Recovery

### 4.1 Temporary Failure (Recovery)
- When a node is temporarily down (e.g., restart, network blip):
    - Writes are preserved in `ReplicationQueue` on healthy nodes.
- **Auto-Recovery**:
    - A background task monitors node health.
    - When the node returns to `Healthy` state, queued operations are automatically replayed to sync data.

### 4.2 Permanent Failure (Auto-Rebalancing)
- When a node is down for an extended period (exceeding `failure_threshold`, e.g., 30s):
    1.  **Detection**: `NodeHealth` marks the node as dead.
    2.  **Topology Change**: The dead node is removed from the active cluster ring.
    3.  **Data Migration**:
        - `rebalance()` is triggered on all remaining nodes.
        - Nodes scan their data and calculate new replica targets based on the updated topology.
        - Data is forwarded to new owners to restore the Replication Factor.

## 5. API Extensions

### 5.1 Document Metadata
The `get_document` API includes a `_replicas` system field when sharding is enabled:
```json
{
  "_id": "users/123",
  "_key": "123",
  "_replicas": ["127.0.0.1:8001", "127.0.0.1:8002"],
  "name": "Alice"
}
```
This allows clients to verify physical data location.

## 6. Configuration

Collections are configured for sharding at creation time:
```rust
struct CollectionShardConfig {
    num_shards: u16,           // e.g. 16
    replication_factor: u16,   // e.g. 2 or 3
    shard_key: String          // e.g. "_key"
}
```
