# Cluster Module

## Purpose
Multi-node coordination for distributed deployments. Handles node discovery, health monitoring, and distributed timestamps using Hybrid Logical Clocks.

## Key Files

| File | Description |
|------|-------------|
| `manager.rs` | ClusterManager - main coordinator |
| `hlc.rs` | Hybrid Logical Clock implementation |
| `health.rs` | Node health monitoring |
| `node.rs` | Node representation and state |
| `state.rs` | Cluster state machine |
| `transport.rs` | Inter-node communication |
| `config.rs` | Cluster configuration |
| `stats.rs` | Cluster statistics |
| `websocket_client.rs` | WS client for node communication |

## Architecture

### Cluster Topology
```
       ┌──────────┐
       │  Leader  │
       └────┬─────┘
            │
    ┌───────┼───────┐
    ▼       ▼       ▼
┌──────┐ ┌──────┐ ┌──────┐
│Node 1│ │Node 2│ │Node 3│
└──────┘ └──────┘ └──────┘
```

### ClusterManager (manager.rs)
```rust
pub struct ClusterManager {
    node_id: String,
    config: ClusterConfig,
    state: Arc<RwLock<ClusterState>>,
    hlc: Arc<HybridLogicalClock>,
    health_checker: HealthChecker,
}
```

### Hybrid Logical Clock (hlc.rs)
Provides globally ordered timestamps:
```rust
pub struct HybridLogicalClock {
    physical: AtomicU64,  // Wall clock
    logical: AtomicU32,   // Logical counter
}

impl HybridLogicalClock {
    pub fn now(&self) -> HLCTimestamp;
    pub fn update(&self, received: HLCTimestamp);
}
```

HLC guarantees:
- Causally consistent ordering
- Monotonically increasing
- Close to physical time

### Node States
```rust
pub enum NodeState {
    Joining,    // Connecting to cluster
    Active,     // Normal operation
    Leaving,    // Graceful shutdown
    Failed,     // Detected failure
    Unknown,    // No recent heartbeat
}
```

## Health Monitoring (health.rs)

Heartbeat-based failure detection:
- Heartbeat interval: 1 second
- Failure threshold: 5 missed heartbeats
- Suspicion before confirmed failure

## Common Tasks

### Starting a Cluster Node
```bash
./solidb --cluster-peers "node1:6745,node2:6745" --node-id "node3"
```

### Checking Cluster Status
```
GET /_api/cluster/status
```

### Adding a New Node
1. Start node with existing peers
2. Node auto-joins and syncs
3. Shards rebalance if configured

## Dependencies
- **Uses**: `sync` for replication, `sharding` for data distribution
- **Used by**: `server::handlers` for cluster endpoints, `main.rs`

## Gotchas
- HLC timestamps are 12 bytes (8 physical + 4 logical)
- Node IDs should be stable across restarts
- Network partitions may cause split-brain - uses leader election
- Cluster requires minimum 3 nodes for fault tolerance
- First node becomes initial leader automatically
