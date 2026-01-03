"""
SoliDB AI Client Module

Provides a comprehensive interface to SoliDB's AI features including:
- Contributions: Submit and manage AI contributions
- Tasks: Claim and process AI tasks
- Agents: Register and manage AI agents
- Marketplace: Discover and rank agents by trust scores
- Learning: Access feedback and learned patterns
- Recovery: Monitor system health and recovery events
"""

import requests
from typing import Optional, List, Dict, Any
from dataclasses import dataclass
from enum import Enum


class ContributionType(Enum):
    FEATURE = "feature"
    BUGFIX = "bugfix"
    ENHANCEMENT = "enhancement"
    DOCUMENTATION = "documentation"


class AgentType(Enum):
    ANALYZER = "analyzer"
    CODER = "coder"
    TESTER = "tester"
    REVIEWER = "reviewer"
    INTEGRATOR = "integrator"


class TaskType(Enum):
    ANALYZE_CONTRIBUTION = "analyze_contribution"
    GENERATE_CODE = "generate_code"
    VALIDATE_CODE = "validate_code"
    RUN_TESTS = "run_tests"
    PREPARE_REVIEW = "prepare_review"
    MERGE_CHANGES = "merge_changes"


class TaskStatus(Enum):
    PENDING = "pending"
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"


class FeedbackType(Enum):
    HUMAN_REVIEW = "human_review"
    VALIDATION_FAILURE = "validation_failure"
    TEST_FAILURE = "test_failure"
    TASK_ESCALATION = "task_escalation"
    SUCCESS = "success"


class PatternType(Enum):
    SUCCESS_PATTERN = "success_pattern"
    ANTI_PATTERN = "anti_pattern"
    ERROR_PATTERN = "error_pattern"
    ESCALATION_PATTERN = "escalation_pattern"


class CircuitState(Enum):
    CLOSED = "closed"
    OPEN = "open"
    HALF_OPEN = "half_open"


class AIClient:
    """
    AI Client for SoliDB's AI-augmented database features.

    Example usage:
        from solidb.ai import AIClient

        ai = AIClient("http://localhost:8080", "mydb", "your_api_key")

        # Submit a contribution
        contrib = ai.contributions.submit(
            contribution_type="feature",
            description="Add user authentication",
            context={"priority": "high"}
        )

        # Register as an agent
        agent = ai.agents.register(
            name="MyWorker",
            agent_type="coder",
            capabilities=["python", "rust"]
        )

        # Poll for tasks
        tasks = ai.tasks.list(status="pending")
    """

    def __init__(self, base_url: str, database: str, api_key: str):
        """
        Initialize the AI client.

        Args:
            base_url: SoliDB server URL (e.g., "http://localhost:8080")
            database: Database name to operate on
            api_key: API key for authentication
        """
        self.base_url = base_url.rstrip('/')
        self.database = database
        self.api_key = api_key
        self._headers = {
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json"
        }

        # Initialize sub-clients
        self.contributions = ContributionsClient(self)
        self.tasks = TasksClient(self)
        self.agents = AgentsClient(self)
        self.marketplace = MarketplaceClient(self)
        self.learning = LearningClient(self)
        self.recovery = RecoveryClient(self)

    def _api_url(self, path: str) -> str:
        """Build full API URL."""
        return f"{self.base_url}/_api/database/{self.database}{path}"

    def _request(self, method: str, path: str, **kwargs) -> Any:
        """Make an authenticated request."""
        url = self._api_url(path)
        response = requests.request(method, url, headers=self._headers, **kwargs)

        if response.status_code >= 400:
            error_msg = response.text
            try:
                error_data = response.json()
                error_msg = error_data.get('error', error_msg)
            except:
                pass
            raise AIClientError(f"API error ({response.status_code}): {error_msg}")

        if response.status_code == 204:
            return None

        return response.json()

    def _get(self, path: str, params: Optional[Dict] = None) -> Any:
        return self._request("GET", path, params=params)

    def _post(self, path: str, data: Optional[Dict] = None) -> Any:
        return self._request("POST", path, json=data)

    def _delete(self, path: str) -> Any:
        return self._request("DELETE", path)


class AIClientError(Exception):
    """Exception raised for AI client errors."""
    pass


# =============================================================================
# CONTRIBUTIONS
# =============================================================================

