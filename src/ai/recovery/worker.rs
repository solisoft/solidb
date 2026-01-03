//! Recovery Worker
//!
//! Background worker that monitors for stalled tasks, agent health issues,
//! and stuck pipelines, taking automatic recovery actions.

use chrono::{Duration, Utc};
use std::sync::Arc;
use tokio::time;

use crate::ai::contribution::{Contribution, ContributionStatus};
use crate::ai::task::{AITask, AITaskStatus};
use crate::error::DbError;
use crate::storage::StorageEngine;

use super::config::RecoveryConfig;
use super::event::{RecoveryCycleStats, RecoveryEvent, RECOVERY_EVENTS_COLLECTION};
use super::health::{AgentHealthMetrics, CircuitState, RecoverySystemStatus, AGENT_HEALTH_COLLECTION};

/// The Recovery Worker monitors and recovers from issues
pub struct RecoveryWorker {
    storage: Arc<StorageEngine>,
    config: RecoveryConfig,
    db_name: String,
}

impl RecoveryWorker {
    /// Create a new recovery worker
    pub fn new(storage: Arc<StorageEngine>, db_name: String, config: RecoveryConfig) -> Self {
        Self {
            storage,
            config,
            db_name,
        }
    }

    /// Start the recovery worker background loop
    pub async fn start(self: Arc<Self>) {
        let interval = time::Duration::from_secs(self.config.scan_interval_secs);

        loop {
            match self.run_recovery_cycle().await {
                Ok(stats) => {
                    if stats.tasks_recovered > 0
                        || stats.tasks_reassigned > 0
                        || stats.circuits_opened > 0
                        || stats.contributions_stuck > 0
                    {
                        tracing::info!(
                            "Recovery cycle complete: {} tasks recovered, {} reassigned, {} circuits opened, {} stuck contributions",
                            stats.tasks_recovered,
                            stats.tasks_reassigned,
                            stats.circuits_opened,
                            stats.contributions_stuck
                        );
                    }
                }
                Err(e) => {
                    tracing::error!("Recovery cycle failed: {}", e);
                }
            }

            time::sleep(interval).await;
        }
    }

    /// Run a single recovery cycle
    pub async fn run_recovery_cycle(&self) -> Result<RecoveryCycleStats, DbError> {
        let start = std::time::Instant::now();
        let mut stats = RecoveryCycleStats::default();

        // 1. Check for stalled tasks
        if let Err(e) = self.recover_stalled_tasks(&mut stats) {
            stats.errors.push(format!("Stalled task recovery: {}", e));
        }

        // 2. Check agent health
        if let Err(e) = self.check_agent_health(&mut stats) {
            stats.errors.push(format!("Agent health check: {}", e));
        }

        // 3. Update circuit breakers
        if self.config.enable_circuit_breaker {
            if let Err(e) = self.update_circuit_breakers(&mut stats) {
                stats.errors.push(format!("Circuit breaker update: {}", e));
            }
        }

        // 4. Check for stuck contributions
        if let Err(e) = self.check_stuck_contributions(&mut stats) {
            stats.errors.push(format!("Stuck contribution check: {}", e));
        }

        stats.duration_ms = start.elapsed().as_millis() as u64;
        Ok(stats)
    }

    /// Recover stalled tasks (tasks running longer than timeout)
    fn recover_stalled_tasks(&self, stats: &mut RecoveryCycleStats) -> Result<(), DbError> {
        let db = self.storage.get_database(&self.db_name)?;

        // Check if tasks collection exists
        if db.get_collection("_ai_tasks").is_err() {
            return Ok(());
        }

        let tasks_coll = db.get_collection("_ai_tasks")?;
        let now = Utc::now();

        for doc in tasks_coll.scan(None) {
            let task: AITask = match serde_json::from_value(doc.to_value()) {
                Ok(t) => t,
                Err(_) => continue,
            };

            // Only check running tasks
            if task.status != AITaskStatus::Running {
                continue;
            }

            // Check if task has exceeded timeout
            let started_at = match task.started_at {
                Some(t) => t,
                None => continue,
            };

            let timeout_secs = self.config.get_task_timeout(&task.task_type) as i64;
            let elapsed = (now - started_at).num_seconds();

            if elapsed > timeout_secs {
                // Task is stalled - attempt recovery
                match self.recover_task(&task) {
                    Ok(recovered) => {
                        if recovered {
                            stats.tasks_recovered += 1;
                            self.log_event(RecoveryEvent::task_recovered(
                                &task.id,
                                format!(
                                    "Task stalled for {} seconds (timeout: {}s), reset to pending",
                                    elapsed, timeout_secs
                                ),
                            ))?;
                        } else {
                            stats.tasks_cancelled += 1;
                        }
                    }
                    Err(e) => {
                        stats.errors.push(format!("Failed to recover task {}: {}", task.id, e));
                    }
                }
            }
        }

        Ok(())
    }

