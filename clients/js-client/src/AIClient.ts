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

// =============================================================================
// TYPES
// =============================================================================

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

// =============================================================================
// ERROR CLASS
// =============================================================================

export class AIClientError extends Error {
    constructor(
        message: string,
        public statusCode?: number,
        public responseBody?: any
    ) {
        super(message);
        this.name = 'AIClientError';
    }
}

// =============================================================================
// MAIN CLIENT
// =============================================================================

export class AIClient {
    private baseUrl: string;
    private database: string;
    private apiKey: string;
    private headers: Record<string, string>;

    public readonly contributions: ContributionsClient;
    public readonly tasks: TasksClient;
    public readonly agents: AgentsClient;
    public readonly marketplace: MarketplaceClient;
    public readonly learning: LearningClient;
    public readonly recovery: RecoveryClient;

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
    constructor(baseUrl: string, database: string, apiKey: string) {
        this.baseUrl = baseUrl.replace(/\/$/, '');
        this.database = database;
        this.apiKey = apiKey;
        this.headers = {
            'Authorization': `Bearer ${apiKey}`,
            'Content-Type': 'application/json'
        };

        // Initialize sub-clients
        this.contributions = new ContributionsClient(this);
        this.tasks = new TasksClient(this);
        this.agents = new AgentsClient(this);
        this.marketplace = new MarketplaceClient(this);
        this.learning = new LearningClient(this);
        this.recovery = new RecoveryClient(this);
    }

    /** @internal */
    _apiUrl(path: string): string {
        return `${this.baseUrl}/_api/database/${this.database}${path}`;
    }

    /** @internal */
    async _request<T = any>(method: string, path: string, options: {
        params?: Record<string, any>;
        body?: any;
    } = {}): Promise<T> {
        let url = this._apiUrl(path);

        if (options.params) {
            const searchParams = new URLSearchParams();
            for (const [key, value] of Object.entries(options.params)) {
                if (value !== undefined && value !== null) {
                    searchParams.append(key, String(value));
                }
            }
            const queryString = searchParams.toString();
            if (queryString) {
                url += `?${queryString}`;
            }
        }

        const fetchOptions: RequestInit = {
            method,
            headers: this.headers,
        };

        if (options.body) {
            fetchOptions.body = JSON.stringify(options.body);
        }

        const response = await fetch(url, fetchOptions);

        if (!response.ok) {
            let errorMsg = response.statusText;
            try {
                const errorData = await response.json();
                errorMsg = errorData.error || errorMsg;
            } catch { }
            throw new AIClientError(`API error (${response.status}): ${errorMsg}`, response.status);
        }

        if (response.status === 204) {
            return null as T;
        }

        return response.json();
    }

    /** @internal */
    async _get<T = any>(path: string, params?: Record<string, any>): Promise<T> {
        return this._request<T>('GET', path, { params });
    }

    /** @internal */
    async _post<T = any>(path: string, body?: any): Promise<T> {
        return this._request<T>('POST', path, { body });
    }

    /** @internal */
    async _delete<T = any>(path: string): Promise<T> {
        return this._request<T>('DELETE', path);
    }
}

// =============================================================================
// CONTRIBUTIONS CLIENT
// =============================================================================

export class ContributionsClient {
    constructor(private client: AIClient) { }

    /**
     * Submit a new contribution request.
     */
    async submit(options: {
        contributionType: ContributionType;
        description: string;
        context?: Record<string, any>;
        requester?: string;
        priority?: 'low' | 'medium' | 'high' | 'critical';
    }): Promise<Contribution> {
        return this.client._post('/ai/contributions', {
            contribution_type: options.contributionType,
            description: options.description,
            context: options.context,
            requester: options.requester,
            priority: options.priority || 'medium'
        });
    }

    /**
     * List contributions with optional filters.
     */
    async list(options: {
        status?: string;
        type?: ContributionType;
        limit?: number;
        offset?: number;
    } = {}): Promise<{ contributions: Contribution[]; total: number }> {
        return this.client._get('/ai/contributions', {
            status: options.status,
            type: options.type,
            limit: options.limit ?? 50,
            offset: options.offset ?? 0
        });
    }

    /**
     * Get a specific contribution by ID.
     */
    async get(contributionId: string): Promise<Contribution> {
        return this.client._get(`/ai/contributions/${contributionId}`);
    }

