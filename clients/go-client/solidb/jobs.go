package solidb

// JobsClient provides access to background jobs management API
type JobsClient struct {
	client *Client
}

// ListQueues returns all job queues
func (j *JobsClient) ListQueues() ([]interface{}, error) {
	res, err := j.client.SendCommand("list_queues", map[string]interface{}{
		"database": j.client.database,
	})
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// ListJobs returns jobs in a specific queue
func (j *JobsClient) ListJobs(queueName string, status *string, limit, offset *int) ([]interface{}, error) {
	args := map[string]interface{}{
		"database":   j.client.database,
		"queue_name": queueName,
	}
	if status != nil {
		args["status"] = *status
	}
	if limit != nil {
		args["limit"] = *limit
	}
	if offset != nil {
		args["offset"] = *offset
	}
	res, err := j.client.SendCommand("list_jobs", args)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// Enqueue adds a new job to the queue
func (j *JobsClient) Enqueue(queueName string, scriptPath string, params map[string]interface{}, priority *int, runAt *string) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":    j.client.database,
		"queue_name":  queueName,
		"script_path": scriptPath,
	}
	if params != nil {
		args["params"] = params
	}
	if priority != nil {
		args["priority"] = *priority
	}
	if runAt != nil {
		args["run_at"] = *runAt
	}
	res, err := j.client.SendCommand("enqueue_job", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Cancel cancels a pending or running job
func (j *JobsClient) Cancel(jobID string) error {
	_, err := j.client.SendCommand("cancel_job", map[string]interface{}{
		"database": j.client.database,
		"job_id":   jobID,
	})
	return err
}

// GetJob retrieves a specific job by ID
func (j *JobsClient) GetJob(jobID string) (map[string]interface{}, error) {
	res, err := j.client.SendCommand("get_job", map[string]interface{}{
		"database": j.client.database,
		"job_id":   jobID,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}