    /// Attempt to recover a stalled task
    fn recover_task(&self, task: &AITask) -> Result<bool, DbError> {
        let db = self.storage.get_database(&self.db_name)?;
        let tasks_coll = db.get_collection("_ai_tasks")?;

        let mut task = task.clone();

        // Check retry count
        if task.retry_count >= self.config.max_recovery_retries {
            // Max retries exceeded - cancel task
            task.status = AITaskStatus::Failed;
            task.completed_at = Some(Utc::now());
            task.error = Some("Max recovery retries exceeded".to_string());

            let task_value = serde_json::to_value(&task)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
            tasks_coll.update(&task.id, task_value)?;

            return Ok(false);
        }

        // Reset task for retry
        task.status = AITaskStatus::Pending;
        task.started_at = None;
        task.retry_count += 1;

        // Unassign from current agent if task reassignment is enabled
        if self.config.enable_task_reassignment {
            if let Some(ref agent_id) = task.agent_id {
                // Record failure for the agent
                self.record_agent_failure(agent_id)?;
            }
            task.agent_id = None;
        }

        let task_value = serde_json::to_value(&task)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        tasks_coll.update(&task.id, task_value)?;

        Ok(true)
    }

    /// Check health of all agents
    fn check_agent_health(&self, stats: &mut RecoveryCycleStats) -> Result<(), DbError> {
        let db = self.storage.get_database(&self.db_name)?;

        // Check if agents collection exists
        if db.get_collection("_ai_agents").is_err() {
            return Ok(());
        }

        let agents_coll = db.get_collection("_ai_agents")?;
        let timeout = self.config.agent_heartbeat_timeout_secs;

        // Ensure health collection exists
        if db.get_collection(AGENT_HEALTH_COLLECTION).is_err() {
            db.create_collection(AGENT_HEALTH_COLLECTION.to_string(), None)?;
        }
        let health_coll = db.get_collection(AGENT_HEALTH_COLLECTION)?;

        for doc in agents_coll.scan(None) {
            let agent: crate::ai::Agent = match serde_json::from_value(doc.to_value()) {
                Ok(a) => a,
                Err(_) => continue,
            };

            // Get or create health metrics
            let mut health = match health_coll.get(&agent.id) {
                Ok(h) => serde_json::from_value(h.to_value()).unwrap_or_else(|_| {
                    AgentHealthMetrics::new(agent.id.clone())
                }),
                Err(_) => AgentHealthMetrics::new(agent.id.clone()),
            };

            // Check heartbeat
            if !health.is_healthy(timeout) {
                // Agent is unhealthy
                if health.circuit_state != CircuitState::Open {
                    stats.agents_unhealthy += 1;
                    self.log_event(RecoveryEvent::agent_unhealthy(
                        &agent.id,
                        &format!("No heartbeat for {} seconds", timeout),
                    ))?;
                }
            }

            // Update health metrics
            health.updated_at = Utc::now();
            let health_value = serde_json::to_value(&health)
                .map_err(|e| DbError::InternalError(e.to_string()))?;

            if health_coll.get(&agent.id).is_ok() {
                health_coll.update(&agent.id, health_value)?;
            } else {
                health_coll.insert(health_value)?;
            }
        }

        Ok(())
    }

    /// Update circuit breaker states
    fn update_circuit_breakers(&self, stats: &mut RecoveryCycleStats) -> Result<(), DbError> {
        let db = self.storage.get_database(&self.db_name)?;

        if db.get_collection(AGENT_HEALTH_COLLECTION).is_err() {
            return Ok(());
        }

        let health_coll = db.get_collection(AGENT_HEALTH_COLLECTION)?;

        for doc in health_coll.scan(None) {
            let mut health: AgentHealthMetrics = match serde_json::from_value(doc.to_value()) {
                Ok(h) => h,
                Err(_) => continue,
            };

            let original_state = health.circuit_state;

            // Check if open circuit should try half-open
            if health.should_try_half_open() {
                health.transition_to_half_open();

                let health_value = serde_json::to_value(&health)
                    .map_err(|e| DbError::InternalError(e.to_string()))?;
                health_coll.update(&health.agent_id, health_value)?;
            }

            // Count state changes
            if original_state != health.circuit_state {
                match health.circuit_state {
                    CircuitState::Open => stats.circuits_opened += 1,
                    CircuitState::Closed => stats.circuits_closed += 1,
                    CircuitState::HalfOpen => {}
                }
            }
        }

        Ok(())
    }

