use std::time::Duration;
use tokio::time::interval;

use super::state::{ClusterState, NodeStatus};

/// Configuration for health monitoring
#[derive(Debug, Clone)]
pub struct HealthConfig {
    pub heartbeat_interval: Duration,
    pub failure_threshold: Duration,
    pub suspicion_threshold: Duration,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            // TODO: These are set high because heartbeat SENDING is not yet implemented.
            // Nodes are updated via sync operations, but dedicated heartbeats need to be added.
            heartbeat_interval: Duration::from_secs(5),
            suspicion_threshold: Duration::from_secs(10),
            failure_threshold: Duration::from_secs(15),
        }
    }
}

/// Monitor node health and update status
pub struct HealthMonitor {
    config: HealthConfig,
    state: ClusterState,
}

impl HealthMonitor {
    pub fn new(config: HealthConfig, state: ClusterState) -> Self {
        Self { config, state }
    }

    /// Start the background health check loop
    pub async fn start(self) {
        let mut tick = interval(self.config.heartbeat_interval);

        loop {
            tick.tick().await;
            self.check_nodes();
        }
    }

    fn check_nodes(&self) {
        let members = self.state.get_all_members();
        let now = chrono::Utc::now().timestamp_millis() as u64;

        for member in members {
            if member.node.id == self.state.local_node_id {
                continue;
            }

            let elapsed_ms = now.saturating_sub(member.last_heartbeat);
            let elapsed = Duration::from_millis(elapsed_ms);

            if elapsed > self.config.failure_threshold {
                if member.status != NodeStatus::Dead {
                    // TODO: Log warning
                    self.state.mark_status(&member.node.id, NodeStatus::Dead);
                }
            } else if elapsed > self.config.suspicion_threshold
                && member.status == NodeStatus::Active
            {
                // TODO: Log warning
                self.state
                    .mark_status(&member.node.id, NodeStatus::Suspected);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::node::Node;

    #[test]
    fn test_health_config_default() {
        let config = HealthConfig::default();

        assert_eq!(config.heartbeat_interval, Duration::from_secs(5));
        assert_eq!(config.suspicion_threshold, Duration::from_secs(10));
        assert_eq!(config.failure_threshold, Duration::from_secs(15));
    }

    #[test]
    fn test_health_config_custom() {
        let config = HealthConfig {
            heartbeat_interval: Duration::from_secs(2),
            suspicion_threshold: Duration::from_secs(5),
            failure_threshold: Duration::from_secs(10),
        };

        assert_eq!(config.heartbeat_interval, Duration::from_secs(2));
    }

    #[test]
    fn test_health_config_clone() {
        let config = HealthConfig::default();
        let cloned = config.clone();

        assert_eq!(config.heartbeat_interval, cloned.heartbeat_interval);
        assert_eq!(config.failure_threshold, cloned.failure_threshold);
    }

    #[test]
    fn test_health_monitor_new() {
        let config = HealthConfig::default();
        let state = ClusterState::new("local".to_string());

        let _monitor = HealthMonitor::new(config, state);
        // Should not panic
    }

    #[test]
    fn test_check_nodes_skips_local() {
        let config = HealthConfig::default();
        let state = ClusterState::new("local".to_string());

        // Add local node
        let local_node = Node::new(
            "local".to_string(),
            "127.0.0.1:8000".to_string(),
            "127.0.0.1:9000".to_string(),
        );
        state.add_member(local_node, NodeStatus::Active);

        let monitor = HealthMonitor::new(config, state.clone());

        // Should not change local node status
        monitor.check_nodes();

        let member = state.get_member("local").unwrap();
        assert_eq!(member.status, NodeStatus::Active);
    }

    #[test]
    fn test_check_nodes_marks_suspected() {
        let config = HealthConfig {
            heartbeat_interval: Duration::from_secs(1),
            suspicion_threshold: Duration::from_millis(1), // Very short for test
            failure_threshold: Duration::from_secs(1000),  // Very long to avoid Dead
        };
        let state = ClusterState::new("local".to_string());

        // Add a remote node with old heartbeat
        let remote_node = Node::new(
            "remote".to_string(),
            "127.0.0.1:8001".to_string(),
            "127.0.0.1:9001".to_string(),
        );
        state.add_member(remote_node, NodeStatus::Active);

        // Sleep to exceed suspicion threshold
        std::thread::sleep(Duration::from_millis(10));

        let monitor = HealthMonitor::new(config, state.clone());
        monitor.check_nodes();

        let member = state.get_member("remote").unwrap();
        assert_eq!(member.status, NodeStatus::Suspected);
    }

    #[test]
    fn test_check_nodes_marks_dead() {
        let config = HealthConfig {
            heartbeat_interval: Duration::from_secs(1),
            suspicion_threshold: Duration::from_millis(1),
            failure_threshold: Duration::from_millis(1), // Very short for test
        };
        let state = ClusterState::new("local".to_string());

        let remote_node = Node::new(
            "remote".to_string(),
            "127.0.0.1:8001".to_string(),
            "127.0.0.1:9001".to_string(),
        );
        state.add_member(remote_node, NodeStatus::Active);

        // Sleep to exceed failure threshold
        std::thread::sleep(Duration::from_millis(10));

        let monitor = HealthMonitor::new(config, state.clone());
        monitor.check_nodes();

        let member = state.get_member("remote").unwrap();
        assert_eq!(member.status, NodeStatus::Dead);
    }
}
