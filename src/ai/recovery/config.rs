//! Recovery configuration
//!
//! Configuration settings for the autonomous recovery system.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::ai::task::AITaskType;

/// Configuration for the recovery system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryConfig {
    /// How often to scan for issues (seconds)
    #[serde(default = "default_scan_interval")]
    pub scan_interval_secs: u64,

    /// Timeout thresholds per task type (seconds)
    #[serde(default = "default_task_timeouts")]
    pub task_timeouts: HashMap<String, u64>,

    /// How long before an agent is considered unhealthy (seconds)
    #[serde(default = "default_heartbeat_timeout")]
    pub agent_heartbeat_timeout_secs: u64,

    /// Number of consecutive failures before circuit opens
    #[serde(default = "default_circuit_failure_threshold")]
    pub circuit_failure_threshold: u32,

    /// Failure rate threshold (0.0 - 1.0) for circuit breaker
    #[serde(default = "default_circuit_failure_rate")]
    pub circuit_failure_rate_threshold: f64,

    /// How long to wait before trying half-open state (seconds)
    #[serde(default = "default_circuit_cooldown")]
    pub circuit_cooldown_secs: u64,

    /// How long before a contribution is considered stuck (seconds)
    #[serde(default = "default_contribution_stuck_timeout")]
    pub contribution_stuck_timeout_secs: u64,

    /// Maximum retry attempts for task recovery
    #[serde(default = "default_max_recovery_retries")]
    pub max_recovery_retries: u32,

    /// Enable automatic task reassignment
    #[serde(default = "default_true")]
    pub enable_task_reassignment: bool,

    /// Enable circuit breaker protection
    #[serde(default = "default_true")]
    pub enable_circuit_breaker: bool,
}

fn default_scan_interval() -> u64 {
    30
}

fn default_task_timeouts() -> HashMap<String, u64> {
    let mut timeouts = HashMap::new();
    timeouts.insert(AITaskType::AnalyzeContribution.to_string(), 300); // 5 min
    timeouts.insert(AITaskType::GenerateCode.to_string(), 600); // 10 min
    timeouts.insert(AITaskType::ValidateCode.to_string(), 300); // 5 min
    timeouts.insert(AITaskType::RunTests.to_string(), 900); // 15 min
    timeouts.insert(AITaskType::PrepareReview.to_string(), 180); // 3 min
    timeouts.insert(AITaskType::MergeChanges.to_string(), 120); // 2 min
    timeouts
}

fn default_heartbeat_timeout() -> u64 {
    60
}

fn default_circuit_failure_threshold() -> u32 {
    5
}

fn default_circuit_failure_rate() -> f64 {
    0.5
}

fn default_circuit_cooldown() -> u64 {
    300 // 5 min
}

fn default_contribution_stuck_timeout() -> u64 {
    1800 // 30 min
}

fn default_max_recovery_retries() -> u32 {
    3
}

fn default_true() -> bool {
    true
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            scan_interval_secs: default_scan_interval(),
            task_timeouts: default_task_timeouts(),
            agent_heartbeat_timeout_secs: default_heartbeat_timeout(),
            circuit_failure_threshold: default_circuit_failure_threshold(),
            circuit_failure_rate_threshold: default_circuit_failure_rate(),
            circuit_cooldown_secs: default_circuit_cooldown(),
            contribution_stuck_timeout_secs: default_contribution_stuck_timeout(),
            max_recovery_retries: default_max_recovery_retries(),
            enable_task_reassignment: true,
            enable_circuit_breaker: true,
        }
    }
}

impl RecoveryConfig {
    /// Get timeout for a specific task type
    pub fn get_task_timeout(&self, task_type: &AITaskType) -> u64 {
        self.task_timeouts
            .get(&task_type.to_string())
            .copied()
            .unwrap_or(300) // Default 5 min
    }

    /// Create a minimal config for testing
    #[cfg(test)]
    pub fn minimal() -> Self {
        Self {
            scan_interval_secs: 5,
            agent_heartbeat_timeout_secs: 10,
            circuit_cooldown_secs: 30,
            contribution_stuck_timeout_secs: 60,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RecoveryConfig::default();
        assert_eq!(config.scan_interval_secs, 30);
        assert_eq!(config.agent_heartbeat_timeout_secs, 60);
        assert_eq!(config.circuit_failure_threshold, 5);
        assert!(config.enable_circuit_breaker);
    }

    #[test]
    fn test_task_timeout() {
        let config = RecoveryConfig::default();
        assert_eq!(
            config.get_task_timeout(&AITaskType::AnalyzeContribution),
            300
        );
        assert_eq!(config.get_task_timeout(&AITaskType::GenerateCode), 600);
        assert_eq!(config.get_task_timeout(&AITaskType::RunTests), 900);
    }

    #[test]
    fn test_config_serialization() {
        let config = RecoveryConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: RecoveryConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.scan_interval_secs, config.scan_interval_secs);
    }
}
