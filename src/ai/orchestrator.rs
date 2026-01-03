//! Task Orchestrator for the AI contribution pipeline
//!
//! This module handles the automatic progression of tasks through the pipeline.
//! When a task completes, it determines what the next task should be and creates it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::agent::AgentType;
use super::contribution::{Contribution, ContributionStatus};
use super::marketplace::{AgentMarketplace, RankedAgent};
use super::task::{AITask, AITaskType};
use crate::storage::Database;

/// The task orchestrator manages task flow through the pipeline
pub struct TaskOrchestrator;

/// Result of orchestration - what tasks to create next
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationResult {
    /// Tasks to create
    pub next_tasks: Vec<AITask>,
    /// New status for the contribution
    pub contribution_status: Option<ContributionStatus>,
    /// Message describing what happened
    pub message: String,
}

impl TaskOrchestrator {
    /// Determine the next steps after a task completes
    ///
    /// This is the core orchestration logic that implements the pipeline:
    /// 1. AnalyzeContribution → GenerateCode (or Review if high risk)
    /// 2. GenerateCode → ValidateCode
    /// 3. ValidateCode → RunTests
    /// 4. RunTests → PrepareReview (or direct to Review)
    /// 5. PrepareReview → (human review)
    /// 6. After approval → MergeChanges
    pub fn on_task_complete(
        task: &AITask,
        contribution: &Contribution,
        output: Option<&Value>,
    ) -> OrchestrationResult {
        match task.task_type {
            AITaskType::AnalyzeContribution => {
                Self::handle_analysis_complete(task, contribution, output)
            }
            AITaskType::GenerateCode => {
                Self::handle_generation_complete(task, contribution, output)
            }
            AITaskType::ValidateCode => {
                Self::handle_validation_complete(task, contribution, output)
            }
            AITaskType::RunTests => {
                Self::handle_tests_complete(task, contribution, output)
            }
            AITaskType::PrepareReview => {
                Self::handle_prepare_review_complete(task, contribution, output)
            }
            AITaskType::MergeChanges => {
                Self::handle_merge_complete(task, contribution, output)
            }
        }
    }

    /// Handle completion of analysis task
    fn handle_analysis_complete(
        task: &AITask,
        _contribution: &Contribution,
        output: Option<&Value>,
    ) -> OrchestrationResult {
        // Check if high risk or requires immediate review
        // Works with either full AnalysisResult struct or simple JSON with risk_score/requires_review
        let requires_review = output
            .map(|v| {
                // Check requires_review field
                let explicit_review = v.get("requires_review")
                    .and_then(|r| r.as_bool())
                    .unwrap_or(false);

                // Check risk_score field
                let high_risk = v.get("risk_score")
                    .and_then(|r| r.as_f64())
                    .map(|score| score > 0.7)
                    .unwrap_or(false);

                explicit_review || high_risk
            })
            .unwrap_or(false);

        if requires_review {
            // Skip to review for high-risk contributions
            OrchestrationResult {
                next_tasks: vec![AITask::new(
                    task.contribution_id.clone(),
                    AITaskType::PrepareReview,
                    task.priority,
                )],
                contribution_status: Some(ContributionStatus::Review),
                message: "High-risk contribution - proceeding to human review".to_string(),
            }
        } else {
            // Proceed to code generation
            let mut next_task = AITask::new(
                task.contribution_id.clone(),
                AITaskType::GenerateCode,
                task.priority,
            );
            // Pass analysis result to code generation
            next_task.input = output.cloned();

            OrchestrationResult {
                next_tasks: vec![next_task],
                contribution_status: Some(ContributionStatus::Generating),
                message: "Analysis complete - proceeding to code generation".to_string(),
            }
        }
    }

    /// Handle completion of code generation task
    fn handle_generation_complete(
        task: &AITask,
        _contribution: &Contribution,
        output: Option<&Value>,
    ) -> OrchestrationResult {
        let mut next_task = AITask::new(
            task.contribution_id.clone(),
            AITaskType::ValidateCode,
            task.priority,
        );
        // Pass generated code to validation
        next_task.input = output.cloned();

        OrchestrationResult {
            next_tasks: vec![next_task],
            contribution_status: Some(ContributionStatus::Validating),
            message: "Code generation complete - proceeding to validation".to_string(),
        }
    }

