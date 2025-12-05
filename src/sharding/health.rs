//! Node health tracking for failover

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use reqwest::Client;
use tokio::time::interval;

/// Status of a single node
#[derive(Debug, Clone)]
pub struct NodeStatus {
    pub is_healthy: bool,
    pub last_check: Instant,
    pub last_success: Option<Instant>,
    pub consecutive_failures: u32,
}

impl Default for NodeStatus {
    fn default() -> Self {
        Self {
            is_healthy: true, // Assume healthy until proven otherwise
            last_check: Instant::now(),
            last_success: Some(Instant::now()),
            consecutive_failures: 0,
        }
    }
}

/// Tracks health of all cluster nodes
#[derive(Clone)]
pub struct NodeHealth {
    nodes: Arc<RwLock<HashMap<String, NodeStatus>>>,
    /// Number of consecutive failures before marking unhealthy
    failure_threshold: u32,
}

impl NodeHealth {
    /// Create a new health tracker
    pub fn new(node_addresses: Vec<String>, failure_threshold: u32) -> Self {
        let mut nodes = HashMap::new();
        for addr in node_addresses {
            nodes.insert(addr, NodeStatus::default());
        }
        
        Self {
            nodes: Arc::new(RwLock::new(nodes)),
            failure_threshold,
        }
    }

    /// Check if a node is currently healthy
    pub fn is_healthy(&self, node_addr: &str) -> bool {
        self.nodes
            .read()
            .unwrap()
            .get(node_addr)
            .map(|s| s.is_healthy)
            .unwrap_or(false)
    }

    /// Get all healthy nodes
    pub fn healthy_nodes(&self) -> Vec<String> {
        self.nodes
            .read()
            .unwrap()
            .iter()
            .filter(|(_, status)| status.is_healthy)
            .map(|(addr, _)| addr.clone())
            .collect()
    }

    /// Mark a node as having succeeded
    pub fn mark_success(&self, node_addr: &str) {
        if let Some(status) = self.nodes.write().unwrap().get_mut(node_addr) {
            status.is_healthy = true;
            status.last_check = Instant::now();
            status.last_success = Some(Instant::now());
            status.consecutive_failures = 0;
        }
    }

    /// Mark a node as having failed
    pub fn mark_failure(&self, node_addr: &str) {
        if let Some(status) = self.nodes.write().unwrap().get_mut(node_addr) {
            status.consecutive_failures += 1;
            status.last_check = Instant::now();
            
            if status.consecutive_failures >= self.failure_threshold {
                status.is_healthy = false;
            }
        }
    }

    /// Get status summary for all nodes
    pub fn status_summary(&self) -> HashMap<String, bool> {
        self.nodes
            .read()
            .unwrap()
            .iter()
            .map(|(addr, status)| (addr.clone(), status.is_healthy))
            .collect()
    }

    /// Start background health check task
    pub fn start_health_checker(
        self,
        check_interval: Duration,
    ) -> tokio::task::JoinHandle<()> {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("Failed to create HTTP client for health checks");

        tokio::spawn(async move {
            let mut ticker = interval(check_interval);
            
            loop {
                ticker.tick().await;
                
                // Get list of nodes to check
                let nodes: Vec<String> = self.nodes
                    .read()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect();

                for node_addr in nodes {
                    let url = format!("http://{}/_api/health", node_addr);
                    
                    match http_client.get(&url).send().await {
                        Ok(response) if response.status().is_success() => {
                            self.mark_success(&node_addr);
                        }
                        _ => {
                            self.mark_failure(&node_addr);
                        }
                    }
                }
            }
        })
    }
}

impl std::fmt::Debug for NodeHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeHealth")
            .field("nodes", &self.status_summary())
            .field("failure_threshold", &self.failure_threshold)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state_is_healthy() {
        let health = NodeHealth::new(vec!["node1:8080".to_string()], 3);
        assert!(health.is_healthy("node1:8080"));
    }

    #[test]
    fn test_mark_failure_threshold() {
        let health = NodeHealth::new(vec!["node1:8080".to_string()], 3);
        
        // First two failures: still healthy
        health.mark_failure("node1:8080");
        health.mark_failure("node1:8080");
        assert!(health.is_healthy("node1:8080"));
        
        // Third failure: now unhealthy
        health.mark_failure("node1:8080");
        assert!(!health.is_healthy("node1:8080"));
    }

    #[test]
    fn test_mark_success_resets_failures() {
        let health = NodeHealth::new(vec!["node1:8080".to_string()], 3);
        
        health.mark_failure("node1:8080");
        health.mark_failure("node1:8080");
        health.mark_success("node1:8080");
        
        // Failures reset, still healthy
        assert!(health.is_healthy("node1:8080"));
        
        // Need 3 more failures
        health.mark_failure("node1:8080");
        health.mark_failure("node1:8080");
        assert!(health.is_healthy("node1:8080"));
    }

    #[test]
    fn test_recovery_after_unhealthy() {
        let health = NodeHealth::new(vec!["node1:8080".to_string()], 3);
        
        // Make unhealthy
        health.mark_failure("node1:8080");
        health.mark_failure("node1:8080");
        health.mark_failure("node1:8080");
        assert!(!health.is_healthy("node1:8080"));
        
        // One success restores health
        health.mark_success("node1:8080");
        assert!(health.is_healthy("node1:8080"));
    }
}
