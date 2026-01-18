//! Sharding compatibility layer
//!
//! This module provides backwards-compatible types for sharding
//! that map to the new sync module's ShardConfig.

pub mod batch;
pub mod cleanup;
pub mod coordinator;
pub mod distribution;
pub mod healing;
pub mod migration;
pub mod rebalance;
pub mod repro_issue;
pub mod router;

pub use coordinator::{CollectionShardConfig, ShardCoordinator};