class ContributionsClient:
    """Client for AI contributions API."""

    def __init__(self, client: AIClient):
        self._client = client

    def submit(
        self,
        contribution_type: str,
        description: str,
        context: Optional[Dict] = None,
        requester: Optional[str] = None,
        priority: str = "medium"
    ) -> Dict:
        """
        Submit a new contribution request.

        Args:
            contribution_type: One of "feature", "bugfix", "enhancement", "documentation"
            description: Natural language description of the contribution
            context: Optional context (related_collections, etc.)
            requester: Optional requester identifier
            priority: Priority level ("low", "medium", "high", "critical")

        Returns:
            The created contribution object
        """
        payload = {
            "contribution_type": contribution_type,
            "description": description,
            "priority": priority
        }
        if context:
            payload["context"] = context
        if requester:
            payload["requester"] = requester

        return self._client._post("/ai/contributions", payload)

    def list(
        self,
        status: Optional[str] = None,
        contribution_type: Optional[str] = None,
        limit: int = 50,
        offset: int = 0
    ) -> Dict:
        """
        List contributions with optional filters.

        Returns:
            Dict with 'contributions' list and 'total' count
        """
        params = {"limit": limit, "offset": offset}
        if status:
            params["status"] = status
        if contribution_type:
            params["type"] = contribution_type

        return self._client._get("/ai/contributions", params)

    def get(self, contribution_id: str) -> Dict:
        """Get a specific contribution by ID."""
        return self._client._get(f"/ai/contributions/{contribution_id}")

    def approve(self, contribution_id: str, feedback: Optional[str] = None) -> Dict:
        """
        Approve a contribution.

        Args:
            contribution_id: The contribution ID
            feedback: Optional approval feedback/comments
        """
        payload = {}
        if feedback:
            payload["feedback"] = feedback
        return self._client._post(f"/ai/contributions/{contribution_id}/approve", payload)

    def reject(self, contribution_id: str, reason: str) -> Dict:
        """
        Reject a contribution.

        Args:
            contribution_id: The contribution ID
            reason: Reason for rejection
        """
        return self._client._post(f"/ai/contributions/{contribution_id}/reject", {"reason": reason})


# =============================================================================
# TASKS
# =============================================================================

class TasksClient:
    """Client for AI tasks API."""

    def __init__(self, client: AIClient):
        self._client = client

    def list(
        self,
        status: Optional[str] = None,
        task_type: Optional[str] = None,
        contribution_id: Optional[str] = None,
        agent_id: Optional[str] = None,
        limit: int = 50
    ) -> Dict:
        """
        List tasks with optional filters.

        Args:
            status: Filter by status (pending, running, completed, failed)
            task_type: Filter by task type
            contribution_id: Filter by contribution
            agent_id: Filter by assigned agent
            limit: Maximum results

        Returns:
            Dict with 'tasks' list and 'total' count
        """
        params = {"limit": limit}
        if status:
            params["status"] = status
        if task_type:
            params["task_type"] = task_type
        if contribution_id:
            params["contribution_id"] = contribution_id
        if agent_id:
            params["agent_id"] = agent_id

        return self._client._get("/ai/tasks", params)

    def get(self, task_id: str) -> Dict:
        """Get a specific task by ID."""
        return self._client._get(f"/ai/tasks/{task_id}")

    def claim(self, task_id: str, agent_id: str) -> Dict:
        """
        Claim a task for processing.

        Args:
            task_id: The task to claim
            agent_id: The agent claiming the task
        """
        return self._client._post(f"/ai/tasks/{task_id}/claim", {"agent_id": agent_id})

    def complete(self, task_id: str, output: Dict) -> Dict:
        """
        Mark a task as completed with output.

        Args:
            task_id: The task ID
            output: The task output/result
        """
        return self._client._post(f"/ai/tasks/{task_id}/complete", {"output": output})

    def fail(self, task_id: str, error: str) -> Dict:
        """
        Mark a task as failed.

        Args:
            task_id: The task ID
            error: Error message describing the failure
        """
        return self._client._post(f"/ai/tasks/{task_id}/fail", {"error": error})

    def release(self, task_id: str) -> Dict:
        """Release a claimed task back to pending state."""
        return self._client._post(f"/ai/tasks/{task_id}/release", {})


# =============================================================================
# AGENTS
# =============================================================================

