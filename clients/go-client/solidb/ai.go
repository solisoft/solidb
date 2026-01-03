package solidb

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"time"
)

// ContributionType represents types of contributions
type ContributionType string

const (
	ContributionTypeFeature       ContributionType = "feature"
	ContributionTypeBugfix        ContributionType = "bugfix"
	ContributionTypeEnhancement   ContributionType = "enhancement"
	ContributionTypeDocumentation ContributionType = "documentation"
)

// AgentType represents types of AI agents
type AgentType string

const (
	AgentTypeAnalyzer   AgentType = "analyzer"
	AgentTypeCoder      AgentType = "coder"
	AgentTypeTester     AgentType = "tester"
	AgentTypeReviewer   AgentType = "reviewer"
	AgentTypeIntegrator AgentType = "integrator"
)

// TaskType represents types of AI tasks
type TaskType string

const (
	TaskTypeAnalyzeContribution TaskType = "analyze_contribution"
	TaskTypeGenerateCode        TaskType = "generate_code"
	TaskTypeValidateCode        TaskType = "validate_code"
	TaskTypeRunTests            TaskType = "run_tests"
	TaskTypePrepareReview       TaskType = "prepare_review"
	TaskTypeMergeChanges        TaskType = "merge_changes"
)

// TaskStatus represents task statuses
type TaskStatus string

const (
	TaskStatusPending   TaskStatus = "pending"
	TaskStatusRunning   TaskStatus = "running"
	TaskStatusCompleted TaskStatus = "completed"
	TaskStatusFailed    TaskStatus = "failed"
	TaskStatusCancelled TaskStatus = "cancelled"
)

// Contribution represents an AI contribution
type Contribution struct {
	ID               string                 `json:"id,omitempty"`
	ContributionType ContributionType       `json:"contribution_type"`
	Description      string                 `json:"description"`
	Status           string                 `json:"status,omitempty"`
	Requester        string                 `json:"requester,omitempty"`
	Context          map[string]interface{} `json:"context,omitempty"`
	Priority         string                 `json:"priority,omitempty"`
	CreatedAt        string                 `json:"created_at,omitempty"`
	UpdatedAt        string                 `json:"updated_at,omitempty"`
}

// Task represents an AI task
type Task struct {
	ID             string                 `json:"id,omitempty"`
	ContributionID string                 `json:"contribution_id"`
	TaskType       TaskType               `json:"task_type"`
	Status         TaskStatus             `json:"status,omitempty"`
	Priority       int                    `json:"priority,omitempty"`
	Input          map[string]interface{} `json:"input,omitempty"`
	Output         map[string]interface{} `json:"output,omitempty"`
	AgentID        string                 `json:"agent_id,omitempty"`
	CreatedAt      string                 `json:"created_at,omitempty"`
	StartedAt      string                 `json:"started_at,omitempty"`
	CompletedAt    string                 `json:"completed_at,omitempty"`
}

// Agent represents an AI agent
type Agent struct {
	ID             string                 `json:"id,omitempty"`
	Name           string                 `json:"name"`
	AgentType      AgentType              `json:"agent_type"`
	Status         string                 `json:"status,omitempty"`
	URL            string                 `json:"url,omitempty"`
	Capabilities   []string               `json:"capabilities,omitempty"`
	Config         map[string]interface{} `json:"config,omitempty"`
	RegisteredAt   string                 `json:"registered_at,omitempty"`
	LastHeartbeat  string                 `json:"last_heartbeat,omitempty"`
	TasksCompleted int64                  `json:"tasks_completed,omitempty"`
	TasksFailed    int64                  `json:"tasks_failed,omitempty"`
}

// AgentReputation represents agent trust metrics
type AgentReputation struct {
	AgentID            string             `json:"agent_id"`
	TrustScore         float64            `json:"trust_score"`
	SuccessRates       map[string]float64 `json:"success_rates,omitempty"`
	AvgCompletionTimes map[string]int64   `json:"avg_completion_times,omitempty"`
}

