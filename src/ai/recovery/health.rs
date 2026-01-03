//! Agent health monitoring
//!
//! Tracks agent health metrics and circuit breaker state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Collection name for agent health data
pub const AGENT_HEALTH_COLLECTION: &str = "_ai_agent_health";

/// Circuit breaker state
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CircuitState {
    /// Circuit is closed - allowing requests
    Closed,
    /// Circuit is open - blocking requests
    Open,
    /// Circuit is half-open - testing with limited requests
    HalfOpen,
}

impl Default for CircuitState {
    fn default() -> Self {
        CircuitState::Closed
    }
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CircuitState::Closed => write!(f, "closed"),
            CircuitState::Open => write!(f, "open"),
            CircuitState::HalfOpen => write!(f, "half_open"),
        }
    }
}

/// Health metrics for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHealthMetrics {
    /// Agent identifier
    #[serde(rename = "_key")]
    pub agent_id: String,

    /// Current circuit breaker state
    #[serde(default)]
    pub circuit_state: CircuitState,

    /// Number of consecutive failures
    #[serde(default)]
    pub consecutive_failures: u32,

    /// Total failures in current window
    #[serde(default)]
    pub window_failures: u32,

    /// Total requests in current window
    #[serde(default)]
    pub window_requests: u32,

    /// Last time the agent sent a heartbeat
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_heartbeat: Option<DateTime<Utc>>,

    /// Last time the circuit state changed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub circuit_state_changed_at: Option<DateTime<Utc>>,

    /// When to try half-open state (if circuit is open)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub circuit_retry_at: Option<DateTime<Utc>>,

    /// Number of tasks currently assigned
    #[serde(default)]
    pub active_tasks: u32,

    /// Tasks recovered from this agent
    #[serde(default)]
    pub tasks_recovered: u32,

    /// When this record was last updated
    pub updated_at: DateTime<Utc>,
}

impl AgentHealthMetrics {
    /// Create new health metrics for an agent
    pub fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            circuit_state: CircuitState::Closed,
            consecutive_failures: 0,
            window_failures: 0,
            window_requests: 0,
            last_heartbeat: Some(Utc::now()),
            circuit_state_changed_at: Some(Utc::now()),
            circuit_retry_at: None,
            active_tasks: 0,
            tasks_recovered: 0,
            updated_at: Utc::now(),
        }
    }

    /// Record a successful request
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.window_requests += 1;
        self.updated_at = Utc::now();

        // If half-open and success, transition to closed
        if self.circuit_state == CircuitState::HalfOpen {
            self.transition_to_closed();
        }
    }

    /// Record a failed request
    pub fn record_failure(&mut self, failure_threshold: u32, failure_rate_threshold: f64) {
        self.consecutive_failures += 1;
        self.window_failures += 1;
        self.window_requests += 1;
        self.updated_at = Utc::now();

        // Check if circuit should open
        let should_open = match self.circuit_state {
            CircuitState::Closed => {
                // Open if consecutive failures exceed threshold
                self.consecutive_failures >= failure_threshold
                    || (self.window_requests >= 10
                        && self.failure_rate() >= failure_rate_threshold)
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open reopens circuit
                true
            }
            CircuitState::Open => false,
        };

        if should_open {
            self.transition_to_open();
        }
    }

    /// Get current failure rate
    pub fn failure_rate(&self) -> f64 {
        if self.window_requests == 0 {
            0.0
        } else {
            self.window_failures as f64 / self.window_requests as f64
        }
    }

    /// Transition to closed state
    pub fn transition_to_closed(&mut self) {
        self.circuit_state = CircuitState::Closed;
        self.circuit_state_changed_at = Some(Utc::now());
        self.circuit_retry_at = None;
        self.consecutive_failures = 0;
        self.window_failures = 0;
        self.window_requests = 0;
        self.updated_at = Utc::now();
    }

    /// Transition to open state
    pub fn transition_to_open(&mut self) {
        self.circuit_state = CircuitState::Open;
        self.circuit_state_changed_at = Some(Utc::now());
        self.updated_at = Utc::now();
    }

    /// Set retry time for half-open transition
    pub fn set_retry_at(&mut self, retry_at: DateTime<Utc>) {
        self.circuit_retry_at = Some(retry_at);
        self.updated_at = Utc::now();
    }

    /// Transition to half-open state
    pub fn transition_to_half_open(&mut self) {
        self.circuit_state = CircuitState::HalfOpen;
        self.circuit_state_changed_at = Some(Utc::now());
        self.circuit_retry_at = None;
        self.updated_at = Utc::now();
    }

    /// Check if circuit should transition to half-open
    pub fn should_try_half_open(&self) -> bool {
        if self.circuit_state != CircuitState::Open {
            return false;
        }
        match self.circuit_retry_at {
            Some(retry_at) => Utc::now() >= retry_at,
            None => false,
        }
    }

    /// Check if agent is healthy (heartbeat within timeout)
    pub fn is_healthy(&self, heartbeat_timeout_secs: u64) -> bool {
        match self.last_heartbeat {
            Some(last) => {
                let elapsed = (Utc::now() - last).num_seconds() as u64;
                elapsed < heartbeat_timeout_secs
            }
            None => false,
        }
    }

    /// Update heartbeat timestamp
    pub fn update_heartbeat(&mut self) {
        self.last_heartbeat = Some(Utc::now());
        self.updated_at = Utc::now();
    }

    /// Check if circuit allows requests
    pub fn allows_requests(&self) -> bool {
        matches!(self.circuit_state, CircuitState::Closed | CircuitState::HalfOpen)
    }
}