class AgentsClient:
    """Client for AI agents API."""

    def __init__(self, client: AIClient):
        self._client = client

    def register(
        self,
        name: str,
        agent_type: str,
        capabilities: Optional[List[str]] = None,
        url: Optional[str] = None,
        config: Optional[Dict] = None
    ) -> Dict:
        """
        Register a new agent.

        Args:
            name: Human-readable agent name
            agent_type: One of "analyzer", "coder", "tester", "reviewer", "integrator"
            capabilities: List of capabilities (e.g., ["python", "rust", "code-review"])
            url: Optional webhook URL for task notifications
            config: Optional configuration dict

        Returns:
            The created agent object with ID
        """
        payload = {
            "name": name,
            "agent_type": agent_type,
            "capabilities": capabilities or []
        }
        if url:
            payload["url"] = url
        if config:
            payload["config"] = config

        return self._client._post("/ai/agents", payload)

    def list(self, status: Optional[str] = None, agent_type: Optional[str] = None) -> Dict:
        """
        List registered agents.

        Args:
            status: Filter by status (idle, busy, offline, error)
            agent_type: Filter by agent type

        Returns:
            Dict with 'agents' list
        """
        params = {}
        if status:
            params["status"] = status
        if agent_type:
            params["agent_type"] = agent_type

        return self._client._get("/ai/agents", params)

    def get(self, agent_id: str) -> Dict:
        """Get a specific agent by ID."""
        return self._client._get(f"/ai/agents/{agent_id}")

    def heartbeat(self, agent_id: str) -> Dict:
        """
        Send a heartbeat for an agent.

        Should be called periodically (every 30-60 seconds) to indicate
        the agent is still alive.
        """
        return self._client._post(f"/ai/agents/{agent_id}/heartbeat", {})

    def update_status(self, agent_id: str, status: str) -> Dict:
        """
        Update an agent's status.

        Args:
            agent_id: The agent ID
            status: New status (idle, busy, offline)
        """
        return self._client._post(f"/ai/agents/{agent_id}/status", {"status": status})

    def delete(self, agent_id: str) -> None:
        """Unregister/delete an agent."""
        self._client._delete(f"/ai/agents/{agent_id}")


# =============================================================================
# MARKETPLACE
# =============================================================================

class MarketplaceClient:
    """Client for agent marketplace API."""

    def __init__(self, client: AIClient):
        self._client = client

    def discover(
        self,
        agent_type: Optional[str] = None,
        required_capabilities: Optional[List[str]] = None,
        min_trust_score: Optional[float] = None,
        task_type: Optional[str] = None,
        idle_only: bool = False,
        limit: int = 10
    ) -> Dict:
        """
        Discover agents matching criteria, ranked by suitability.

        Args:
            agent_type: Filter by agent type
            required_capabilities: Required capabilities (AND logic)
            min_trust_score: Minimum trust score (0.0-1.0)
            task_type: Task type for specialized ranking
            idle_only: Only return idle agents
            limit: Maximum results

        Returns:
            Dict with 'agents' (ranked) and 'total'
        """
        params = {"limit": limit}
        if agent_type:
            params["agent_type"] = agent_type
        if required_capabilities:
            params["required_capabilities"] = ",".join(required_capabilities)
        if min_trust_score is not None:
            params["min_trust_score"] = min_trust_score
        if task_type:
            params["task_type"] = task_type
        if idle_only:
            params["idle_only"] = "true"

        return self._client._get("/ai/marketplace/discover", params)

    def get_reputation(self, agent_id: str) -> Dict:
        """
        Get an agent's reputation and trust metrics.

        Returns trust score, success rates, completion times, etc.
        """
        return self._client._get(f"/ai/marketplace/agent/{agent_id}/reputation")

    def select_for_task(self, task_id: str) -> Dict:
        """
        Select the best agent for a specific task.

        Args:
            task_id: The task to find an agent for

        Returns:
            The selected agent with suitability score
        """
        return self._client._post("/ai/marketplace/select", {"task_id": task_id})

    def get_rankings(self, limit: int = 10) -> Dict:
        """
        Get agent rankings/leaderboard.

        Args:
            limit: Maximum results

        Returns:
            Dict with 'rankings' list sorted by trust score
        """
        return self._client._get("/ai/marketplace/rankings", {"limit": limit})


# =============================================================================
# LEARNING
# =============================================================================