    /**
     * Approve a contribution.
     */
    async approve(contributionId: string, feedback?: string): Promise<Contribution> {
        return this.client._post(`/ai/contributions/${contributionId}/approve`, { feedback });
    }

    /**
     * Reject a contribution.
     */
    async reject(contributionId: string, reason: string): Promise<Contribution> {
        return this.client._post(`/ai/contributions/${contributionId}/reject`, { reason });
    }
}

// =============================================================================
// TASKS CLIENT
// =============================================================================

export class TasksClient {
    constructor(private client: AIClient) { }

    /**
     * List tasks with optional filters.
     */
    async list(options: {
        status?: TaskStatus;
        taskType?: TaskType;
        contributionId?: string;
        agentId?: string;
        limit?: number;
    } = {}): Promise<{ tasks: Task[]; total: number }> {
        return this.client._get('/ai/tasks', {
            status: options.status,
            task_type: options.taskType,
            contribution_id: options.contributionId,
            agent_id: options.agentId,
            limit: options.limit ?? 50
        });
    }

    /**
     * Get a specific task by ID.
     */
    async get(taskId: string): Promise<Task> {
        return this.client._get(`/ai/tasks/${taskId}`);
    }

    /**
     * Claim a task for processing.
     */
    async claim(taskId: string, agentId: string): Promise<Task> {
        return this.client._post(`/ai/tasks/${taskId}/claim`, { agent_id: agentId });
    }

    /**
     * Mark a task as completed with output.
     */
    async complete(taskId: string, output: Record<string, any>): Promise<Task> {
        return this.client._post(`/ai/tasks/${taskId}/complete`, { output });
    }

    /**
     * Mark a task as failed.
     */
    async fail(taskId: string, error: string): Promise<Task> {
        return this.client._post(`/ai/tasks/${taskId}/fail`, { error });
    }

    /**
     * Release a claimed task back to pending state.
     */
    async release(taskId: string): Promise<Task> {
        return this.client._post(`/ai/tasks/${taskId}/release`, {});
    }
}

// =============================================================================
// AGENTS CLIENT
// =============================================================================

export class AgentsClient {
    constructor(private client: AIClient) { }

    /**
     * Register a new agent.
     */
    async register(options: {
        name: string;
        agentType: AgentType;
        capabilities?: string[];
        url?: string;
        config?: Record<string, any>;
    }): Promise<Agent> {
        return this.client._post('/ai/agents', {
            name: options.name,
            agent_type: options.agentType,
            capabilities: options.capabilities || [],
            url: options.url,
            config: options.config
        });
    }

    /**
     * List registered agents.
     */
    async list(options: {
        status?: string;
        agentType?: AgentType;
    } = {}): Promise<{ agents: Agent[] }> {
        return this.client._get('/ai/agents', {
            status: options.status,
            agent_type: options.agentType
        });
    }

    /**
     * Get a specific agent by ID.
     */
    async get(agentId: string): Promise<Agent> {
        return this.client._get(`/ai/agents/${agentId}`);
    }

    /**
     * Send a heartbeat for an agent.
     */
    async heartbeat(agentId: string): Promise<void> {
        await this.client._post(`/ai/agents/${agentId}/heartbeat`, {});
    }

    /**
     * Update an agent's status.
     */
    async updateStatus(agentId: string, status: 'idle' | 'busy' | 'offline'): Promise<Agent> {
        return this.client._post(`/ai/agents/${agentId}/status`, { status });
    }

    /**
     * Unregister/delete an agent.
     */
    async delete(agentId: string): Promise<void> {
        await this.client._delete(`/ai/agents/${agentId}`);
    }
}

// =============================================================================
// MARKETPLACE CLIENT
// =============================================================================

export class MarketplaceClient {
    constructor(private client: AIClient) { }

    /**
     * Discover agents matching criteria, ranked by suitability.
     */
    async discover(options: {
        agentType?: AgentType;
        requiredCapabilities?: string[];
        minTrustScore?: number;
        taskType?: string;
        idleOnly?: boolean;
        limit?: number;
    } = {}): Promise<{ agents: RankedAgent[]; total: number }> {
        return this.client._get('/ai/marketplace/discover', {
            agent_type: options.agentType,
            required_capabilities: options.requiredCapabilities?.join(','),
            min_trust_score: options.minTrustScore,
            task_type: options.taskType,
            idle_only: options.idleOnly,
            limit: options.limit ?? 10
        });
    }

