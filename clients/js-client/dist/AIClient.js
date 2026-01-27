"use strict";
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
Object.defineProperty(exports, "__esModule", { value: true });
exports.RecoveryClient = exports.LearningClient = exports.MarketplaceClient = exports.AgentsClient = exports.TasksClient = exports.ContributionsClient = exports.AIClient = exports.AIClientError = void 0;
exports.createWorker = createWorker;
// =============================================================================
// ERROR CLASS
// =============================================================================
class AIClientError extends Error {
    constructor(message, statusCode, responseBody) {
        super(message);
        this.statusCode = statusCode;
        this.responseBody = responseBody;
        this.name = 'AIClientError';
    }
}
exports.AIClientError = AIClientError;
// =============================================================================
// MAIN CLIENT
// =============================================================================
class AIClient {
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
    constructor(baseUrl, database, apiKey) {
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
    _apiUrl(path) {
        return `${this.baseUrl}/_api/database/${this.database}${path}`;
    }
    /** @internal */
    async _request(method, path, options = {}) {
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
        const fetchOptions = {
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
            }
            catch { }
            throw new AIClientError(`API error (${response.status}): ${errorMsg}`, response.status);
        }
        if (response.status === 204) {
            return null;
        }
        return response.json();
    }
    /** @internal */
    async _get(path, params) {
        return this._request('GET', path, { params });
    }
    /** @internal */
    async _post(path, body) {
        return this._request('POST', path, { body });
    }
    /** @internal */
    async _delete(path) {
        return this._request('DELETE', path);
    }
}
exports.AIClient = AIClient;
// =============================================================================
// CONTRIBUTIONS CLIENT
// =============================================================================
class ContributionsClient {
    constructor(client) {
        this.client = client;
    }
    /**
     * Submit a new contribution request.
     */
    async submit(options) {
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
    async list(options = {}) {
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
    async get(contributionId) {
        return this.client._get(`/ai/contributions/${contributionId}`);
    }
    /**
     * Approve a contribution.
     */
    async approve(contributionId, feedback) {
        return this.client._post(`/ai/contributions/${contributionId}/approve`, { feedback });
    }
    /**
     * Reject a contribution.
     */
    async reject(contributionId, reason) {
        return this.client._post(`/ai/contributions/${contributionId}/reject`, { reason });
    }
}
exports.ContributionsClient = ContributionsClient;
// =============================================================================
// TASKS CLIENT
// =============================================================================
class TasksClient {
    constructor(client) {
        this.client = client;
    }
    /**
     * List tasks with optional filters.
     */
    async list(options = {}) {
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
    async get(taskId) {
        return this.client._get(`/ai/tasks/${taskId}`);
    }
    /**
     * Claim a task for processing.
     */
    async claim(taskId, agentId) {
        return this.client._post(`/ai/tasks/${taskId}/claim`, { agent_id: agentId });
    }
    /**
     * Mark a task as completed with output.
     */
    async complete(taskId, output) {
        return this.client._post(`/ai/tasks/${taskId}/complete`, { output });
    }
    /**
     * Mark a task as failed.
     */
    async fail(taskId, error) {
        return this.client._post(`/ai/tasks/${taskId}/fail`, { error });
    }
    /**
     * Release a claimed task back to pending state.
     */
    async release(taskId) {
        return this.client._post(`/ai/tasks/${taskId}/release`, {});
    }
}
exports.TasksClient = TasksClient;
// =============================================================================
// AGENTS CLIENT
// =============================================================================
class AgentsClient {
    constructor(client) {
        this.client = client;
    }
    /**
     * Register a new agent.
     */
    async register(options) {
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
    async list(options = {}) {
        return this.client._get('/ai/agents', {
            status: options.status,
            agent_type: options.agentType
        });
    }
    /**
     * Get a specific agent by ID.
     */
    async get(agentId) {
        return this.client._get(`/ai/agents/${agentId}`);
    }
    /**
     * Send a heartbeat for an agent.
     */
    async heartbeat(agentId) {
        await this.client._post(`/ai/agents/${agentId}/heartbeat`, {});
    }
    /**
     * Update an agent's status.
     */
    async updateStatus(agentId, status) {
        return this.client._post(`/ai/agents/${agentId}/status`, { status });
    }
    /**
     * Unregister/delete an agent.
     */
    async delete(agentId) {
        await this.client._delete(`/ai/agents/${agentId}`);
    }
}
exports.AgentsClient = AgentsClient;
// =============================================================================
// MARKETPLACE CLIENT
// =============================================================================
class MarketplaceClient {
    constructor(client) {
        this.client = client;
    }
    /**
     * Discover agents matching criteria, ranked by suitability.
     */
    async discover(options = {}) {
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
    async getReputation(agentId) {
        return this.client._get(`/ai/marketplace/agent/${agentId}/reputation`);
    }
    /**
     * Select the best agent for a specific task.
     */
    async selectForTask(taskId) {
        return this.client._post('/ai/marketplace/select', { task_id: taskId });
    }
    /**
     * Get agent rankings/leaderboard.
     */
    async getRankings(limit = 10) {
        return this.client._get('/ai/marketplace/rankings', { limit });
    }
}
exports.MarketplaceClient = MarketplaceClient;
// =============================================================================
// LEARNING CLIENT
// =============================================================================
class LearningClient {
    constructor(client) {
        this.client = client;
    }
    /**
     * List feedback events.
     */
    async listFeedback(options = {}) {
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
    async listPatterns(options = {}) {
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
    async processBatch(limit = 100) {
        return this.client._post('/ai/learning/process', { limit });
    }
    /**
     * Get recommendations for a task based on learned patterns.
     */
    async getRecommendations(taskId) {
        return this.client._get('/ai/learning/recommendations', { task_id: taskId });
    }
}
exports.LearningClient = LearningClient;
// =============================================================================
// RECOVERY CLIENT
// =============================================================================
class RecoveryClient {
    constructor(client) {
        this.client = client;
    }
    /**
     * Get recovery system status.
     */
    async getStatus() {
        return this.client._get('/ai/recovery/status');
    }
    /**
     * Force retry a stalled or failed task.
     */
    async retryTask(taskId) {
        return this.client._post(`/ai/recovery/task/${taskId}/retry`, {});
    }
    /**
     * Reset an agent's circuit breaker to closed state.
     */
    async resetCircuitBreaker(agentId) {
        await this.client._post(`/ai/recovery/agent/${agentId}/reset`, {});
    }
    /**
     * List recovery events.
     */
    async listEvents(options = {}) {
        return this.client._get('/ai/recovery/events', {
            action_type: options.actionType,
            severity: options.severity,
            entity_id: options.entityId,
            limit: options.limit ?? 50
        });
    }
}
exports.RecoveryClient = RecoveryClient;
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
async function createWorker(options) {
    const client = new AIClient(options.baseUrl, options.database, options.apiKey);
    const agent = await client.agents.register({
        name: options.name,
        agentType: options.agentType,
        capabilities: options.capabilities,
        url: options.url
    });
    return [client, agent.id];
}