class LearningClient:
    """Client for learning system API."""

    def __init__(self, client: AIClient):
        self._client = client

    def list_feedback(
        self,
        feedback_type: Optional[str] = None,
        outcome: Optional[str] = None,
        contribution_id: Optional[str] = None,
        agent_id: Optional[str] = None,
        processed: Optional[bool] = None,
        limit: int = 50
    ) -> Dict:
        """
        List feedback events.

        Args:
            feedback_type: Filter by type (human_review, validation_failure, etc.)
            outcome: Filter by outcome (positive, negative, neutral)
            contribution_id: Filter by contribution
            agent_id: Filter by agent
            processed: Filter by processed status
            limit: Maximum results

        Returns:
            Dict with 'feedback' list and 'total'
        """
        params = {"limit": limit}
        if feedback_type:
            params["feedback_type"] = feedback_type
        if outcome:
            params["outcome"] = outcome
        if contribution_id:
            params["contribution_id"] = contribution_id
        if agent_id:
            params["agent_id"] = agent_id
        if processed is not None:
            params["processed"] = str(processed).lower()

        return self._client._get("/ai/learning/feedback", params)

    def list_patterns(
        self,
        pattern_type: Optional[str] = None,
        min_confidence: Optional[float] = None,
        task_type: Optional[str] = None,
        limit: int = 50
    ) -> Dict:
        """
        List learned patterns.

        Args:
            pattern_type: Filter by type (success_pattern, anti_pattern, etc.)
            min_confidence: Minimum confidence threshold
            task_type: Filter by applicable task type
            limit: Maximum results

        Returns:
            Dict with 'patterns' list and 'total'
        """
        params = {"limit": limit}
        if pattern_type:
            params["pattern_type"] = pattern_type
        if min_confidence is not None:
            params["min_confidence"] = min_confidence
        if task_type:
            params["task_type"] = task_type

        return self._client._get("/ai/learning/patterns", params)

    def process_batch(self, limit: int = 100) -> Dict:
        """
        Trigger batch processing of unprocessed feedback.

        Args:
            limit: Maximum feedback events to process

        Returns:
            Processing result with counts of events processed and patterns created
        """
        return self._client._post("/ai/learning/process", {"limit": limit})

    def get_recommendations(self, task_id: str) -> Dict:
        """
        Get recommendations for a task based on learned patterns.

        Args:
            task_id: The task to get recommendations for

        Returns:
            Dict with 'recommendations' list
        """
        return self._client._get("/ai/learning/recommendations", {"task_id": task_id})


# =============================================================================
# RECOVERY
# =============================================================================

class RecoveryClient:
    """Client for autonomous recovery API."""

    def __init__(self, client: AIClient):
        self._client = client

    def get_status(self) -> Dict:
        """
        Get recovery system status.

        Returns system health, circuit breaker states, and recent stats.
        """
        return self._client._get("/ai/recovery/status")

    def retry_task(self, task_id: str) -> Dict:
        """
        Force retry a stalled or failed task.

        Args:
            task_id: The task to retry

        Returns:
            Result indicating success
        """
        return self._client._post(f"/ai/recovery/task/{task_id}/retry", {})

    def reset_circuit_breaker(self, agent_id: str) -> Dict:
        """
        Reset an agent's circuit breaker to closed state.

        Args:
            agent_id: The agent whose circuit to reset
        """
        return self._client._post(f"/ai/recovery/agent/{agent_id}/reset", {})

    def list_events(
        self,
        action_type: Optional[str] = None,
        severity: Optional[str] = None,
        entity_id: Optional[str] = None,
        limit: int = 50
    ) -> Dict:
        """
        List recovery events.

        Args:
            action_type: Filter by action type (task_recovered, circuit_opened, etc.)
            severity: Filter by severity (info, warning, error, critical)
            entity_id: Filter by entity (agent or task ID)
            limit: Maximum results

        Returns:
            Dict with 'events' list
        """
        params = {"limit": limit}
        if action_type:
            params["action_type"] = action_type
        if severity:
            params["severity"] = severity
        if entity_id:
            params["entity_id"] = entity_id

        return self._client._get("/ai/recovery/events", params)


# =============================================================================
# CONVENIENCE FUNCTIONS
# =============================================================================

def create_worker(
    base_url: str,
    database: str,
    api_key: str,
    name: str,
    agent_type: str,
    capabilities: List[str],
    url: Optional[str] = None
) -> tuple:
    """
    Convenience function to create an AI client and register as a worker.

    Args:
        base_url: SoliDB server URL
        database: Database name
        api_key: API key
        name: Worker name
        agent_type: Agent type
        capabilities: List of capabilities
        url: Optional webhook URL

    Returns:
        Tuple of (AIClient, agent_id)

    Example:
        client, agent_id = create_worker(
            "http://localhost:8080",
            "default",
            "my_key",
            "MyWorker",
            "coder",
            ["python", "rust"]
        )
    """
    client = AIClient(base_url, database, api_key)
    agent = client.agents.register(
        name=name,
        agent_type=agent_type,
        capabilities=capabilities,
        url=url
    )
    return client, agent["id"]
