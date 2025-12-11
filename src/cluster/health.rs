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
            heartbeat_interval: Duration::from_secs(1),
            suspicion_threshold: Duration::from_secs(3),
            failure_threshold: Duration::from_secs(10),
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
            } else if elapsed > self.config.suspicion_threshold {
                if member.status == NodeStatus::Active {
                    // TODO: Log warning
                    self.state.mark_status(&member.node.id, NodeStatus::Suspected);
                }
            }
        }
    }
}
