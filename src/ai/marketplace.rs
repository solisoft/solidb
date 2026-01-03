//! Agent Marketplace - Discovery, ranking, and trust management
//!
//! This module provides agent discovery, trust scoring, and ranking capabilities
//! for the AI contribution pipeline.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

use super::agent::{Agent, AgentStatus, AgentType};
use super::task::{AITask, AITaskType};
use crate::error::DbError;
use crate::storage::Database;

/// Default window size for recent performance tracking
const DEFAULT_WINDOW_SIZE: usize = 20;

/// Default starting trust score for new agents
const DEFAULT_TRUST_SCORE: f64 = 0.5;

// ============================================================================
// Reputation Data Structures
// ============================================================================

/// Agent reputation and performance metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReputation {
    /// Agent ID (same as agent's _key)
    #[serde(rename = "_key")]
    pub agent_id: String,

    /// Overall trust score (0.0 - 1.0)
    pub trust_score: f64,

    /// Success rate by task type (task_type -> rate)
    #[serde(default)]
    pub success_rates: HashMap<String, f64>,

    /// Average task completion time in ms by task type
    #[serde(default)]
    pub avg_completion_times: HashMap<String, u64>,

    /// Total tasks completed by type
    #[serde(default)]
    pub tasks_by_type: HashMap<String, TaskTypeStats>,

    /// Recent performance window (last N tasks)
    pub recent_window: RecentPerformance,

    /// Capability verification status
    #[serde(default)]
    pub verified_capabilities: Vec<VerifiedCapability>,

    /// Last reputation update timestamp
    pub updated_at: DateTime<Utc>,

    /// Reputation history (for trend analysis)
    #[serde(default)]
    pub history: Vec<ReputationSnapshot>,
}

impl AgentReputation {
    /// Create a new reputation record for an agent
    pub fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            trust_score: DEFAULT_TRUST_SCORE,
            success_rates: HashMap::new(),
            avg_completion_times: HashMap::new(),
            tasks_by_type: HashMap::new(),
            recent_window: RecentPerformance::new(DEFAULT_WINDOW_SIZE),
            verified_capabilities: Vec::new(),
            updated_at: Utc::now(),
            history: Vec::new(),
        }
    }

    /// Calculate trust score from metrics
    pub fn calculate_trust_score(&mut self) {
        let base_score = self.calculate_base_score();
        let recency_factor = self.calculate_recency_factor();
        let consistency_factor = self.calculate_consistency_factor();

        // Weighted combination
        self.trust_score = (base_score * 0.5 + recency_factor * 0.3 + consistency_factor * 0.2)
            .clamp(0.0, 1.0);

        self.updated_at = Utc::now();
    }

    /// Calculate base score from overall success rate
    fn calculate_base_score(&self) -> f64 {
        let total_completed: u64 = self.tasks_by_type.values().map(|s| s.completed).sum();
        let total_failed: u64 = self.tasks_by_type.values().map(|s| s.failed).sum();

        if total_completed + total_failed == 0 {
            return DEFAULT_TRUST_SCORE; // Default for new agents
        }

        total_completed as f64 / (total_completed + total_failed) as f64
    }

    /// Calculate recency factor from recent window
    fn calculate_recency_factor(&self) -> f64 {
        self.recent_window.recent_success_rate
    }

    /// Calculate consistency factor from variance in success rates
    fn calculate_consistency_factor(&self) -> f64 {
        if self.success_rates.is_empty() {
            return DEFAULT_TRUST_SCORE;
        }

        let rates: Vec<f64> = self.success_rates.values().copied().collect();
        let mean: f64 = rates.iter().sum::<f64>() / rates.len() as f64;
        let variance: f64 =
            rates.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / rates.len() as f64;

        // Convert variance to 0-1 score (lower variance = higher score)
        1.0 - variance.sqrt().min(1.0)
    }

    /// Record a task completion
    pub fn record_task(&mut self, task_type: &str, success: bool, duration_ms: u64) {
        // Update tasks_by_type
        let stats = self
            .tasks_by_type
            .entry(task_type.to_string())
            .or_insert_with(TaskTypeStats::default);

        if success {
            stats.completed += 1;
        } else {
            stats.failed += 1;
        }
        stats.total_duration_ms += duration_ms;

        // Update success rate for this task type
        let total = stats.completed + stats.failed;
        self.success_rates
            .insert(task_type.to_string(), stats.completed as f64 / total as f64);

        // Update average completion time
        self.avg_completion_times
            .insert(task_type.to_string(), stats.total_duration_ms / total);

        // Update recent window
        self.recent_window.record(success);

        // Recalculate trust score
        self.calculate_trust_score();

        // Save snapshot every 10 tasks
        let total_tasks: u64 = self
            .tasks_by_type
            .values()
            .map(|s| s.completed + s.failed)
            .sum();
        if total_tasks % 10 == 0 {
            self.history.push(ReputationSnapshot {
                timestamp: Utc::now(),
                trust_score: self.trust_score,
                total_tasks,
            });

            // Keep only last 100 snapshots
            if self.history.len() > 100 {
                self.history.remove(0);
            }
        }
    }

    /// Verify a capability based on successful task completion
    pub fn verify_capability(&mut self, capability: &str, task_type: &str) {
        // Check if we have enough successful tasks using this capability
        if let Some(stats) = self.tasks_by_type.get(task_type) {
            if stats.completed >= 3 {
                // Require at least 3 successes
                // Check if already verified
                if !self
                    .verified_capabilities
                    .iter()
                    .any(|v| v.capability == capability)
                {
                    self.verified_capabilities.push(VerifiedCapability {
                        capability: capability.to_string(),
                        verified_at: Utc::now(),
                        verification_method: VerificationMethod::TaskSuccess,
                        usage_count: stats.completed,
                    });
                } else {
                    // Update usage count
                    if let Some(v) = self
                        .verified_capabilities
                        .iter_mut()
                        .find(|v| v.capability == capability)
                    {
                        v.usage_count = stats.completed;
                    }
                }
            }
        }
    }
}