// RankedAgent represents an agent with suitability ranking
type RankedAgent struct {
	Agent            Agent           `json:"agent"`
	Reputation       AgentReputation `json:"reputation"`
	SuitabilityScore float64         `json:"suitability_score"`
}

// FeedbackEvent represents a learning feedback event
type FeedbackEvent struct {
	ID             string `json:"id,omitempty"`
	FeedbackType   string `json:"feedback_type"`
	Outcome        string `json:"outcome"`
	ContributionID string `json:"contribution_id,omitempty"`
	TaskID         string `json:"task_id,omitempty"`
	AgentID        string `json:"agent_id,omitempty"`
	Processed      bool   `json:"processed"`
	CreatedAt      string `json:"created_at,omitempty"`
}

// Pattern represents a learned pattern
type Pattern struct {
	ID              string  `json:"id,omitempty"`
	PatternType     string  `json:"pattern_type"`
	Confidence      float64 `json:"confidence"`
	OccurrenceCount int64   `json:"occurrence_count"`
	CreatedAt       string  `json:"created_at,omitempty"`
}

// RecoveryEvent represents a recovery system event
type RecoveryEvent struct {
	ID         string `json:"id,omitempty"`
	ActionType string `json:"action_type"`
	Severity   string `json:"severity"`
	EntityID   string `json:"entity_id,omitempty"`
	Message    string `json:"message"`
	CreatedAt  string `json:"created_at,omitempty"`
}

// RecoveryStatus represents system recovery status
type RecoveryStatus struct {
	Healthy         bool   `json:"healthy"`
	LastCycle       string `json:"last_cycle,omitempty"`
	AgentsMonitored int    `json:"agents_monitored"`
	TasksRecovered  int    `json:"tasks_recovered"`
	OpenCircuits    int    `json:"open_circuits"`
}

// AIClient provides access to SoliDB AI features
type AIClient struct {
	baseURL    string
	database   string
	apiKey     string
	httpClient *http.Client

	Contributions *ContributionsClient
	Tasks         *TasksClient
	Agents        *AgentsClient
	Marketplace   *MarketplaceClient
	Learning      *LearningClient
	Recovery      *RecoveryClient
}

// NewAIClient creates a new AI client instance
func NewAIClient(baseURL, database, apiKey string) *AIClient {
	c := &AIClient{
		baseURL:  baseURL,
		database: database,
		apiKey:   apiKey,
		httpClient: &http.Client{
			Timeout: 30 * time.Second,
		},
	}

	c.Contributions = &ContributionsClient{client: c}
	c.Tasks = &TasksClient{client: c}
	c.Agents = &AgentsClient{client: c}
	c.Marketplace = &MarketplaceClient{client: c}
	c.Learning = &LearningClient{client: c}
	c.Recovery = &RecoveryClient{client: c}

	return c
}

func (c *AIClient) apiURL(path string) string {
	return fmt.Sprintf("%s/_api/database/%s%s", c.baseURL, c.database, path)
}

func (c *AIClient) doRequest(method, path string, params url.Values, body interface{}) ([]byte, error) {
	reqURL := c.apiURL(path)
	if len(params) > 0 {
		reqURL += "?" + params.Encode()
	}

	var bodyReader io.Reader
	if body != nil {
		data, err := json.Marshal(body)
		if err != nil {
			return nil, fmt.Errorf("marshal error: %w", err)
		}
		bodyReader = bytes.NewReader(data)
	}

	req, err := http.NewRequest(method, reqURL, bodyReader)
	if err != nil {
		return nil, fmt.Errorf("request creation error: %w", err)
	}

	req.Header.Set("Authorization", "Bearer "+c.apiKey)
	req.Header.Set("Content-Type", "application/json")

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return nil, fmt.Errorf("request error: %w", err)
	}
	defer resp.Body.Close()

	respBody, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("read error: %w", err)
	}

	if resp.StatusCode >= 400 {
		var errResp struct {
			Error string `json:"error"`
		}
		if json.Unmarshal(respBody, &errResp) == nil && errResp.Error != "" {
			return nil, fmt.Errorf("API error (%d): %s", resp.StatusCode, errResp.Error)
		}
		return nil, fmt.Errorf("API error (%d): %s", resp.StatusCode, string(respBody))
	}

	return respBody, nil
}

