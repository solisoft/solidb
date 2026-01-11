//! Autonomous Recovery Module
//!
//! This module provides self-healing capabilities for the AI-augmented database:
//! - Stalled task detection and recovery
//! - Agent health monitoring
//! - Circuit breaker protection for unreliable agents
//! - Stuck pipeline detection and restart

pub mod config;
pub mod event;
pub mod health;
pub mod worker;

pub use config::RecoveryConfig;
pub use event::{
    ListRecoveryEventsResponse, RecoveryActionType, RecoveryCycleStats, RecoveryEvent,
    RecoveryEventQuery, RecoverySeverity, RECOVERY_EVENTS_COLLECTION,
};
pub use health::{AgentHealthMetrics, CircuitState, RecoverySystemStatus, AGENT_HEALTH_COLLECTION};
pub use worker::RecoveryWorker;