    /// Handle completion of code validation task
    fn handle_validation_complete(
        task: &AITask,
        _contribution: &Contribution,
        output: Option<&Value>,
    ) -> OrchestrationResult {
        // Check if validation passed
        let validation_passed = output
            .and_then(|v| v.get("passed"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if validation_passed {
            // Proceed to tests
            let mut next_task = AITask::new(
                task.contribution_id.clone(),
                AITaskType::RunTests,
                task.priority,
            );
            next_task.input = output.cloned();

            OrchestrationResult {
                next_tasks: vec![next_task],
                contribution_status: None, // Stay in Validating
                message: "Validation passed - proceeding to tests".to_string(),
            }
        } else {
            // Validation failed - go back to code generation with feedback
            let mut retry_task = AITask::new(
                task.contribution_id.clone(),
                AITaskType::GenerateCode,
                task.priority,
            );
            // Include validation errors as input for retry
            retry_task.input = Some(serde_json::json!({
                "retry": true,
                "validation_errors": output
            }));

            OrchestrationResult {
                next_tasks: vec![retry_task],
                contribution_status: Some(ContributionStatus::Generating),
                message: "Validation failed - retrying code generation".to_string(),
            }
        }
    }

    /// Handle completion of test run task
    fn handle_tests_complete(
        task: &AITask,
        contribution: &Contribution,
        output: Option<&Value>,
    ) -> OrchestrationResult {
        let tests_passed = output
            .and_then(|v| v.get("passed"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if tests_passed {
            // Check if human review is required
            if contribution.requires_human_review() {
                let mut next_task = AITask::new(
                    task.contribution_id.clone(),
                    AITaskType::PrepareReview,
                    task.priority,
                );
                next_task.input = output.cloned();

                OrchestrationResult {
                    next_tasks: vec![next_task],
                    contribution_status: Some(ContributionStatus::Review),
                    message: "Tests passed - preparing for human review".to_string(),
                }
            } else {
                // Auto-approve low-risk contributions
                let mut next_task = AITask::new(
                    task.contribution_id.clone(),
                    AITaskType::MergeChanges,
                    task.priority,
                );
                next_task.input = output.cloned();

                OrchestrationResult {
                    next_tasks: vec![next_task],
                    contribution_status: Some(ContributionStatus::Approved),
                    message: "Tests passed - auto-approved, proceeding to merge".to_string(),
                }
            }
        } else {
            // Tests failed - go back to code generation
            let mut retry_task = AITask::new(
                task.contribution_id.clone(),
                AITaskType::GenerateCode,
                task.priority,
            );
            retry_task.input = Some(serde_json::json!({
                "retry": true,
                "test_failures": output
            }));

            OrchestrationResult {
                next_tasks: vec![retry_task],
                contribution_status: Some(ContributionStatus::Generating),
                message: "Tests failed - retrying code generation".to_string(),
            }
        }
    }

    /// Handle completion of prepare review task
    fn handle_prepare_review_complete(
        _task: &AITask,
        _contribution: &Contribution,
        _output: Option<&Value>,
    ) -> OrchestrationResult {
        // Review preparation complete - now waiting for human
        // No automatic next task - human must approve/reject
        OrchestrationResult {
            next_tasks: vec![],
            contribution_status: Some(ContributionStatus::Review),
            message: "Ready for human review".to_string(),
        }
    }

    /// Handle completion of merge task
    fn handle_merge_complete(
        _task: &AITask,
        _contribution: &Contribution,
        _output: Option<&Value>,
    ) -> OrchestrationResult {
        // Merge complete - contribution is done!
        OrchestrationResult {
            next_tasks: vec![],
            contribution_status: Some(ContributionStatus::Merged),
            message: "Changes merged successfully".to_string(),
        }
    }

    /// Create a merge task after human approval
    pub fn on_approval(contribution: &Contribution, priority: i32) -> OrchestrationResult {
        let next_task = AITask::new(
            contribution.id.clone(),
            AITaskType::MergeChanges,
            priority,
        );

        OrchestrationResult {
            next_tasks: vec![next_task],
            contribution_status: None, // Already set to Approved
            message: "Approved - creating merge task".to_string(),
        }
    }

    /// Handle task failure - determine if retry or escalate
    pub fn on_task_failure(
        task: &AITask,
        _contribution: &Contribution,
        error: &str,
    ) -> OrchestrationResult {
        if task.can_retry() {
            // Task will be automatically retried by the task system
            OrchestrationResult {
                next_tasks: vec![],
                contribution_status: None,
                message: format!("Task failed, will retry: {}", error),
            }
        } else {
            // Max retries exceeded - escalate to review
            let mut review_task = AITask::new(
                task.contribution_id.clone(),
                AITaskType::PrepareReview,
                task.priority + 10, // Increase priority for failed tasks
            );
            review_task.input = Some(serde_json::json!({
                "escalated": true,
                "failed_task": task.task_type.to_string(),
                "error": error
            }));

            OrchestrationResult {
                next_tasks: vec![review_task],
                contribution_status: Some(ContributionStatus::Review),
                message: format!("Task failed after max retries - escalating to review: {}", error),
            }
        }
    }

    // =========================================
    // SMART AGENT SELECTION (using Marketplace)
    // =========================================

    /// Get the required agent type for a given task type
    pub fn get_required_agent_type(task_type: &AITaskType) -> AgentType {
        match task_type {
            AITaskType::AnalyzeContribution => AgentType::Analyzer,
            AITaskType::GenerateCode => AgentType::Coder,
            AITaskType::ValidateCode => AgentType::Reviewer,
            AITaskType::RunTests => AgentType::Tester,
            AITaskType::PrepareReview => AgentType::Reviewer,
            AITaskType::MergeChanges => AgentType::Integrator,
        }
    }

    /// Select the best agent for a task using the marketplace
    ///
    /// This method queries the agent marketplace to find the most suitable agent
    /// based on trust scores, recent performance, and availability.
    pub fn select_agent_for_task(db: &Database, task: &AITask) -> Option<RankedAgent> {
        AgentMarketplace::select_agent_for_task(db, task).ok().flatten()
    }

    /// Enhanced orchestration that includes agent selection
    ///
    /// This method performs normal orchestration plus smart agent selection
    /// for each new task created. The selected agent is recommended but not
    /// automatically assigned (the task remains in Pending state for agents to claim).
    pub fn on_task_complete_with_agent_selection(
        db: &Database,
        task: &AITask,
        contribution: &Contribution,
        output: Option<&Value>,
    ) -> OrchestrationResultWithAgents {
        // First, run normal orchestration
        let basic_result = Self::on_task_complete(task, contribution, output);

        // Then, select agents for each new task
        let tasks_with_agents: Vec<TaskWithRecommendedAgent> = basic_result
            .next_tasks
            .iter()
            .map(|task| {
                let recommended_agent = Self::select_agent_for_task(db, task);
                TaskWithRecommendedAgent {
                    task: task.clone(),
                    recommended_agent,
                }
            })
            .collect();

        OrchestrationResultWithAgents {
            tasks_with_agents,
            contribution_status: basic_result.contribution_status,
            message: basic_result.message,
        }
    }

    /// Get recommended agents for a set of pending tasks
    ///
    /// Useful for pre-computing agent assignments for task queues.
    pub fn recommend_agents_for_pending_tasks(
        db: &Database,
        tasks: &[AITask],
    ) -> Vec<TaskWithRecommendedAgent> {
        tasks
            .iter()
            .map(|task| {
                let recommended_agent = Self::select_agent_for_task(db, task);
                TaskWithRecommendedAgent {
                    task: task.clone(),
                    recommended_agent,
                }
            })
            .collect()
    }
}

/// Result of orchestration with smart agent selection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationResultWithAgents {
    /// Tasks to create with recommended agents
    pub tasks_with_agents: Vec<TaskWithRecommendedAgent>,
    /// New status for the contribution
    pub contribution_status: Option<ContributionStatus>,
    /// Message describing what happened
    pub message: String,
}

/// A task with its recommended agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskWithRecommendedAgent {
    /// The task to be created
    pub task: AITask,
    /// The recommended agent (if one was found)
    pub recommended_agent: Option<RankedAgent>,
}

/// Pipeline stage transitions for documentation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineStage {
    Submitted,
    Analyzing,
    Generating,
    Validating,
    Testing,
    Review,
    Approved,
    Merging,
    Merged,
    Rejected,
}

impl PipelineStage {
    /// Get the next stage in the happy path
    pub fn next(&self) -> Option<PipelineStage> {
        match self {
            PipelineStage::Submitted => Some(PipelineStage::Analyzing),
            PipelineStage::Analyzing => Some(PipelineStage::Generating),
            PipelineStage::Generating => Some(PipelineStage::Validating),
            PipelineStage::Validating => Some(PipelineStage::Testing),
            PipelineStage::Testing => Some(PipelineStage::Review),
            PipelineStage::Review => Some(PipelineStage::Approved),
            PipelineStage::Approved => Some(PipelineStage::Merging),
            PipelineStage::Merging => Some(PipelineStage::Merged),
            PipelineStage::Merged => None,
            PipelineStage::Rejected => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::contribution::ContributionType;

    fn create_test_contribution() -> Contribution {
        Contribution::new(
            ContributionType::Feature,
            "Test feature".to_string(),
            "test@example.com".to_string(),
            None,
        )
    }

    fn create_test_task(task_type: AITaskType) -> AITask {
        AITask::new("contrib-123".to_string(), task_type, 50)
    }

    #[test]
    fn test_analysis_complete_low_risk() {
        let task = create_test_task(AITaskType::AnalyzeContribution);
        let contribution = create_test_contribution();

        let output = serde_json::json!({
            "risk_score": 0.3,
            "requires_review": false,
            "affected_files": ["src/utils.rs"]
        });

        let result = TaskOrchestrator::on_task_complete(&task, &contribution, Some(&output));

        assert_eq!(result.next_tasks.len(), 1);
        assert_eq!(result.next_tasks[0].task_type, AITaskType::GenerateCode);
        assert_eq!(result.contribution_status, Some(ContributionStatus::Generating));
    }

    #[test]
    fn test_analysis_complete_high_risk() {
        let task = create_test_task(AITaskType::AnalyzeContribution);
        let contribution = create_test_contribution();

        let output = serde_json::json!({
            "risk_score": 0.9,
            "requires_review": true,
            "affected_files": ["src/storage/engine.rs"]
        });

        let result = TaskOrchestrator::on_task_complete(&task, &contribution, Some(&output));

        assert_eq!(result.next_tasks.len(), 1);
        assert_eq!(result.next_tasks[0].task_type, AITaskType::PrepareReview);
        assert_eq!(result.contribution_status, Some(ContributionStatus::Review));
    }

    #[test]
    fn test_validation_passed() {
        let task = create_test_task(AITaskType::ValidateCode);
        let contribution = create_test_contribution();

        let output = serde_json::json!({
            "passed": true,
            "stages": []
        });

        let result = TaskOrchestrator::on_task_complete(&task, &contribution, Some(&output));

        assert_eq!(result.next_tasks.len(), 1);
        assert_eq!(result.next_tasks[0].task_type, AITaskType::RunTests);
    }

    #[test]
    fn test_validation_failed() {
        let task = create_test_task(AITaskType::ValidateCode);
        let contribution = create_test_contribution();

        let output = serde_json::json!({
            "passed": false,
            "errors": [{"message": "type error"}]
        });

        let result = TaskOrchestrator::on_task_complete(&task, &contribution, Some(&output));

        assert_eq!(result.next_tasks.len(), 1);
        assert_eq!(result.next_tasks[0].task_type, AITaskType::GenerateCode);
        assert!(result.next_tasks[0].input.is_some());
    }

    #[test]
    fn test_tests_passed_auto_approve() {
        let task = create_test_task(AITaskType::RunTests);
        let mut contribution = create_test_contribution();
        contribution.risk_score = Some(0.2); // Low risk

        let output = serde_json::json!({
            "passed": true,
            "tests_run": 10,
            "tests_passed": 10
        });

        let result = TaskOrchestrator::on_task_complete(&task, &contribution, Some(&output));

        assert_eq!(result.next_tasks.len(), 1);
        assert_eq!(result.next_tasks[0].task_type, AITaskType::MergeChanges);
        assert_eq!(result.contribution_status, Some(ContributionStatus::Approved));
    }

    #[test]
    fn test_merge_complete() {
        let task = create_test_task(AITaskType::MergeChanges);
        let contribution = create_test_contribution();

        let result = TaskOrchestrator::on_task_complete(&task, &contribution, None);

        assert!(result.next_tasks.is_empty());
        assert_eq!(result.contribution_status, Some(ContributionStatus::Merged));
    }

    #[test]
    fn test_pipeline_stage_progression() {
        assert_eq!(PipelineStage::Submitted.next(), Some(PipelineStage::Analyzing));
        assert_eq!(PipelineStage::Analyzing.next(), Some(PipelineStage::Generating));
        assert_eq!(PipelineStage::Merged.next(), None);
        assert_eq!(PipelineStage::Rejected.next(), None);
    }
}