/// Statistics for a specific task type
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskTypeStats {
    pub completed: u64,
    pub failed: u64,
    pub total_duration_ms: u64,
}

/// Recent performance window
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentPerformance {
    /// Last N task outcomes (true = success)
    pub outcomes: VecDeque<bool>,
    /// Window size
    pub window_size: usize,
    /// Recent success rate (calculated from outcomes)
    pub recent_success_rate: f64,
}

impl RecentPerformance {
    pub fn new(window_size: usize) -> Self {
        Self {
            outcomes: VecDeque::with_capacity(window_size),
            window_size,
            recent_success_rate: DEFAULT_TRUST_SCORE, // Default until we have data
        }
    }

    pub fn record(&mut self, success: bool) {
        if self.outcomes.len() >= self.window_size {
            self.outcomes.pop_front();
        }
        self.outcomes.push_back(success);

        // Recalculate success rate
        if !self.outcomes.is_empty() {
            let successes = self.outcomes.iter().filter(|&&s| s).count();
            self.recent_success_rate = successes as f64 / self.outcomes.len() as f64;
        }
    }
}

/// A verified capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedCapability {
    pub capability: String,
    pub verified_at: DateTime<Utc>,
    pub verification_method: VerificationMethod,
    /// Number of successful tasks using this capability
    pub usage_count: u64,
}

/// How a capability was verified
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationMethod {
    /// Self-declared by agent
    Declared,
    /// Verified by successful task completion
    TaskSuccess,
    /// Manually verified by admin
    AdminVerified,
}

/// A snapshot of reputation at a point in time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationSnapshot {
    pub timestamp: DateTime<Utc>,
    pub trust_score: f64,
    pub total_tasks: u64,
}

// ============================================================================
// Discovery Data Structures
// ============================================================================

