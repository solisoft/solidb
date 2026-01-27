/**
 * SoliDB AI Client Module
 *
 * Provides a comprehensive interface to SoliDB's AI features including:
 * - Contributions: Submit and manage AI contributions
 * - Tasks: Claim and process AI tasks
 * - Agents: Register and manage AI agents
 * - Marketplace: Discover and rank agents by trust scores
 * - Learning: Access feedback and learned patterns
 * - Recovery: Monitor system health and recovery events
 */
export type ContributionType = 'feature' | 'bugfix' | 'enhancement' | 'documentation';
export type AgentType = 'analyzer' | 'coder' | 'tester' | 'reviewer' | 'integrator';
export type TaskType = 'analyze_contribution' | 'generate_code' | 'validate_code' | 'run_tests' | 'prepare_review' | 'merge_changes';
export type TaskStatus = 'pending' | 'running' | 'completed' | 'failed' | 'cancelled';
export type FeedbackType = 'human_review' | 'validation_failure' | 'test_failure' | 'task_escalation' | 'success';
export type PatternType = 'success_pattern' | 'anti_pattern' | 'error_pattern' | 'escalation_pattern';
export type CircuitState = 'closed' | 'open' | 'half_open';
export type FeedbackOutcome = 'positive' | 'negative' | 'neutral';
export interface Contribution {
    id: string;
    contribution_type: ContributionType;
    description: string;
    status: string;
    requester?: string;
    context?: Record<string, any>;
    created_at: string;
    updated_at: string;
}
export interface Task {
    id: string;
    contribution_id: string;
    task_type: TaskType;
    status: TaskStatus;
    priority: number;
    input?: Record<string, any>;
    output?: Record<string, any>;
    agent_id?: string;
    created_at: string;
    started_at?: string;
    completed_at?: string;
}
export interface Agent {
    id: string;
    name: string;
    agent_type: AgentType;
    status: string;
    url?: string;
    capabilities: string[];
    config?: Record<string, any>;
    registered_at: string;
    last_heartbeat?: string;
    tasks_completed: number;
    tasks_failed: number;
}
export interface RankedAgent {
    agent: Agent;
    reputation: AgentReputation;
    suitability_score: number;
    score_breakdown: ScoreBreakdown;
}
export interface AgentReputation {
    agent_id: string;
    trust_score: number;
    success_rates: Record<string, number>;
    avg_completion_times: Record<string, number>;
}
export interface ScoreBreakdown {
    trust_component: number;
    capability_match: number;
    availability_bonus: number;
    recency_factor: number;
}
export interface FeedbackEvent {
    id: string;
    feedback_type: FeedbackType;
    outcome: FeedbackOutcome;
    contribution_id: string;
    task_id?: string;
    agent_id?: string;
    processed: boolean;
    created_at: string;
}
export interface Pattern {
    id: string;
    pattern_type: PatternType;
    confidence: number;
    occurrence_count: number;
    created_at: string;
}
export interface RecoveryEvent {
    id: string;
    action_type: string;
    severity: string;
    entity_id?: string;
    message: string;
    created_at: string;
}
export interface RecoveryStatus {
    healthy: boolean;
    last_cycle: string;
    agents_monitored: number;
    tasks_recovered: number;
    open_circuits: number;
}
export declare class AIClientError extends Error {
    statusCode?: number | undefined;
    responseBody?: any | undefined;
    constructor(message: string, statusCode?: number | undefined, responseBody?: any | undefined);
}
export declare class AIClient {
    private baseUrl;
    private database;
    private apiKey;
    private headers;
    readonly contributions: ContributionsClient;
    readonly tasks: TasksClient;
    readonly agents: AgentsClient;
    readonly marketplace: MarketplaceClient;
    readonly learning: LearningClient;
    readonly recovery: RecoveryClient;
    /**
     * Create a new AI client instance.
     *
     * @param baseUrl - SoliDB server URL (e.g., "http://localhost:8080")
     * @param database - Database name to operate on
     * @param apiKey - API key for authentication
     *
     * @example
     * ```typescript
     * const ai = new AIClient("http://localhost:8080", "mydb", "your_api_key");
     *
     * // Submit a contribution
     * const contrib = await ai.contributions.submit({
     *   contributionType: "feature",
     *   description: "Add user authentication"
     * });
     *
     * // Register as an agent
     * const agent = await ai.agents.register({
     *   name: "MyWorker",
     *   agentType: "coder",
     *   capabilities: ["typescript", "rust"]
     * });
     * ```
     */
    constructor(baseUrl: string, database: string, apiKey: string);
    /** @internal */
    _apiUrl(path: string): string;
    /** @internal */
    _request<T = any>(method: string, path: string, options?: {
        params?: Record<string, any>;
        body?: any;
    }): Promise<T>;
    /** @internal */
    _get<T = any>(path: string, params?: Record<string, any>): Promise<T>;
    /** @internal */
    _post<T = any>(path: string, body?: any): Promise<T>;
    /** @internal */
    _delete<T = any>(path: string): Promise<T>;
}
export declare class ContributionsClient {
    private client;
    constructor(client: AIClient);
    /**
     * Submit a new contribution request.
     */
    submit(options: {
        contributionType: ContributionType;
        description: string;
        context?: Record<string, any>;
        requester?: string;
        priority?: 'low' | 'medium' | 'high' | 'critical';
    }): Promise<Contribution>;
    /**
     * List contributions with optional filters.
     */
    list(options?: {
        status?: string;
        type?: ContributionType;
        limit?: number;
        offset?: number;
    }): Promise<{
        contributions: Contribution[];
        total: number;
    }>;
    /**
     * Get a specific contribution by ID.
     */
    get(contributionId: string): Promise<Contribution>;
    /**
     * Approve a contribution.
     */
    approve(contributionId: string, feedback?: string): Promise<Contribution>;
    /**
     * Reject a contribution.
     */
    reject(contributionId: string, reason: string): Promise<Contribution>;
}
export declare class TasksClient {
    private client;
    constructor(client: AIClient);
    /**
     * List tasks with optional filters.
     */
    list(options?: {
        status?: TaskStatus;
        taskType?: TaskType;
        contributionId?: string;
        agentId?: string;
        limit?: number;
    }): Promise<{
        tasks: Task[];
        total: number;
    }>;
    /**
     * Get a specific task by ID.
     */
    get(taskId: string): Promise<Task>;
    /**
     * Claim a task for processing.
     */
    claim(taskId: string, agentId: string): Promise<Task>;
    /**
     * Mark a task as completed with output.
     */
    complete(taskId: string, output: Record<string, any>): Promise<Task>;
    /**
     * Mark a task as failed.
     */
    fail(taskId: string, error: string): Promise<Task>;
    /**
     * Release a claimed task back to pending state.
     */
    release(taskId: string): Promise<Task>;
}
export declare class AgentsClient {
    private client;
    constructor(client: AIClient);
    /**
     * Register a new agent.
     */
    register(options: {
        name: string;
        agentType: AgentType;
        capabilities?: string[];
        url?: string;
        config?: Record<string, any>;
    }): Promise<Agent>;
    /**
     * List registered agents.
     */
    list(options?: {
        status?: string;
        agentType?: AgentType;
    }): Promise<{
        agents: Agent[];
    }>;
    /**
     * Get a specific agent by ID.
     */
    get(agentId: string): Promise<Agent>;
    /**
     * Send a heartbeat for an agent.
     */
    heartbeat(agentId: string): Promise<void>;
    /**
     * Update an agent's status.
     */
    updateStatus(agentId: string, status: 'idle' | 'busy' | 'offline'): Promise<Agent>;
    /**
     * Unregister/delete an agent.
     */
    delete(agentId: string): Promise<void>;
}
export declare class MarketplaceClient {
    private client;
    constructor(client: AIClient);
    /**
     * Discover agents matching criteria, ranked by suitability.
     */
    discover(options?: {
        agentType?: AgentType;
        requiredCapabilities?: string[];
        minTrustScore?: number;
        taskType?: string;
        idleOnly?: boolean;
        limit?: number;
    }): Promise<{
        agents: RankedAgent[];
        total: number;
    }>;
    /**
     * Get an agent's reputation and trust metrics.
     */
    getReputation(agentId: string): Promise<AgentReputation>;
    /**
     * Select the best agent for a specific task.
     */
    selectForTask(taskId: string): Promise<RankedAgent | null>;
    /**
     * Get agent rankings/leaderboard.
     */
    getRankings(limit?: number): Promise<{
        rankings: any[];
        total: number;
    }>;
}
export declare class LearningClient {
    private client;
    constructor(client: AIClient);
    /**
     * List feedback events.
     */
    listFeedback(options?: {
        feedbackType?: FeedbackType;
        outcome?: FeedbackOutcome;
        contributionId?: string;
        agentId?: string;
        processed?: boolean;
        limit?: number;
    }): Promise<{
        feedback: FeedbackEvent[];
        total: number;
    }>;
    /**
     * List learned patterns.
     */
    listPatterns(options?: {
        patternType?: PatternType;
        minConfidence?: number;
        taskType?: TaskType;
        limit?: number;
    }): Promise<{
        patterns: Pattern[];
        total: number;
    }>;
    /**
     * Trigger batch processing of unprocessed feedback.
     */
    processBatch(limit?: number): Promise<{
        processed: number;
        patternsCreated: number;
    }>;
    /**
     * Get recommendations for a task based on learned patterns.
     */
    getRecommendations(taskId: string): Promise<{
        recommendations: any[];
    }>;
}
export declare class RecoveryClient {
    private client;
    constructor(client: AIClient);
    /**
     * Get recovery system status.
     */
    getStatus(): Promise<RecoveryStatus>;
    /**
     * Force retry a stalled or failed task.
     */
    retryTask(taskId: string): Promise<{
        success: boolean;
    }>;
    /**
     * Reset an agent's circuit breaker to closed state.
     */
    resetCircuitBreaker(agentId: string): Promise<void>;
    /**
     * List recovery events.
     */
    listEvents(options?: {
        actionType?: string;
        severity?: string;
        entityId?: string;
        limit?: number;
    }): Promise<{
        events: RecoveryEvent[];
    }>;
}
/**
 * Create an AI client and register as a worker agent.
 *
 * @returns Tuple of [AIClient, agentId]
 *
 * @example
 * ```typescript
 * const [client, agentId] = await createWorker({
 *   baseUrl: "http://localhost:8080",
 *   database: "default",
 *   apiKey: "my_key",
 *   name: "MyWorker",
 *   agentType: "coder",
 *   capabilities: ["typescript", "rust"]
 * });
 * ```
 */
export declare function createWorker(options: {
    baseUrl: string;
    database: string;
    apiKey: string;
    name: string;
    agentType: AgentType;
    capabilities: string[];
    url?: string;
}): Promise<[AIClient, string]>;
