//! AI-augmented database module
//!
//! This module provides the foundation for AI agent contributions to SoliDB.
//! It enables natural language contribution requests that are processed by
//! AI agents and validated through a multi-stage pipeline.

pub mod agent;
pub mod contribution;
pub mod learning;
pub mod marketplace;
pub mod orchestrator;
pub mod recovery;
pub mod task;
pub mod validation;

pub use contribution::{
    Contribution, ContributionContext, ContributionStatus, ContributionType,
    ListContributionsResponse, Priority, ReviewContributionRequest, SubmitContributionRequest,
    SubmitContributionResponse,
};

pub use task::{AITask, AITaskStatus, AITaskType, ListAITasksResponse};

pub use agent::{
    Agent, AgentStatus, AgentType, AnalysisResult, CodeGenerationResult, GeneratedFile,
    ListAgentsResponse, ValidationMessage, ValidationResult, ValidationStage,
    ValidationStageResult,
};

pub use validation::{ValidationConfig, ValidationPipeline};

pub use orchestrator::{
    OrchestrationResult, OrchestrationResultWithAgents, PipelineStage, TaskOrchestrator,
    TaskWithRecommendedAgent,
};

pub use marketplace::{
    AgentDiscoveryQuery, AgentDiscoveryResponse, AgentMarketplace, AgentRanking,
    AgentRankingsResponse, AgentReputation, RankedAgent, RecentPerformance, ScoreBreakdown,
    TaskTypeStats, VerifiedCapability, VerificationMethod,
};

pub use learning::{
    ActionType, FeedbackContext, FeedbackEvent, FeedbackOutcome, FeedbackQuery, FeedbackType,
    LearningSystem, ListFeedbackResponse, ListPatternsResponse, Pattern, PatternMatch,
    PatternQuery, PatternSignature, PatternType, ProcessingResult, Recommendation,
    SuggestedAction, TestOutput,
};

pub use recovery::{
    AgentHealthMetrics, CircuitState, ListRecoveryEventsResponse, RecoveryActionType,
    RecoveryConfig, RecoveryCycleStats, RecoveryEvent, RecoveryEventQuery, RecoverySeverity,
    RecoverySystemStatus, RecoveryWorker,
};