/// Query for discovering agents in the marketplace
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentDiscoveryQuery {
    /// Required capabilities (AND logic)
    #[serde(default)]
    pub required_capabilities: Option<Vec<String>>,
    /// Preferred agent type
    pub agent_type: Option<AgentType>,
    /// Minimum trust score
    pub min_trust_score: Option<f64>,
    /// Maximum agents to return
    pub limit: Option<usize>,
    /// Task type for specialized ranking
    pub task_type: Option<String>,
    /// Only return idle agents
    pub idle_only: Option<bool>,
}

/// Ranked agent result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedAgent {
    pub agent: Agent,
    pub reputation: AgentReputation,
    /// Calculated suitability score for the query
    pub suitability_score: f64,
    /// Breakdown of how score was calculated
    pub score_breakdown: ScoreBreakdown,
}

/// Breakdown of suitability score calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub trust_component: f64,
    pub capability_match: f64,
    pub availability_bonus: f64,
    pub task_type_affinity: f64,
}

/// Response for agent discovery
#[derive(Debug, Serialize)]
pub struct AgentDiscoveryResponse {
    pub agents: Vec<RankedAgent>,
    pub total: usize,
}

/// Response for agent rankings
#[derive(Debug, Serialize)]
pub struct AgentRankingsResponse {
    pub rankings: Vec<AgentRanking>,
    pub total: usize,
}

/// A single agent's ranking
#[derive(Debug, Clone, Serialize)]
pub struct AgentRanking {
    pub agent_id: String,
    pub agent_name: String,
    pub agent_type: AgentType,
    pub trust_score: f64,
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub success_rate: f64,
}

// ============================================================================
// Marketplace Implementation
// ============================================================================

/// Agent Marketplace for discovery, ranking, and trust management
pub struct AgentMarketplace;