/// Summary of recovery system health
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoverySystemStatus {
    /// Total agents monitored
    pub total_agents: usize,
    /// Agents with open circuit breakers
    pub agents_circuit_open: usize,
    /// Agents with missed heartbeats
    pub agents_unhealthy: usize,
    /// Tasks currently stalled
    pub stalled_tasks: usize,
    /// Contributions stuck in pipeline
    pub stuck_contributions: usize,
    /// Last recovery scan time
    pub last_scan: Option<DateTime<Utc>>,
    /// Recovery events in last hour
    pub recent_events: usize,
}

impl Default for RecoverySystemStatus {
    fn default() -> Self {
        Self {
            total_agents: 0,
            agents_circuit_open: 0,
            agents_unhealthy: 0,
            stalled_tasks: 0,
            stuck_contributions: 0,
            last_scan: None,
            recent_events: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_health_new() {
        let health = AgentHealthMetrics::new("agent-001".to_string());
        assert_eq!(health.agent_id, "agent-001");
        assert_eq!(health.circuit_state, CircuitState::Closed);
        assert_eq!(health.consecutive_failures, 0);
        assert!(health.last_heartbeat.is_some());
    }

    #[test]
    fn test_record_success() {
        let mut health = AgentHealthMetrics::new("agent-001".to_string());
        health.consecutive_failures = 3;
        health.record_success();
        assert_eq!(health.consecutive_failures, 0);
        assert_eq!(health.window_requests, 1);
    }

    #[test]
    fn test_record_failure_opens_circuit() {
        let mut health = AgentHealthMetrics::new("agent-001".to_string());

        // Record failures until threshold
        for _ in 0..5 {
            health.record_failure(5, 0.5);
        }

        assert_eq!(health.circuit_state, CircuitState::Open);
        assert_eq!(health.consecutive_failures, 5);
    }

    #[test]
    fn test_failure_rate() {
        let mut health = AgentHealthMetrics::new("agent-001".to_string());
        health.window_requests = 10;
        health.window_failures = 3;
        assert!((health.failure_rate() - 0.3).abs() < 0.001);
    }

    #[test]
    fn test_half_open_success_closes() {
        let mut health = AgentHealthMetrics::new("agent-001".to_string());
        health.transition_to_half_open();
        assert_eq!(health.circuit_state, CircuitState::HalfOpen);

        health.record_success();
        assert_eq!(health.circuit_state, CircuitState::Closed);
    }

    #[test]
    fn test_half_open_failure_reopens() {
        let mut health = AgentHealthMetrics::new("agent-001".to_string());
        health.transition_to_half_open();

        health.record_failure(5, 0.5);
        assert_eq!(health.circuit_state, CircuitState::Open);
    }

    #[test]
    fn test_allows_requests() {
        let mut health = AgentHealthMetrics::new("agent-001".to_string());
        assert!(health.allows_requests()); // Closed

        health.transition_to_half_open();
        assert!(health.allows_requests()); // Half-open

        health.transition_to_open();
        assert!(!health.allows_requests()); // Open
    }

    #[test]
    fn test_circuit_state_display() {
        assert_eq!(CircuitState::Closed.to_string(), "closed");
        assert_eq!(CircuitState::Open.to_string(), "open");
        assert_eq!(CircuitState::HalfOpen.to_string(), "half_open");
    }
}