    /// Check for stuck contributions
    fn check_stuck_contributions(&self, stats: &mut RecoveryCycleStats) -> Result<(), DbError> {
        let db = self.storage.get_database(&self.db_name)?;

        if db.get_collection("_ai_contributions").is_err() {
            return Ok(());
        }

        let contrib_coll = db.get_collection("_ai_contributions")?;
        let now = Utc::now();
        let stuck_threshold = Duration::seconds(self.config.contribution_stuck_timeout_secs as i64);

        for doc in contrib_coll.scan(None) {
            let contribution: Contribution = match serde_json::from_value(doc.to_value()) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Check if contribution is in an intermediate status for too long
            let is_intermediate = matches!(
                contribution.status,
                ContributionStatus::Analyzing
                    | ContributionStatus::Generating
                    | ContributionStatus::Validating
            );

            if !is_intermediate {
                continue;
            }

            let elapsed = now - contribution.updated_at;
            if elapsed > stuck_threshold {
                stats.contributions_stuck += 1;
                let duration_mins = elapsed.num_minutes() as u64;

                self.log_event(RecoveryEvent::contribution_stuck(
                    &contribution.id,
                    &contribution.status.to_string(),
                    duration_mins,
                ))?;
            }
        }

        Ok(())
    }

    /// Record a failure for an agent
    fn record_agent_failure(&self, agent_id: &str) -> Result<(), DbError> {
        let db = self.storage.get_database(&self.db_name)?;

        if db.get_collection(AGENT_HEALTH_COLLECTION).is_err() {
            db.create_collection(AGENT_HEALTH_COLLECTION.to_string(), None)?;
        }

        let health_coll = db.get_collection(AGENT_HEALTH_COLLECTION)?;

        let mut health = match health_coll.get(agent_id) {
            Ok(h) => serde_json::from_value(h.to_value())
                .unwrap_or_else(|_| AgentHealthMetrics::new(agent_id.to_string())),
            Err(_) => AgentHealthMetrics::new(agent_id.to_string()),
        };

        let was_closed = health.circuit_state == CircuitState::Closed;

        health.record_failure(
            self.config.circuit_failure_threshold,
            self.config.circuit_failure_rate_threshold,
        );

        // If circuit just opened, set retry time
        if health.circuit_state == CircuitState::Open && was_closed {
            let retry_at = Utc::now() + Duration::seconds(self.config.circuit_cooldown_secs as i64);
            health.set_retry_at(retry_at);

            self.log_event(RecoveryEvent::circuit_opened(
                agent_id,
                health.consecutive_failures,
                health.failure_rate(),
            ))?;
        }

        let health_value = serde_json::to_value(&health)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        if health_coll.get(agent_id).is_ok() {
            health_coll.update(agent_id, health_value)?;
        } else {
            health_coll.insert(health_value)?;
        }

        Ok(())
    }

    /// Log a recovery event
    fn log_event(&self, event: RecoveryEvent) -> Result<(), DbError> {
        let db = self.storage.get_database(&self.db_name)?;

        if db.get_collection(RECOVERY_EVENTS_COLLECTION).is_err() {
            db.create_collection(RECOVERY_EVENTS_COLLECTION.to_string(), None)?;
        }

        let events_coll = db.get_collection(RECOVERY_EVENTS_COLLECTION)?;
        let event_value = serde_json::to_value(&event)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        events_coll.insert(event_value)?;

        Ok(())
    }

    /// Get current recovery system status
    pub fn get_status(&self) -> Result<RecoverySystemStatus, DbError> {
        let db = self.storage.get_database(&self.db_name)?;
        let mut status = RecoverySystemStatus::default();

        // Count agents
        if let Ok(agents_coll) = db.get_collection("_ai_agents") {
            status.total_agents = agents_coll.scan(None).len();
        }

        // Count health metrics
        if let Ok(health_coll) = db.get_collection(AGENT_HEALTH_COLLECTION) {
            for doc in health_coll.scan(None) {
                if let Ok(health) = serde_json::from_value::<AgentHealthMetrics>(doc.to_value()) {
                    if health.circuit_state == CircuitState::Open {
                        status.agents_circuit_open += 1;
                    }
                    if !health.is_healthy(self.config.agent_heartbeat_timeout_secs) {
                        status.agents_unhealthy += 1;
                    }
                }
            }
        }

        // Count stalled tasks
        if let Ok(tasks_coll) = db.get_collection("_ai_tasks") {
            let now = Utc::now();
            for doc in tasks_coll.scan(None) {
                if let Ok(task) = serde_json::from_value::<AITask>(doc.to_value()) {
                    if task.status == AITaskStatus::Running {
                        if let Some(started_at) = task.started_at {
                            let timeout = self.config.get_task_timeout(&task.task_type) as i64;
                            if (now - started_at).num_seconds() > timeout {
                                status.stalled_tasks += 1;
                            }
                        }
                    }
                }
            }
        }

        // Count recent events
        if let Ok(events_coll) = db.get_collection(RECOVERY_EVENTS_COLLECTION) {
            let one_hour_ago = Utc::now() - Duration::hours(1);
            for doc in events_coll.scan(None) {
                if let Ok(event) = serde_json::from_value::<RecoveryEvent>(doc.to_value()) {
                    if event.created_at >= one_hour_ago {
                        status.recent_events += 1;
                    }
                }
            }
        }

        status.last_scan = Some(Utc::now());
        Ok(status)
    }