impl AgentMarketplace {
    /// Discover and rank agents for a task
    pub fn discover_agents(
        db: &Database,
        query: &AgentDiscoveryQuery,
    ) -> Result<Vec<RankedAgent>, DbError> {
        let agents_coll = db.get_collection("_ai_agents")?;

        // Get or create reputations collection
        if db.get_collection("_ai_agent_reputations").is_err() {
            db.create_collection("_ai_agent_reputations".to_string(), None)?;
        }
        let reputations_coll = db.get_collection("_ai_agent_reputations")?;

        let mut ranked_agents = Vec::new();

        for doc in agents_coll.scan(None) {
            let agent: Agent = serde_json::from_value(doc.to_value())
                .map_err(|_| DbError::InternalError("Corrupted agent data".to_string()))?;

            // Apply filters
            if !Self::matches_query(&agent, query) {
                continue;
            }

            // Get or create reputation
            let reputation = match reputations_coll.get(&agent.id) {
                Ok(rep_doc) => serde_json::from_value(rep_doc.to_value())
                    .map_err(|_| DbError::InternalError("Corrupted reputation data".to_string()))?,
                Err(_) => {
                    // Initialize reputation for agent without one
                    let rep = AgentReputation::new(agent.id.clone());
                    let rep_value = serde_json::to_value(&rep)
                        .map_err(|e| DbError::InternalError(e.to_string()))?;
                    let _ = reputations_coll.insert(rep_value);
                    rep
                }
            };

            // Calculate suitability score
            let (suitability_score, score_breakdown) =
                Self::calculate_suitability_score(&agent, &reputation, query);

            ranked_agents.push(RankedAgent {
                agent,
                reputation,
                suitability_score,
                score_breakdown,
            });
        }

        // Sort by suitability score descending
        ranked_agents.sort_by(|a, b| {
            b.suitability_score
                .partial_cmp(&a.suitability_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Apply limit
        if let Some(limit) = query.limit {
            ranked_agents.truncate(limit);
        }

        Ok(ranked_agents)
    }

    /// Check if an agent matches the discovery query
    fn matches_query(agent: &Agent, query: &AgentDiscoveryQuery) -> bool {
        // Check agent type
        if let Some(ref agent_type) = query.agent_type {
            if agent.agent_type != *agent_type {
                return false;
            }
        }

        // Check idle status
        if query.idle_only.unwrap_or(false) && agent.status != AgentStatus::Idle {
            return false;
        }

        // Check required capabilities
        if let Some(ref required) = query.required_capabilities {
            for cap in required {
                if !agent.capabilities.contains(cap) {
                    return false;
                }
            }
        }

        true
    }

    /// Calculate suitability score for an agent
    fn calculate_suitability_score(
        agent: &Agent,
        reputation: &AgentReputation,
        query: &AgentDiscoveryQuery,
    ) -> (f64, ScoreBreakdown) {
        // Base trust component (40%)
        let trust_component = reputation.trust_score;

        // Capability match (30%)
        let capability_match = if let Some(ref required) = query.required_capabilities {
            if required.is_empty() {
                1.0
            } else {
                let matched = required
                    .iter()
                    .filter(|c| agent.capabilities.contains(c))
                    .count();

                // Bonus for verified capabilities
                let verified_bonus = required
                    .iter()
                    .filter(|c| {
                        reputation
                            .verified_capabilities
                            .iter()
                            .any(|v| &v.capability == *c)
                    })
                    .count() as f64
                    * 0.1;

                (matched as f64 / required.len() as f64) + verified_bonus.min(0.2)
            }
        } else {
            1.0
        };

        // Availability bonus (15%)
        let availability_bonus = match agent.status {
            AgentStatus::Idle => 1.0,
            AgentStatus::Busy => 0.3,
            AgentStatus::Offline => 0.0,
            AgentStatus::Error => 0.0,
        };

        // Task type affinity (15%)
        let task_type_affinity = if let Some(ref task_type) = query.task_type {
            reputation
                .success_rates
                .get(task_type)
                .copied()
                .unwrap_or(DEFAULT_TRUST_SCORE)
        } else {
            DEFAULT_TRUST_SCORE
        };

        // Check minimum trust score
        if let Some(min_score) = query.min_trust_score {
            if reputation.trust_score < min_score {
                return (
                    0.0,
                    ScoreBreakdown {
                        trust_component: 0.0,
                        capability_match: 0.0,
                        availability_bonus: 0.0,
                        task_type_affinity: 0.0,
                    },
                );
            }
        }

        let score = trust_component * 0.4
            + capability_match * 0.3
            + availability_bonus * 0.15
            + task_type_affinity * 0.15;

        (
            score,
            ScoreBreakdown {
                trust_component,
                capability_match,
                availability_bonus,
                task_type_affinity,
            },
        )
    }

    /// Select the best agent for a specific task
    pub fn select_agent_for_task(
        db: &Database,
        task: &AITask,
    ) -> Result<Option<RankedAgent>, DbError> {
        let agent_type = Self::task_type_to_agent_type(&task.task_type);

        let query = AgentDiscoveryQuery {
            agent_type: Some(agent_type),
            idle_only: Some(true),
            task_type: Some(task.task_type.to_string()),
            limit: Some(1),
            ..Default::default()
        };

        let agents = Self::discover_agents(db, &query)?;
        Ok(agents.into_iter().next())
    }

    /// Map task type to agent type
    pub fn task_type_to_agent_type(task_type: &AITaskType) -> AgentType {
        match task_type {
            AITaskType::AnalyzeContribution => AgentType::Analyzer,
            AITaskType::GenerateCode => AgentType::Coder,
            AITaskType::ValidateCode => AgentType::Reviewer,
            AITaskType::RunTests => AgentType::Tester,
            AITaskType::PrepareReview => AgentType::Reviewer,
            AITaskType::MergeChanges => AgentType::Integrator,
        }
    }

    /// Update agent reputation after task completion
    pub fn update_reputation(
        db: &Database,
        agent_id: &str,
        task: &AITask,
        success: bool,
        duration_ms: u64,
    ) -> Result<AgentReputation, DbError> {
        // Get or create reputations collection
        if db.get_collection("_ai_agent_reputations").is_err() {
            db.create_collection("_ai_agent_reputations".to_string(), None)?;
        }
        let coll = db.get_collection("_ai_agent_reputations")?;

        // Get or create reputation
        let mut reputation = match coll.get(agent_id) {
            Ok(doc) => serde_json::from_value(doc.to_value())
                .map_err(|_| DbError::InternalError("Corrupted reputation data".to_string()))?,
            Err(_) => AgentReputation::new(agent_id.to_string()),
        };

        // Record the task
        reputation.record_task(&task.task_type.to_string(), success, duration_ms);

        // Save updated reputation
        let value = serde_json::to_value(&reputation)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        if coll.get(agent_id).is_ok() {
            coll.update(agent_id, value)?;
        } else {
            coll.insert(value)?;
        }

        Ok(reputation)
    }

    /// Initialize reputation for a new agent
    pub fn initialize_reputation(db: &Database, agent_id: &str) -> Result<AgentReputation, DbError> {
        // Get or create reputations collection
        if db.get_collection("_ai_agent_reputations").is_err() {
            db.create_collection("_ai_agent_reputations".to_string(), None)?;
        }
        let coll = db.get_collection("_ai_agent_reputations")?;

        // Check if already exists
        if coll.get(agent_id).is_ok() {
            return Err(DbError::CollectionAlreadyExists(format!(
                "Reputation already exists for agent {}",
                agent_id
            )));
        }

        let reputation = AgentReputation::new(agent_id.to_string());
        let value = serde_json::to_value(&reputation)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        coll.insert(value)?;

        Ok(reputation)
    }

    /// Get agent reputation
    pub fn get_reputation(db: &Database, agent_id: &str) -> Result<AgentReputation, DbError> {
        let coll = db.get_collection("_ai_agent_reputations")?;
        let doc = coll.get(agent_id)?;
        serde_json::from_value(doc.to_value())
            .map_err(|_| DbError::InternalError("Corrupted reputation data".to_string()))
    }

    /// Get agent rankings
    pub fn get_rankings(db: &Database, limit: Option<usize>) -> Result<Vec<AgentRanking>, DbError> {
        let agents_coll = db.get_collection("_ai_agents")?;

        // Get or create reputations collection
        if db.get_collection("_ai_agent_reputations").is_err() {
            db.create_collection("_ai_agent_reputations".to_string(), None)?;
        }
        let reputations_coll = db.get_collection("_ai_agent_reputations")?;

        let mut rankings = Vec::new();

        for doc in agents_coll.scan(None) {
            let agent: Agent = serde_json::from_value(doc.to_value())
                .map_err(|_| DbError::InternalError("Corrupted agent data".to_string()))?;

            let reputation = match reputations_coll.get(&agent.id) {
                Ok(rep_doc) => serde_json::from_value::<AgentReputation>(rep_doc.to_value())
                    .unwrap_or_else(|_| AgentReputation::new(agent.id.clone())),
                Err(_) => AgentReputation::new(agent.id.clone()),
            };

            let total = agent.tasks_completed + agent.tasks_failed;
            let success_rate = if total > 0 {
                agent.tasks_completed as f64 / total as f64
            } else {
                0.0
            };

            rankings.push(AgentRanking {
                agent_id: agent.id,
                agent_name: agent.name,
                agent_type: agent.agent_type,
                trust_score: reputation.trust_score,
                tasks_completed: agent.tasks_completed,
                tasks_failed: agent.tasks_failed,
                success_rate,
            });
        }

        // Sort by trust score descending
        rankings.sort_by(|a, b| {
            b.trust_score
                .partial_cmp(&a.trust_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if let Some(limit) = limit {
            rankings.truncate(limit);
        }

        Ok(rankings)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_reputation_new() {
        let rep = AgentReputation::new("agent-001".to_string());
        assert_eq!(rep.agent_id, "agent-001");
        assert_eq!(rep.trust_score, DEFAULT_TRUST_SCORE);
        assert!(rep.tasks_by_type.is_empty());
    }

    #[test]
    fn test_record_task_success() {
        let mut rep = AgentReputation::new("agent-001".to_string());
        rep.record_task("analyze_contribution", true, 1000);

        assert_eq!(rep.tasks_by_type.get("analyze_contribution").unwrap().completed, 1);
        assert_eq!(rep.tasks_by_type.get("analyze_contribution").unwrap().failed, 0);
        assert_eq!(rep.success_rates.get("analyze_contribution").unwrap(), &1.0);
    }

    #[test]
    fn test_record_task_failure() {
        let mut rep = AgentReputation::new("agent-001".to_string());
        rep.record_task("generate_code", false, 2000);

        assert_eq!(rep.tasks_by_type.get("generate_code").unwrap().completed, 0);
        assert_eq!(rep.tasks_by_type.get("generate_code").unwrap().failed, 1);
        assert_eq!(rep.success_rates.get("generate_code").unwrap(), &0.0);
    }

    #[test]
    fn test_trust_score_calculation() {
        let mut rep = AgentReputation::new("agent-001".to_string());

        // Record multiple tasks
        for _ in 0..8 {
            rep.record_task("analyze_contribution", true, 1000);
        }
        for _ in 0..2 {
            rep.record_task("analyze_contribution", false, 1500);
        }

        // Trust score should be above 0.5 (80% success rate)
        assert!(rep.trust_score > 0.5);
        assert!(rep.trust_score < 1.0);
    }

    #[test]
    fn test_recent_performance_window() {
        let mut window = RecentPerformance::new(5);

        window.record(true);
        window.record(true);
        window.record(false);

        assert_eq!(window.outcomes.len(), 3);
        assert!((window.recent_success_rate - 0.666).abs() < 0.01);

        // Fill window
        window.record(true);
        window.record(true);
        window.record(true); // This should push out the first

        assert_eq!(window.outcomes.len(), 5);
        assert_eq!(window.recent_success_rate, 0.8); // 4/5 successes
    }

    #[test]
    fn test_verify_capability() {
        let mut rep = AgentReputation::new("agent-001".to_string());

        // Record 3 successful tasks
        for _ in 0..3 {
            rep.record_task("analyze_contribution", true, 1000);
        }

        rep.verify_capability("rust", "analyze_contribution");

        assert_eq!(rep.verified_capabilities.len(), 1);
        assert_eq!(rep.verified_capabilities[0].capability, "rust");
        assert!(matches!(
            rep.verified_capabilities[0].verification_method,
            VerificationMethod::TaskSuccess
        ));
    }

    #[test]
    fn test_agent_matches_query() {
        let agent = Agent::new(
            "test-agent".to_string(),
            AgentType::Analyzer,
            vec!["rust".to_string(), "lua".to_string()],
        );

        // Match by type
        let query = AgentDiscoveryQuery {
            agent_type: Some(AgentType::Analyzer),
            ..Default::default()
        };
        assert!(AgentMarketplace::matches_query(&agent, &query));

        // No match - wrong type
        let query = AgentDiscoveryQuery {
            agent_type: Some(AgentType::Coder),
            ..Default::default()
        };
        assert!(!AgentMarketplace::matches_query(&agent, &query));

        // Match by capabilities
        let query = AgentDiscoveryQuery {
            required_capabilities: Some(vec!["rust".to_string()]),
            ..Default::default()
        };
        assert!(AgentMarketplace::matches_query(&agent, &query));

        // No match - missing capability
        let query = AgentDiscoveryQuery {
            required_capabilities: Some(vec!["python".to_string()]),
            ..Default::default()
        };
        assert!(!AgentMarketplace::matches_query(&agent, &query));
    }

    #[test]
    fn test_task_type_to_agent_type() {
        assert_eq!(
            AgentMarketplace::task_type_to_agent_type(&AITaskType::AnalyzeContribution),
            AgentType::Analyzer
        );
        assert_eq!(
            AgentMarketplace::task_type_to_agent_type(&AITaskType::GenerateCode),
            AgentType::Coder
        );
        assert_eq!(
            AgentMarketplace::task_type_to_agent_type(&AITaskType::RunTests),
            AgentType::Tester
        );
    }
}
