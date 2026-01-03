defmodule SoliDB.AIClient do
  @moduledoc """
  AI Client for SoliDB AI features.
  Uses HTTP REST API for AI operations (separate from wire protocol).
  """

  defstruct [:base_url, :database, :api_key, :timeout]

  @type t :: %__MODULE__{
    base_url: String.t(),
    database: String.t(),
    api_key: String.t(),
    timeout: integer()
  }

  # Contribution types
  defmodule ContributionType do
    def feature, do: "feature"
    def bugfix, do: "bugfix"
    def enhancement, do: "enhancement"
    def documentation, do: "documentation"
  end

  # Agent types
  defmodule AgentType do
    def analyzer, do: "analyzer"
    def coder, do: "coder"
    def tester, do: "tester"
    def reviewer, do: "reviewer"
    def integrator, do: "integrator"
  end

  # Task types
  defmodule TaskType do
    def analyze_contribution, do: "analyze_contribution"
    def generate_code, do: "generate_code"
    def validate_code, do: "validate_code"
    def run_tests, do: "run_tests"
    def prepare_review, do: "prepare_review"
    def merge_changes, do: "merge_changes"
  end

  # Task statuses
  defmodule TaskStatus do
    def pending, do: "pending"
    def running, do: "running"
    def completed, do: "completed"
    def failed, do: "failed"
    def cancelled, do: "cancelled"
  end

  # Feedback types
  defmodule FeedbackType do
    def human_review, do: "human_review"
    def validation_failure, do: "validation_failure"
    def test_failure, do: "test_failure"
    def task_escalation, do: "task_escalation"
  end

  # Pattern types
  defmodule PatternType do
    def success, do: "success"
    def anti_pattern, do: "anti_pattern"
    def error, do: "error"
  end

  # Circuit breaker states
  defmodule CircuitState do
    def closed, do: "closed"
    def open, do: "open"
    def half_open, do: "half_open"
  end

  @doc """
  Creates a new AI client instance.
  """
  def new(base_url, database, api_key, opts \\ []) do
    timeout = Keyword.get(opts, :timeout, 30_000)
    %__MODULE__{
      base_url: String.trim_trailing(base_url, "/"),
      database: database,
      api_key: api_key,
      timeout: timeout
    }
  end

  @doc """
  Builds the API URL for the given path.
  """
  def api_url(%__MODULE__{base_url: base_url, database: database}, path) do
    "#{base_url}/_api/database/#{database}#{path}"
  end

  @doc """
  Makes an HTTP request to the API.
  """
  def request(client, method, path, params \\ %{}, body \\ nil) do
    url = api_url(client, path)
    url = if map_size(params) > 0, do: "#{url}?#{URI.encode_query(params)}", else: url

    headers = [
      {"Authorization", "Bearer #{client.api_key}"},
      {"Content-Type", "application/json"}
    ]

    body_data = if body, do: Jason.encode!(body), else: ""

    case :httpc.request(
      method,
      {String.to_charlist(url), Enum.map(headers, fn {k, v} -> {String.to_charlist(k), String.to_charlist(v)} end), 'application/json', body_data},
      [timeout: client.timeout],
      []
    ) do
      {:ok, {{_, status, _}, _, response_body}} ->
        response = if response_body == '', do: %{}, else: Jason.decode!(to_string(response_body))
        if status >= 400 do
          {:error, "API error (#{status}): #{response["error"] || to_string(response_body)}"}
        else
          {:ok, response}
        end
      {:error, reason} ->
        {:error, "Request failed: #{inspect(reason)}"}
    end
  end

  # ============================================================================
  # Contributions
  # ============================================================================

  defmodule Contributions do
    @doc "Submit a new contribution"
    def submit(client, contribution) do
      SoliDB.AIClient.request(client, :post, "/ai/contributions", %{}, contribution)
    end

    @doc "List contributions with optional filters"
    def list(client, opts \\ []) do
      params = %{limit: Keyword.get(opts, :limit, 50), offset: Keyword.get(opts, :offset, 0)}
      params = if opts[:status], do: Map.put(params, :status, opts[:status]), else: params
      params = if opts[:type], do: Map.put(params, :type, opts[:type]), else: params
      SoliDB.AIClient.request(client, :get, "/ai/contributions", params)
    end

    @doc "Get a contribution by ID"
    def get(client, id) do
      SoliDB.AIClient.request(client, :get, "/ai/contributions/#{id}")
    end

    @doc "Approve a contribution"
    def approve(client, id, feedback \\ nil) do
      body = if feedback, do: %{feedback: feedback}, else: %{}
      SoliDB.AIClient.request(client, :post, "/ai/contributions/#{id}/approve", %{}, body)
    end

    @doc "Reject a contribution"
    def reject(client, id, reason) do
      SoliDB.AIClient.request(client, :post, "/ai/contributions/#{id}/reject", %{}, %{reason: reason})
    end
  end

  # ============================================================================
  # Tasks
  # ============================================================================

  defmodule Tasks do
    @doc "List tasks with optional filters"
    def list(client, opts \\ []) do
      params = %{limit: Keyword.get(opts, :limit, 50)}
      params = if opts[:status], do: Map.put(params, :status, opts[:status]), else: params
      params = if opts[:task_type], do: Map.put(params, :task_type, opts[:task_type]), else: params
      params = if opts[:contribution_id], do: Map.put(params, :contribution_id, opts[:contribution_id]), else: params
      params = if opts[:agent_id], do: Map.put(params, :agent_id, opts[:agent_id]), else: params
      SoliDB.AIClient.request(client, :get, "/ai/tasks", params)
    end

    @doc "Get a task by ID"
    def get(client, id) do
      SoliDB.AIClient.request(client, :get, "/ai/tasks/#{id}")
    end

    @doc "Claim a task for an agent"
    def claim(client, task_id, agent_id) do
      SoliDB.AIClient.request(client, :post, "/ai/tasks/#{task_id}/claim", %{}, %{agent_id: agent_id})
    end

    @doc "Complete a task with output"
    def complete(client, task_id, output) do
      SoliDB.AIClient.request(client, :post, "/ai/tasks/#{task_id}/complete", %{}, %{output: output})
    end

    @doc "Fail a task with error message"
    def fail(client, task_id, error) do
      SoliDB.AIClient.request(client, :post, "/ai/tasks/#{task_id}/fail", %{}, %{error: error})
    end
  end

  # ============================================================================
  # Agents
  # ============================================================================

  defmodule Agents do
    @doc "Register a new agent"
    def register(client, agent) do
      SoliDB.AIClient.request(client, :post, "/ai/agents", %{}, agent)
    end

    @doc "List agents with optional filters"
    def list(client, opts \\ []) do
      params = %{}
      params = if opts[:status], do: Map.put(params, :status, opts[:status]), else: params
      params = if opts[:agent_type], do: Map.put(params, :agent_type, opts[:agent_type]), else: params
      SoliDB.AIClient.request(client, :get, "/ai/agents", params)
    end

    @doc "Get an agent by ID"
    def get(client, id) do
      SoliDB.AIClient.request(client, :get, "/ai/agents/#{id}")
    end

    @doc "Send heartbeat for an agent"
    def heartbeat(client, id) do
      SoliDB.AIClient.request(client, :post, "/ai/agents/#{id}/heartbeat", %{}, %{})
    end

    @doc "Delete an agent"
    def delete(client, id) do
      SoliDB.AIClient.request(client, :delete, "/ai/agents/#{id}")
    end
  end

  # ============================================================================
  # Marketplace
  # ============================================================================

  defmodule Marketplace do
    @doc "Discover agents matching criteria"
    def discover(client, opts \\ []) do
      params = %{limit: Keyword.get(opts, :limit, 10)}
      params = if opts[:agent_type], do: Map.put(params, :agent_type, opts[:agent_type]), else: params
      params = if opts[:min_trust_score], do: Map.put(params, :min_trust_score, opts[:min_trust_score]), else: params
      SoliDB.AIClient.request(client, :get, "/ai/marketplace/discover", params)
    end

    @doc "Get agent reputation"
    def get_reputation(client, agent_id) do
      SoliDB.AIClient.request(client, :get, "/ai/marketplace/agent/#{agent_id}/reputation")
    end

    @doc "Get agent rankings"
    def get_rankings(client, limit \\ 10) do
      SoliDB.AIClient.request(client, :get, "/ai/marketplace/rankings", %{limit: limit})
    end
  end

  # ============================================================================
  # Learning
  # ============================================================================

  defmodule Learning do
    @doc "List feedback events"
    def list_feedback(client, opts \\ []) do
      params = %{limit: Keyword.get(opts, :limit, 50)}
      params = if opts[:feedback_type], do: Map.put(params, :feedback_type, opts[:feedback_type]), else: params
      params = if opts[:outcome], do: Map.put(params, :outcome, opts[:outcome]), else: params
      SoliDB.AIClient.request(client, :get, "/ai/learning/feedback", params)
    end

    @doc "List learned patterns"
    def list_patterns(client, opts \\ []) do
      params = %{limit: Keyword.get(opts, :limit, 50)}
      params = if opts[:pattern_type], do: Map.put(params, :pattern_type, opts[:pattern_type]), else: params
      params = if opts[:min_confidence], do: Map.put(params, :min_confidence, opts[:min_confidence]), else: params
      SoliDB.AIClient.request(client, :get, "/ai/learning/patterns", params)
    end

    @doc "Process feedback batch"
    def process_batch(client, limit \\ 100) do
      SoliDB.AIClient.request(client, :post, "/ai/learning/process", %{}, %{limit: limit})
    end
  end

  # ============================================================================
  # Recovery
  # ============================================================================

  defmodule Recovery do
    @doc "Get recovery system status"
    def status(client) do
      SoliDB.AIClient.request(client, :get, "/ai/recovery/status")
    end

    @doc "Force retry a task"
    def retry_task(client, task_id) do
      SoliDB.AIClient.request(client, :post, "/ai/recovery/task/#{task_id}/retry", %{}, %{})
    end

    @doc "Reset circuit breaker for an agent"
    def reset_circuit_breaker(client, agent_id) do
      SoliDB.AIClient.request(client, :post, "/ai/recovery/agent/#{agent_id}/reset", %{}, %{})
    end

    @doc "List recovery events"
    def list_events(client, opts \\ []) do
      params = %{limit: Keyword.get(opts, :limit, 50)}
      params = if opts[:action_type], do: Map.put(params, :action_type, opts[:action_type]), else: params
      params = if opts[:severity], do: Map.put(params, :severity, opts[:severity]), else: params
      SoliDB.AIClient.request(client, :get, "/ai/recovery/events", params)
    end
  end

  # ============================================================================
  # Helper
  # ============================================================================

  @doc """
  Creates a worker agent and returns the client and agent ID.
  """
  def create_worker(base_url, database, api_key, name, agent_type, opts \\ []) do
    client = new(base_url, database, api_key)
    capabilities = Keyword.get(opts, :capabilities, [])
    webhook_url = Keyword.get(opts, :webhook_url)

    agent = %{
      name: name,
      agent_type: agent_type,
      capabilities: capabilities
    }
    agent = if webhook_url, do: Map.put(agent, :url, webhook_url), else: agent

    case Agents.register(client, agent) do
      {:ok, registered} ->
        {:ok, client, registered["id"] || registered["_key"]}
      {:error, reason} ->
        {:error, reason}
    end
  end
end
