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
pub mod log;
pub mod protocol;
pub mod state;
pub mod transport;
pub mod worker;

// Re-export key types
pub use log::{LogEntry, SyncLog};
pub use protocol::{NodeStats, Operation, ShardConfig, SyncEntry, SyncMessage};
pub use state::SyncState;
pub use transport::{ConnectionPool, SyncServer, TransportError};
pub use worker::{create_command_channel, SyncCommand, SyncConfig, SyncWorker};
