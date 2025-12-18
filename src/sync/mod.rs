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

pub mod protocol;
pub mod transport;
pub mod worker;
pub mod state;
pub mod log;
pub mod blob_replication;

// Re-export key types
pub use protocol::{Operation, SyncEntry, SyncMessage, ShardConfig, NodeStats};
pub use state::SyncState;
pub use transport::{ConnectionPool, SyncServer, TransportError};
pub use worker::{SyncWorker, SyncConfig, SyncCommand, create_command_channel};
pub use log::{SyncLog, LogEntry};
