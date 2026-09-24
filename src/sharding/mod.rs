//! Sharding compatibility layer
//!
//! This module provides backwards-compatible types for sharding
//! that map to the new sync module's ShardConfig.

pub mod assignment;
pub mod batch;
pub mod blob_rebalance;
pub mod cleanup;
pub mod coordinator;
pub mod distribution;
pub mod export_stream;
pub mod healing;
pub mod migration;
pub mod rebalance;
pub mod repro_issue;
pub mod router;
pub mod scan;

pub use blob_rebalance::{BlobRebalanceWorker, RebalanceConfig};
pub use coordinator::{CollectionShardConfig, ShardAssignment, ShardCoordinator, ShardTable};
