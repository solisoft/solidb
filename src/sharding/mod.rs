//! Sharding compatibility layer
//!
//! This module provides backwards-compatible types for sharding
//! that map to the new sync module's ShardConfig.

pub mod coordinator;
pub mod router;
pub mod distribution;
pub mod migration;
pub mod repro_issue;

pub use coordinator::{ShardCoordinator, CollectionShardConfig};
