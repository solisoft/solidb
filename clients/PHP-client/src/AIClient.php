<?php

namespace SoliDB;

use SoliDB\Exception\DriverException;

/**
 * AI Client for SoliDB AI features.
 * Uses HTTP REST API for AI operations (separate from wire protocol).
 */
class AIClient
{
    private string $baseUrl;
    private string $database;
    private string $apiKey;
    private int $timeout;

    public ContributionsClient $contributions;
    public TasksClient $tasks;
    public AgentsClient $agents;
    public MarketplaceClient $marketplace;
    public LearningClient $learning;
    public RecoveryClient $recovery;

    public function __construct(string $baseUrl, string $database, string $apiKey, int $timeout = 30)
    {
        $this->baseUrl = rtrim($baseUrl, '/');
        $this->database = $database;
        $this->apiKey = $apiKey;
        $this->timeout = $timeout;

        $this->contributions = new ContributionsClient($this);
        $this->tasks = new TasksClient($this);
        $this->agents = new AgentsClient($this);
        $this->marketplace = new MarketplaceClient($this);
        $this->learning = new LearningClient($this);
        $this->recovery = new RecoveryClient($this);
    }

    public function apiUrl(string $path): string
    {
        return "{$this->baseUrl}/_api/database/{$this->database}{$path}";
    }

    public function request(string $method, string $path, array $params = [], ?array $body = null): array
    {
        $url = $this->apiUrl($path);
        if (!empty($params)) {
            $url .= '?' . http_build_query($params);
        }

        $ch = curl_init($url);
        curl_setopt($ch, CURLOPT_CUSTOMREQUEST, $method);
        curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
        curl_setopt($ch, CURLOPT_TIMEOUT, $this->timeout);
        curl_setopt($ch, CURLOPT_HTTPHEADER, [
            'Authorization: Bearer ' . $this->apiKey,
            'Content-Type: application/json',
        ]);

        if ($body !== null) {
            curl_setopt($ch, CURLOPT_POSTFIELDS, json_encode($body));
        }

        $response = curl_exec($ch);
        $httpCode = curl_getinfo($ch, CURLINFO_HTTP_CODE);
        $error = curl_error($ch);
        curl_close($ch);

        if ($error) {
            throw new AIClientError("Request failed: $error");
        }

        $data = json_decode($response, true);

        if ($httpCode >= 400) {
            $msg = $data['error'] ?? $response;
            throw new AIClientError("API error ($httpCode): $msg");
        }

        return $data ?? [];
    }
}

class AIClientError extends \Exception {}

// Contribution types
class ContributionType
{
    public const FEATURE = 'feature';
    public const BUGFIX = 'bugfix';
    public const ENHANCEMENT = 'enhancement';
    public const DOCUMENTATION = 'documentation';
}

// Agent types
class AgentType
{
    public const ANALYZER = 'analyzer';
    public const CODER = 'coder';
    public const TESTER = 'tester';
    public const REVIEWER = 'reviewer';
    public const INTEGRATOR = 'integrator';
}

// Task types
class TaskType
{
    public const ANALYZE_CONTRIBUTION = 'analyze_contribution';
    public const GENERATE_CODE = 'generate_code';
    public const VALIDATE_CODE = 'validate_code';
    public const RUN_TESTS = 'run_tests';
    public const PREPARE_REVIEW = 'prepare_review';
    public const MERGE_CHANGES = 'merge_changes';
}

// Task statuses
class TaskStatus
{
    public const PENDING = 'pending';
    public const RUNNING = 'running';
    public const COMPLETED = 'completed';
    public const FAILED = 'failed';
    public const CANCELLED = 'cancelled';
}

// Feedback types
class FeedbackType
{
    public const HUMAN_REVIEW = 'human_review';
    public const VALIDATION_FAILURE = 'validation_failure';
    public const TEST_FAILURE = 'test_failure';
    public const TASK_ESCALATION = 'task_escalation';
}

// Pattern types
class PatternType
{
    public const SUCCESS = 'success';
    public const ANTI_PATTERN = 'anti_pattern';
    public const ERROR = 'error';
}

// Circuit breaker states
class CircuitState
{
    public const CLOSED = 'closed';
    public const OPEN = 'open';
    public const HALF_OPEN = 'half_open';
}

/**
 * Contributions client
 */
class ContributionsClient
{
    private AIClient $client;

    public function __construct(AIClient $client)
    {
        $this->client = $client;
    }

    public function submit(array $contribution): array
    {
        return $this->client->request('POST', '/ai/contributions', [], $contribution);
    }

    public function list(?string $status = null, ?string $type = null, int $limit = 50, int $offset = 0): array
    {
        $params = ['limit' => $limit, 'offset' => $offset];
        if ($status) $params['status'] = $status;
        if ($type) $params['type'] = $type;
        return $this->client->request('GET', '/ai/contributions', $params);
    }

    public function get(string $id): array
    {
        return $this->client->request('GET', "/ai/contributions/$id");
    }

    public function approve(string $id, ?string $feedback = null): array
    {
        $body = $feedback ? ['feedback' => $feedback] : [];
        return $this->client->request('POST', "/ai/contributions/$id/approve", [], $body);
    }

    public function reject(string $id, string $reason): array
    {
        return $this->client->request('POST', "/ai/contributions/$id/reject", [], ['reason' => $reason]);
    }
}

/**
 * Tasks client
 */
class TasksClient
{
    private AIClient $client;

