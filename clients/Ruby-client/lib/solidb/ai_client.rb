require 'net/http'
require 'uri'
require 'json'

module SoliDB
  # Contribution types
  module ContributionType
    FEATURE = 'feature'
    BUGFIX = 'bugfix'
    ENHANCEMENT = 'enhancement'
    DOCUMENTATION = 'documentation'
  end

  # Agent types
  module AgentType
    ANALYZER = 'analyzer'
    CODER = 'coder'
    TESTER = 'tester'
    REVIEWER = 'reviewer'
    INTEGRATOR = 'integrator'
  end

  # Task types
  module TaskType
    ANALYZE_CONTRIBUTION = 'analyze_contribution'
    GENERATE_CODE = 'generate_code'
    VALIDATE_CODE = 'validate_code'
    RUN_TESTS = 'run_tests'
    PREPARE_REVIEW = 'prepare_review'
    MERGE_CHANGES = 'merge_changes'
  end

  # Task statuses
  module TaskStatus
    PENDING = 'pending'
    RUNNING = 'running'
    COMPLETED = 'completed'
    FAILED = 'failed'
    CANCELLED = 'cancelled'
  end

  # Feedback types
  module FeedbackType
    HUMAN_REVIEW = 'human_review'
    VALIDATION_FAILURE = 'validation_failure'
    TEST_FAILURE = 'test_failure'
    TASK_ESCALATION = 'task_escalation'
  end

  # Pattern types
  module PatternType
    SUCCESS = 'success'
    ANTI_PATTERN = 'anti_pattern'
    ERROR = 'error'
  end

  # Circuit breaker states
  module CircuitState
    CLOSED = 'closed'
    OPEN = 'open'
    HALF_OPEN = 'half_open'
  end

  class AIClientError < StandardError; end

  # AI Client for SoliDB AI features
  # Uses HTTP REST API for AI operations (separate from wire protocol)
  class AIClient
    attr_reader :contributions, :tasks, :agents, :marketplace, :learning, :recovery

    def initialize(base_url, database, api_key, timeout: 30)
      @base_url = base_url.chomp('/')
      @database = database
      @api_key = api_key
      @timeout = timeout

      @contributions = ContributionsClient.new(self)
      @tasks = TasksClient.new(self)
      @agents = AgentsClient.new(self)
      @marketplace = MarketplaceClient.new(self)
      @learning = LearningClient.new(self)
      @recovery = RecoveryClient.new(self)
    end

    def api_url(path)
      "#{@base_url}/_api/database/#{@database}#{path}"
    end

    def request(method, path, params: {}, body: nil)
      uri = URI.parse(api_url(path))
      uri.query = URI.encode_www_form(params) unless params.empty?

      http = Net::HTTP.new(uri.host, uri.port)
      http.use_ssl = uri.scheme == 'https'
      http.read_timeout = @timeout
      http.open_timeout = @timeout

      request = case method.to_s.upcase
      when 'GET'
        Net::HTTP::Get.new(uri)
      when 'POST'
        Net::HTTP::Post.new(uri)
      when 'PUT'
        Net::HTTP::Put.new(uri)
      when 'DELETE'
        Net::HTTP::Delete.new(uri)
      else
        raise AIClientError, "Unsupported HTTP method: #{method}"
      end

      request['Authorization'] = "Bearer #{@api_key}"
      request['Content-Type'] = 'application/json'
      request.body = body.to_json if body

      response = http.request(request)

      if response.code.to_i >= 400
        error_msg = begin
          JSON.parse(response.body)['error']
        rescue
          response.body
        end
        raise AIClientError, "API error (#{response.code}): #{error_msg}"
      end

      response.body.empty? ? {} : JSON.parse(response.body)
    end
  end

  # Contributions client
  class ContributionsClient
    def initialize(client)
      @client = client
    end

    def submit(contribution)
      @client.request(:post, '/ai/contributions', body: contribution)
    end

    def list(status: nil, type: nil, limit: 50, offset: 0)
      params = { limit: limit, offset: offset }
      params[:status] = status if status
      params[:type] = type if type
      @client.request(:get, '/ai/contributions', params: params)
    end

    def get(id)
      @client.request(:get, "/ai/contributions/#{id}")
    end

    def approve(id, feedback: nil)
      body = feedback ? { feedback: feedback } : {}
      @client.request(:post, "/ai/contributions/#{id}/approve", body: body)
    end

    def reject(id, reason:)
      @client.request(:post, "/ai/contributions/#{id}/reject", body: { reason: reason })
    end
  end

  # Tasks client
  class TasksClient
    def initialize(client)
      @client = client
    end

    def list(status: nil, task_type: nil, contribution_id: nil, agent_id: nil, limit: 50)
      params = { limit: limit }
      params[:status] = status if status
      params[:task_type] = task_type if task_type
      params[:contribution_id] = contribution_id if contribution_id
      params[:agent_id] = agent_id if agent_id
      @client.request(:get, '/ai/tasks', params: params)
    end

    def get(id)
      @client.request(:get, "/ai/tasks/#{id}")
    end

    def claim(task_id, agent_id)
      @client.request(:post, "/ai/tasks/#{task_id}/claim", body: { agent_id: agent_id })
    end

    def complete(task_id, output)
      @client.request(:post, "/ai/tasks/#{task_id}/complete", body: { output: output })
    end

    def fail(task_id, error)
      @client.request(:post, "/ai/tasks/#{task_id}/fail", body: { error: error })
    end
  end

  # Agents client
  class AgentsClient
    def initialize(client)
      @client = client
    end

    def register(agent)
      @client.request(:post, '/ai/agents', body: agent)
    end

    def list(status: nil, agent_type: nil)
      params = {}
      params[:status] = status if status
      params[:agent_type] = agent_type if agent_type
      @client.request(:get, '/ai/agents', params: params)
    end

    def get(id)
      @client.request(:get, "/ai/agents/#{id}")
    end

    def heartbeat(id)
      @client.request(:post, "/ai/agents/#{id}/heartbeat", body: {})
    end

    def delete(id)
      @client.request(:delete, "/ai/agents/#{id}")
    end
  end

  # Marketplace client
  class MarketplaceClient
    def initialize(client)
      @client = client
    end

    def discover(agent_type: nil, min_trust_score: nil, limit: 10)
      params = { limit: limit }
      params[:agent_type] = agent_type if agent_type
      params[:min_trust_score] = min_trust_score if min_trust_score
      @client.request(:get, '/ai/marketplace/discover', params: params)
    end

    def get_reputation(agent_id)
      @client.request(:get, "/ai/marketplace/agent/#{agent_id}/reputation")
    end

    def get_rankings(limit: 10)
      @client.request(:get, '/ai/marketplace/rankings', params: { limit: limit })
    end
  end

  # Learning client
  class LearningClient
    def initialize(client)
      @client = client
    end

    def list_feedback(feedback_type: nil, outcome: nil, limit: 50)
      params = { limit: limit }
      params[:feedback_type] = feedback_type if feedback_type
      params[:outcome] = outcome if outcome
      @client.request(:get, '/ai/learning/feedback', params: params)
    end

    def list_patterns(pattern_type: nil, min_confidence: nil, limit: 50)
      params = { limit: limit }
      params[:pattern_type] = pattern_type if pattern_type
      params[:min_confidence] = min_confidence if min_confidence
      @client.request(:get, '/ai/learning/patterns', params: params)
    end

    def process_batch(limit: 100)
      @client.request(:post, '/ai/learning/process', body: { limit: limit })
    end
  end

  # Recovery client
  class RecoveryClient
    def initialize(client)
      @client = client
    end

    def status
      @client.request(:get, '/ai/recovery/status')
    end

    def retry_task(task_id)
      @client.request(:post, "/ai/recovery/task/#{task_id}/retry", body: {})
    end

    def reset_circuit_breaker(agent_id)
      @client.request(:post, "/ai/recovery/agent/#{agent_id}/reset", body: {})
    end

    def list_events(action_type: nil, severity: nil, limit: 50)
      params = { limit: limit }
      params[:action_type] = action_type if action_type
      params[:severity] = severity if severity
      @client.request(:get, '/ai/recovery/events', params: params)
    end
  end

  # Helper to create a worker agent
  def self.create_worker(base_url, database, api_key, name, agent_type, capabilities: [], webhook_url: nil)
    client = AIClient.new(base_url, database, api_key)

    agent = {
      name: name,
      agent_type: agent_type,
      capabilities: capabilities
    }
    agent[:url] = webhook_url if webhook_url

    registered = client.agents.register(agent)
    [client, registered['id'] || registered['_key']]
  end
end