    /**
     * Get an agent's reputation and trust metrics.
     */
    async getReputation(agentId: string): Promise<AgentReputation> {
        return this.client._get(`/ai/marketplace/agent/${agentId}/reputation`);
    }

    /**
     * Select the best agent for a specific task.
     */
    async selectForTask(taskId: string): Promise<RankedAgent | null> {
        return this.client._post('/ai/marketplace/select', { task_id: taskId });
    }

    /**
     * Get agent rankings/leaderboard.
     */
    async getRankings(limit: number = 10): Promise<{ rankings: any[]; total: number }> {
        return this.client._get('/ai/marketplace/rankings', { limit });
    }
}

// =============================================================================
// LEARNING CLIENT
// =============================================================================

export class LearningClient {
    constructor(private client: AIClient) { }

    /**
     * List feedback events.
     */
    async listFeedback(options: {
        feedbackType?: FeedbackType;
        outcome?: FeedbackOutcome;
        contributionId?: string;
        agentId?: string;
        processed?: boolean;
        limit?: number;
    } = {}): Promise<{ feedback: FeedbackEvent[]; total: number }> {
        return this.client._get('/ai/learning/feedback', {
            feedback_type: options.feedbackType,
            outcome: options.outcome,
            contribution_id: options.contributionId,
            agent_id: options.agentId,
            processed: options.processed,
            limit: options.limit ?? 50
        });
    }

    /**
     * List learned patterns.
     */
    async listPatterns(options: {
        patternType?: PatternType;
        minConfidence?: number;
        taskType?: TaskType;
        limit?: number;
    } = {}): Promise<{ patterns: Pattern[]; total: number }> {
        return this.client._get('/ai/learning/patterns', {
            pattern_type: options.patternType,
            min_confidence: options.minConfidence,
            task_type: options.taskType,
            limit: options.limit ?? 50
        });
    }

    /**
     * Trigger batch processing of unprocessed feedback.
     */
    async processBatch(limit: number = 100): Promise<{ processed: number; patternsCreated: number }> {
        return this.client._post('/ai/learning/process', { limit });
    }

    /**
     * Get recommendations for a task based on learned patterns.
     */
    async getRecommendations(taskId: string): Promise<{ recommendations: any[] }> {
        return this.client._get('/ai/learning/recommendations', { task_id: taskId });
    }
}

// =============================================================================
// RECOVERY CLIENT
// =============================================================================

export class RecoveryClient {
    constructor(private client: AIClient) { }

    /**
     * Get recovery system status.
     */
    async getStatus(): Promise<RecoveryStatus> {
        return this.client._get('/ai/recovery/status');
    }

    /**
     * Force retry a stalled or failed task.
     */
    async retryTask(taskId: string): Promise<{ success: boolean }> {
        return this.client._post(`/ai/recovery/task/${taskId}/retry`, {});
    }

    /**
     * Reset an agent's circuit breaker to closed state.
     */
    async resetCircuitBreaker(agentId: string): Promise<void> {
        await this.client._post(`/ai/recovery/agent/${agentId}/reset`, {});
    }

    /**
     * List recovery events.
     */
    async listEvents(options: {
        actionType?: string;
        severity?: string;
        entityId?: string;
        limit?: number;
    } = {}): Promise<{ events: RecoveryEvent[] }> {
        return this.client._get('/ai/recovery/events', {
            action_type: options.actionType,
            severity: options.severity,
            entity_id: options.entityId,
            limit: options.limit ?? 50
        });
    }
}

// =============================================================================
// CONVENIENCE FUNCTIONS
// =============================================================================

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
export async function createWorker(options: {
    baseUrl: string;
    database: string;
    apiKey: string;
    name: string;
    agentType: AgentType;
    capabilities: string[];
    url?: string;
}): Promise<[AIClient, string]> {
    const client = new AIClient(options.baseUrl, options.database, options.apiKey);
    const agent = await client.agents.register({
        name: options.name,
        agentType: options.agentType,
        capabilities: options.capabilities,
        url: options.url
    });
    return [client, agent.id];
}
