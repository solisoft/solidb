//! Synchronization module for P2P master-master replication
//!
//! This module provides:
//! - Binary protocol using bincode over TCP
//! - Async sync worker for background replication
//! - Batch and incremental sync modes
//! - Shard-aware replication with rebalancing
//! - LZ4 compression for large batches
//!
//! Architecture:
//! - All nodes are equal masters in a P2P cluster
//! - Non-sharded collections replicate to ALL nodes
//! - Sharded collections route by shard owner with replication_factor copies

pub mod blob_replication;
pub mod conflict;
pub mod crdt;
pub mod delta;
pub mod log;
pub mod protocol;
pub mod session;
pub mod state;
pub mod tombstone;
pub mod transport;
pub mod version_vector;
pub mod worker;

// Re-export key types
pub use conflict::{ConflictResolutionStrategy, ConflictResolver};
pub use crdt::{CRDTDocument, GCounter, LWWRegister, ORSet, PNCounter, CRDT};
pub use delta::{apply_patch, compute_diff, JsonPatch, PatchError, PatchOperation};
pub use log::{LogEntry, SyncLog};
pub use protocol::{NodeStats, Operation, ShardConfig, SyncEntry, SyncMessage};
pub use session::{SyncSession, SyncSessionManager};
pub use state::SyncState;
pub use tombstone::{Tombstone, TombstoneConfig, TombstoneManager, TombstoneStats};
pub use transport::{ConnectionPool, SyncServer, TransportError};
pub use version_vector::{CausalDot, ConflictInfo, VectorComparison, VersionVector};
pub use worker::{create_command_channel, SyncCommand, SyncConfig, SyncWorker};