// ContributionsClient handles contribution operations
type ContributionsClient struct {
	client *AIClient
}

// Submit creates a new contribution
func (c *ContributionsClient) Submit(contribution *Contribution) (*Contribution, error) {
	data, err := c.client.doRequest("POST", "/ai/contributions", nil, contribution)
	if err != nil {
		return nil, err
	}

	var result Contribution
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// List returns contributions with optional filters
func (c *ContributionsClient) List(status, contribType string, limit, offset int) ([]Contribution, int, error) {
	params := url.Values{}
	if status != "" {
		params.Set("status", status)
	}
	if contribType != "" {
		params.Set("type", contribType)
	}
	params.Set("limit", strconv.Itoa(limit))
	params.Set("offset", strconv.Itoa(offset))

	data, err := c.client.doRequest("GET", "/ai/contributions", params, nil)
	if err != nil {
		return nil, 0, err
	}

	var result struct {
		Contributions []Contribution `json:"contributions"`
		Total         int            `json:"total"`
	}
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, 0, err
	}
	return result.Contributions, result.Total, nil
}

// Get retrieves a contribution by ID
func (c *ContributionsClient) Get(id string) (*Contribution, error) {
	data, err := c.client.doRequest("GET", "/ai/contributions/"+id, nil, nil)
	if err != nil {
		return nil, err
	}

	var result Contribution
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// Approve approves a contribution
func (c *ContributionsClient) Approve(id, feedback string) (*Contribution, error) {
	body := map[string]string{}
	if feedback != "" {
		body["feedback"] = feedback
	}

	data, err := c.client.doRequest("POST", "/ai/contributions/"+id+"/approve", nil, body)
	if err != nil {
		return nil, err
	}

	var result Contribution
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// Reject rejects a contribution
func (c *ContributionsClient) Reject(id, reason string) (*Contribution, error) {
	body := map[string]string{"reason": reason}

	data, err := c.client.doRequest("POST", "/ai/contributions/"+id+"/reject", nil, body)
	if err != nil {
		return nil, err
	}

	var result Contribution
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// TasksClient handles task operations
type TasksClient struct {
	client *AIClient
}

// List returns tasks with optional filters
func (c *TasksClient) List(status, taskType, contributionID, agentID string, limit int) ([]Task, int, error) {
	params := url.Values{}
	if status != "" {
		params.Set("status", status)
	}
	if taskType != "" {
		params.Set("task_type", taskType)
	}
	if contributionID != "" {
		params.Set("contribution_id", contributionID)
	}
	if agentID != "" {
		params.Set("agent_id", agentID)
	}
	params.Set("limit", strconv.Itoa(limit))

	data, err := c.client.doRequest("GET", "/ai/tasks", params, nil)
	if err != nil {
		return nil, 0, err
	}

	var result struct {
		Tasks []Task `json:"tasks"`
		Total int    `json:"total"`
	}
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, 0, err
	}
	return result.Tasks, result.Total, nil
}

// Get retrieves a task by ID
func (c *TasksClient) Get(id string) (*Task, error) {
	data, err := c.client.doRequest("GET", "/ai/tasks/"+id, nil, nil)
	if err != nil {
		return nil, err
	}

	var result Task
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// Claim claims a task for an agent
func (c *TasksClient) Claim(taskID, agentID string) (*Task, error) {
	body := map[string]string{"agent_id": agentID}

	data, err := c.client.doRequest("POST", "/ai/tasks/"+taskID+"/claim", nil, body)
	if err != nil {
		return nil, err
	}

	var result Task
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// Complete marks a task as completed
func (c *TasksClient) Complete(taskID string, output map[string]interface{}) (*Task, error) {
	body := map[string]interface{}{"output": output}

	data, err := c.client.doRequest("POST", "/ai/tasks/"+taskID+"/complete", nil, body)
	if err != nil {
		return nil, err
	}

	var result Task
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// Fail marks a task as failed
func (c *TasksClient) Fail(taskID, errorMsg string) (*Task, error) {
	body := map[string]string{"error": errorMsg}

	data, err := c.client.doRequest("POST", "/ai/tasks/"+taskID+"/fail", nil, body)
	if err != nil {
		return nil, err
	}

	var result Task
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// AgentsClient handles agent operations
type AgentsClient struct {
	client *AIClient
}

// Register registers a new agent
func (c *AgentsClient) Register(agent *Agent) (*Agent, error) {
	data, err := c.client.doRequest("POST", "/ai/agents", nil, agent)
	if err != nil {
		return nil, err
	}

	var result Agent
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// List returns registered agents
func (c *AgentsClient) List(status, agentType string) ([]Agent, error) {
	params := url.Values{}
	if status != "" {
		params.Set("status", status)
	}
	if agentType != "" {
		params.Set("agent_type", agentType)
	}

	data, err := c.client.doRequest("GET", "/ai/agents", params, nil)
	if err != nil {
		return nil, err
	}

	var result struct {
		Agents []Agent `json:"agents"`
	}
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return result.Agents, nil
}

// Get retrieves an agent by ID
func (c *AgentsClient) Get(id string) (*Agent, error) {
	data, err := c.client.doRequest("GET", "/ai/agents/"+id, nil, nil)
	if err != nil {
		return nil, err
	}

	var result Agent
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// Heartbeat sends a heartbeat for an agent
func (c *AgentsClient) Heartbeat(id string) error {
	_, err := c.client.doRequest("POST", "/ai/agents/"+id+"/heartbeat", nil, map[string]string{})
	return err
}

// Delete unregisters an agent
func (c *AgentsClient) Delete(id string) error {
	_, err := c.client.doRequest("DELETE", "/ai/agents/"+id, nil, nil)
	return err
}

// MarketplaceClient handles marketplace operations
type MarketplaceClient struct {
	client *AIClient
}

// Discover finds agents matching criteria
func (c *MarketplaceClient) Discover(agentType string, minTrustScore float64, limit int) ([]RankedAgent, error) {
	params := url.Values{}
	if agentType != "" {
		params.Set("agent_type", agentType)
	}
	if minTrustScore > 0 {
		params.Set("min_trust_score", fmt.Sprintf("%.2f", minTrustScore))
	}
	params.Set("limit", strconv.Itoa(limit))

	data, err := c.client.doRequest("GET", "/ai/marketplace/discover", params, nil)
	if err != nil {
		return nil, err
	}

	var result struct {
		Agents []RankedAgent `json:"agents"`
	}
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return result.Agents, nil
}

// GetReputation retrieves an agent's reputation
func (c *MarketplaceClient) GetReputation(agentID string) (*AgentReputation, error) {
	data, err := c.client.doRequest("GET", "/ai/marketplace/agent/"+agentID+"/reputation", nil, nil)
	if err != nil {
		return nil, err
	}

	var result AgentReputation
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// GetRankings retrieves agent rankings
func (c *MarketplaceClient) GetRankings(limit int) ([]map[string]interface{}, error) {
	params := url.Values{}
	params.Set("limit", strconv.Itoa(limit))

	data, err := c.client.doRequest("GET", "/ai/marketplace/rankings", params, nil)
	if err != nil {
		return nil, err
	}

	var result struct {
		Rankings []map[string]interface{} `json:"rankings"`
	}
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return result.Rankings, nil
}

// LearningClient handles learning system operations
type LearningClient struct {
	client *AIClient
}

// ListFeedback returns feedback events
func (c *LearningClient) ListFeedback(feedbackType, outcome string, limit int) ([]FeedbackEvent, error) {
	params := url.Values{}
	if feedbackType != "" {
		params.Set("feedback_type", feedbackType)
	}
	if outcome != "" {
		params.Set("outcome", outcome)
	}
	params.Set("limit", strconv.Itoa(limit))

	data, err := c.client.doRequest("GET", "/ai/learning/feedback", params, nil)
	if err != nil {
		return nil, err
	}

	var result struct {
		Feedback []FeedbackEvent `json:"feedback"`
	}
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return result.Feedback, nil
}

// ListPatterns returns learned patterns
func (c *LearningClient) ListPatterns(patternType string, minConfidence float64, limit int) ([]Pattern, error) {
	params := url.Values{}
	if patternType != "" {
		params.Set("pattern_type", patternType)
	}
	if minConfidence > 0 {
		params.Set("min_confidence", fmt.Sprintf("%.2f", minConfidence))
	}
	params.Set("limit", strconv.Itoa(limit))

	data, err := c.client.doRequest("GET", "/ai/learning/patterns", params, nil)
	if err != nil {
		return nil, err
	}

	var result struct {
		Patterns []Pattern `json:"patterns"`
	}
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return result.Patterns, nil
}

// ProcessBatch triggers batch processing of feedback
func (c *LearningClient) ProcessBatch(limit int) (int, int, error) {
	body := map[string]int{"limit": limit}

	data, err := c.client.doRequest("POST", "/ai/learning/process", nil, body)
	if err != nil {
		return 0, 0, err
	}

	var result struct {
		Processed       int `json:"processed"`
		PatternsCreated int `json:"patterns_created"`
	}
	if err := json.Unmarshal(data, &result); err != nil {
		return 0, 0, err
	}
	return result.Processed, result.PatternsCreated, nil
}

// RecoveryClient handles recovery system operations
type RecoveryClient struct {
	client *AIClient
}

// GetStatus retrieves recovery system status
func (c *RecoveryClient) GetStatus() (*RecoveryStatus, error) {
	data, err := c.client.doRequest("GET", "/ai/recovery/status", nil, nil)
	if err != nil {
		return nil, err
	}

	var result RecoveryStatus
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// RetryTask forces retry of a task
func (c *RecoveryClient) RetryTask(taskID string) error {
	_, err := c.client.doRequest("POST", "/ai/recovery/task/"+taskID+"/retry", nil, map[string]string{})
	return err
}

// ResetCircuitBreaker resets an agent's circuit breaker
func (c *RecoveryClient) ResetCircuitBreaker(agentID string) error {
	_, err := c.client.doRequest("POST", "/ai/recovery/agent/"+agentID+"/reset", nil, map[string]string{})
	return err
}

// ListEvents returns recovery events
func (c *RecoveryClient) ListEvents(actionType, severity string, limit int) ([]RecoveryEvent, error) {
	params := url.Values{}
	if actionType != "" {
		params.Set("action_type", actionType)
	}
	if severity != "" {
		params.Set("severity", severity)
	}
	params.Set("limit", strconv.Itoa(limit))

	data, err := c.client.doRequest("GET", "/ai/recovery/events", params, nil)
	if err != nil {
		return nil, err
	}

	var result struct {
		Events []RecoveryEvent `json:"events"`
	}
	if err := json.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return result.Events, nil
}

// CreateWorker is a convenience function to create a client and register as a worker
func CreateWorker(baseURL, database, apiKey, name string, agentType AgentType, capabilities []string, webhookURL string) (*AIClient, string, error) {
	client := NewAIClient(baseURL, database, apiKey)

	agent := &Agent{
		Name:         name,
		AgentType:    agentType,
		Capabilities: capabilities,
		URL:          webhookURL,
	}

	registered, err := client.Agents.Register(agent)
	if err != nil {
		return nil, "", err
	}

	return client, registered.ID, nil
}
