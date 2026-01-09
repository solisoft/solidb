# AI Module

## Purpose
AI agent infrastructure for automated code contributions, task orchestration, and machine learning integration. Implements a multi-agent pipeline with learning capabilities.

## Key Files

| File | Lines | Description |
|------|-------|-------------|
| `learning.rs` | 1,513 | Feedback processing, pattern extraction, recommendations |
| `marketplace.rs` | 831 | Agent discovery, reputation scoring, rankings |
| `orchestrator.rs` | 582 | Task pipeline management, state transitions |
| `validation.rs` | 523 | Code validation (cargo check, clippy, tests) |
| `agent.rs` | 457 | Agent types, lifecycle, capabilities |
| `contribution.rs` | 302 | Contribution request handling |
| `task.rs` | 259 | AI task definitions and status |
| `recovery/` | dir | Error recovery, circuit breakers, health monitoring |

## Architecture

### Contribution Pipeline
```
Contribution → Analysis → Code Generation → Validation → Review → Merge
     ↓            ↓            ↓              ↓          ↓        ↓
  Submitted → Analyzing → Implementing → Validating → Review → Merged
```

### Agent Types (agent.rs)
```rust
pub enum AgentType {
    Analyzer,    // Analyzes contribution requests
    Coder,       // Generates code
    Tester,      // Creates and runs tests
    Reviewer,    // Reviews code quality
    Integrator,  // Merges approved changes
}
```

### Task Types (task.rs)
```rust
pub enum AITaskType {
    AnalyzeContribution,
    GenerateCode,
    ValidateCode,
    RunTests,
    PrepareReview,
    MergeChanges,
}
```

### Collections Used
- `_ai_contributions` - Contribution requests
- `_ai_tasks` - Task queue
- `_ai_agents` - Registered agents
- `_ai_feedback` - Learning feedback events
- `_ai_patterns` - Learned patterns

## Orchestrator (orchestrator.rs)

Manages task state transitions:
```rust
pub struct TaskOrchestrator;

impl TaskOrchestrator {
    // Called when a task completes
    pub fn on_task_complete(task, contribution, output) -> OrchestrationResult {
        // Returns next tasks to create and contribution status updates
    }

    // Called when contribution is approved
    pub fn on_approval(contribution, priority) -> OrchestrationResult {
        // Creates merge task
    }
}
```

## Learning System (learning.rs)

Feedback-driven pattern learning:
```rust
pub struct LearningSystem;

impl LearningSystem {
    // Record feedback events
    pub fn record_feedback(event: FeedbackEvent);

    // Process feedback to extract patterns
    pub fn process_feedback_batch(limit: usize) -> ProcessingResult;

    // Get recommendations for a task
    pub fn get_recommendations(task, status) -> Vec<Recommendation>;
}
```

Pattern types:
- `SuccessPattern` - What worked well
- `AntiPattern` - What to avoid
- `ErrorPattern` - Common errors
- `EscalationPattern` - When to escalate

## Marketplace (marketplace.rs)

Agent discovery and reputation:
```rust
pub struct AgentMarketplace;

impl AgentMarketplace {
    pub fn discover_agents(query: AgentDiscoveryQuery) -> Vec<RankedAgent>;
    pub fn select_agent_for_task(task: &AITask) -> Option<RankedAgent>;
    pub fn get_reputation(agent_id: &str) -> AgentReputation;
    pub fn get_rankings(limit: Option<usize>) -> Vec<RankedAgent>;
}
```

Reputation factors:
- Success rate
- Task completion time
- Specialization affinity
- Recent performance

## Validation (validation.rs)

Code quality checks:
```rust
pub struct ValidationPipeline;

impl ValidationPipeline {
    pub fn run() -> ValidationResult;      // Full validation
    pub fn run_quick() -> ValidationResult; // Skip tests
}
```

Stages:
1. `cargo check` - Type checking
2. `cargo clippy` - Linting
3. `cargo fmt --check` - Formatting
4. `cargo test` - Unit tests (optional)

## Recovery System (recovery/)

Fault tolerance:
- Circuit breakers for failing agents
- Task retry with backoff
- Stalled task detection
- Agent health monitoring

## Common Tasks

### Adding a New Agent Type
1. Add variant to `AgentType` enum in `agent.rs`
2. Update `TaskOrchestrator` for new task routing
3. Add capability matching in `marketplace.rs`

### Adding a New Task Type
1. Add variant to `AITaskType` in `task.rs`
2. Update `orchestrator.rs` for state transitions
3. Add handler logic in `server/ai_handlers.rs`

### Debugging AI Pipeline
1. Check `_ai_contributions` for contribution status
2. Check `_ai_tasks` for pending/failed tasks
3. Check `_ai_feedback` for error patterns

## Dependencies
- **Uses**: `storage` for persistence, `scripting` for managed agents
- **Used by**: `server::ai_handlers` for API endpoints

## Gotchas
- Tasks must be claimed before processing (prevents double-processing)
- Contributions go through strict state machine transitions
- Learning patterns need minimum sample size for confidence
- Circuit breakers trip after 5 consecutive failures (configurable)
- Managed agents store LLM credentials in `_env` collection