    /// Force retry a specific task
    pub fn force_retry_task(&self, task_id: &str) -> Result<bool, DbError> {
        let db = self.storage.get_database(&self.db_name)?;
        let tasks_coll = db.get_collection("_ai_tasks")?;

        let doc = tasks_coll.get(task_id)?;
        let task: AITask = serde_json::from_value(doc.to_value())
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        // Can only retry failed or running (stalled) tasks
        if task.status != AITaskStatus::Failed && task.status != AITaskStatus::Running {
            return Err(DbError::BadRequest(format!(
                "Cannot retry task in {} status",
                task.status
            )));
        }

        let mut task = task;
        task.status = AITaskStatus::Pending;
        task.started_at = None;
        task.completed_at = None;
        task.retry_count += 1;
        task.agent_id = None;
        task.error = None;

        let task_value = serde_json::to_value(&task)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        tasks_coll.update(task_id, task_value)?;

        self.log_event(
            RecoveryEvent::task_recovered(task_id, "Manual retry triggered".to_string())
                .with_context(serde_json::json!({"manual": true})),
        )?;

        Ok(true)
    }

    /// Reset circuit breaker for an agent
    pub fn reset_circuit_breaker(&self, agent_id: &str) -> Result<(), DbError> {
        let db = self.storage.get_database(&self.db_name)?;

        if db.get_collection(AGENT_HEALTH_COLLECTION).is_err() {
            return Err(DbError::DocumentNotFound(format!(
                "No health record for agent {}",
                agent_id
            )));
        }

        let health_coll = db.get_collection(AGENT_HEALTH_COLLECTION)?;
        let doc = health_coll.get(agent_id)?;

        let mut health: AgentHealthMetrics = serde_json::from_value(doc.to_value())
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        health.transition_to_closed();

        let health_value = serde_json::to_value(&health)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        health_coll.update(agent_id, health_value)?;

        self.log_event(RecoveryEvent::circuit_closed(agent_id))?;

        Ok(())
    }

    /// List recovery events
    pub fn list_events(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<RecoveryEvent>, DbError> {
        let db = self.storage.get_database(&self.db_name)?;

        if db.get_collection(RECOVERY_EVENTS_COLLECTION).is_err() {
            return Ok(Vec::new());
        }

        let events_coll = db.get_collection(RECOVERY_EVENTS_COLLECTION)?;
        let limit = limit.unwrap_or(100);

        let mut events: Vec<RecoveryEvent> = events_coll
            .scan(None)
            .into_iter()
            .filter_map(|doc| serde_json::from_value::<RecoveryEvent>(doc.to_value()).ok())
            .collect();

        // Sort by created_at descending
        events.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        events.truncate(limit);
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageEngine;
    use tempfile::TempDir;

    fn setup_test_storage() -> (Arc<StorageEngine>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = StorageEngine::new(temp_dir.path().to_str().unwrap()).unwrap();
        storage.create_database("test_db".to_string()).unwrap();
        (Arc::new(storage), temp_dir)
    }

    #[test]
    fn test_recovery_worker_creation() {
        let (storage, _dir) = setup_test_storage();
        let config = RecoveryConfig::minimal();
        let worker = RecoveryWorker::new(storage, "test_db".to_string(), config);
        assert_eq!(worker.db_name, "test_db");
    }

    #[test]
    fn test_get_status_empty() {
        let (storage, _dir) = setup_test_storage();
        let config = RecoveryConfig::minimal();
        let worker = RecoveryWorker::new(storage, "test_db".to_string(), config);

        let status = worker.get_status().unwrap();
        assert_eq!(status.total_agents, 0);
        assert_eq!(status.stalled_tasks, 0);
    }

    #[test]
    fn test_list_events_empty() {
        let (storage, _dir) = setup_test_storage();
        let config = RecoveryConfig::minimal();
        let worker = RecoveryWorker::new(storage, "test_db".to_string(), config);

        let events = worker.list_events(None).unwrap();
        assert!(events.is_empty());
    }
}