    public function __construct(AIClient $client)
    {
        $this->client = $client;
    }

    public function list(
        ?string $status = null,
        ?string $taskType = null,
        ?string $contributionId = null,
        ?string $agentId = null,
        int $limit = 50
    ): array {
        $params = ['limit' => $limit];
        if ($status) $params['status'] = $status;
        if ($taskType) $params['task_type'] = $taskType;
        if ($contributionId) $params['contribution_id'] = $contributionId;
        if ($agentId) $params['agent_id'] = $agentId;
        return $this->client->request('GET', '/ai/tasks', $params);
    }

    public function get(string $id): array
    {
        return $this->client->request('GET', "/ai/tasks/$id");
    }

    public function claim(string $taskId, string $agentId): array
    {
        return $this->client->request('POST', "/ai/tasks/$taskId/claim", [], ['agent_id' => $agentId]);
    }

    public function complete(string $taskId, array $output): array
    {
        return $this->client->request('POST', "/ai/tasks/$taskId/complete", [], ['output' => $output]);
    }

    public function fail(string $taskId, string $error): array
    {
        return $this->client->request('POST', "/ai/tasks/$taskId/fail", [], ['error' => $error]);
    }
}

/**
 * Agents client
 */
class AgentsClient
{
    private AIClient $client;

    public function __construct(AIClient $client)
    {
        $this->client = $client;
    }

    public function register(array $agent): array
    {
        return $this->client->request('POST', '/ai/agents', [], $agent);
    }

    public function list(?string $status = null, ?string $agentType = null): array
    {
        $params = [];
        if ($status) $params['status'] = $status;
        if ($agentType) $params['agent_type'] = $agentType;
        return $this->client->request('GET', '/ai/agents', $params);
    }

    public function get(string $id): array
    {
        return $this->client->request('GET', "/ai/agents/$id");
    }

    public function heartbeat(string $id): void
    {
        $this->client->request('POST', "/ai/agents/$id/heartbeat", [], []);
    }

    public function delete(string $id): void
    {
        $this->client->request('DELETE', "/ai/agents/$id");
    }
}

/**
 * Marketplace client
 */
class MarketplaceClient
{
    private AIClient $client;

    public function __construct(AIClient $client)
    {
        $this->client = $client;
    }

    public function discover(?string $agentType = null, ?float $minTrustScore = null, int $limit = 10): array
    {
        $params = ['limit' => $limit];
        if ($agentType) $params['agent_type'] = $agentType;
        if ($minTrustScore !== null) $params['min_trust_score'] = $minTrustScore;
        return $this->client->request('GET', '/ai/marketplace/discover', $params);
    }

    public function getReputation(string $agentId): array
    {
        return $this->client->request('GET', "/ai/marketplace/agent/$agentId/reputation");
    }

    public function getRankings(int $limit = 10): array
    {
        return $this->client->request('GET', '/ai/marketplace/rankings', ['limit' => $limit]);
    }
}

/**
 * Learning client
 */
class LearningClient
{
    private AIClient $client;

    public function __construct(AIClient $client)
    {
        $this->client = $client;
    }

    public function listFeedback(?string $feedbackType = null, ?string $outcome = null, int $limit = 50): array
    {
        $params = ['limit' => $limit];
        if ($feedbackType) $params['feedback_type'] = $feedbackType;
        if ($outcome) $params['outcome'] = $outcome;
        return $this->client->request('GET', '/ai/learning/feedback', $params);
    }

    public function listPatterns(?string $patternType = null, ?float $minConfidence = null, int $limit = 50): array
    {
        $params = ['limit' => $limit];
        if ($patternType) $params['pattern_type'] = $patternType;
        if ($minConfidence !== null) $params['min_confidence'] = $minConfidence;
        return $this->client->request('GET', '/ai/learning/patterns', $params);
    }

    public function processBatch(int $limit = 100): array
    {
        return $this->client->request('POST', '/ai/learning/process', [], ['limit' => $limit]);
    }
}

/**
 * Recovery client
 */
class RecoveryClient
{
    private AIClient $client;

    public function __construct(AIClient $client)
    {
        $this->client = $client;
    }

    public function getStatus(): array
    {
        return $this->client->request('GET', '/ai/recovery/status');
    }

    public function retryTask(string $taskId): void
    {
        $this->client->request('POST', "/ai/recovery/task/$taskId/retry", [], []);
    }

    public function resetCircuitBreaker(string $agentId): void
    {
        $this->client->request('POST', "/ai/recovery/agent/$agentId/reset", [], []);
    }

    public function listEvents(?string $actionType = null, ?string $severity = null, int $limit = 50): array
    {
        $params = ['limit' => $limit];
        if ($actionType) $params['action_type'] = $actionType;
        if ($severity) $params['severity'] = $severity;
        return $this->client->request('GET', '/ai/recovery/events', $params);
    }
}

/**
 * Helper function to create a worker agent
 */
function createWorker(
    string $baseUrl,
    string $database,
    string $apiKey,
    string $name,
    string $agentType,
    array $capabilities = [],
    ?string $webhookUrl = null
): array {
    $client = new AIClient($baseUrl, $database, $apiKey);

    $agent = [
        'name' => $name,
        'agent_type' => $agentType,
        'capabilities' => $capabilities,
    ];
    if ($webhookUrl) {
        $agent['url'] = $webhookUrl;
    }

    $registered = $client->agents->register($agent);
    return [$client, $registered['id'] ?? $registered['_key']];
}
