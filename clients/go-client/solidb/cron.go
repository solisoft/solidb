package solidb

// CronClient provides access to cron job management API
type CronClient struct {
	client *Client
}

// List returns all cron jobs
func (c *CronClient) List() ([]interface{}, error) {
	res, err := c.client.SendCommand("list_cron_jobs", map[string]interface{}{
		"database": c.client.database,
	})
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// Create creates a new cron job
func (c *CronClient) Create(name, schedule, scriptPath string, params map[string]interface{}, enabled *bool, description *string) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":    c.client.database,
		"name":        name,
		"schedule":    schedule,
		"script_path": scriptPath,
	}
	if params != nil {
		args["params"] = params
	}
	if enabled != nil {
		args["enabled"] = *enabled
	}
	if description != nil {
		args["description"] = *description
	}
	res, err := c.client.SendCommand("create_cron_job", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Get retrieves a cron job by ID
func (c *CronClient) Get(cronID string) (map[string]interface{}, error) {
	res, err := c.client.SendCommand("get_cron_job", map[string]interface{}{
		"database": c.client.database,
		"cron_id":  cronID,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Update modifies an existing cron job
func (c *CronClient) Update(cronID string, updates map[string]interface{}) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database": c.client.database,
		"cron_id":  cronID,
		"updates":  updates,
	}
	res, err := c.client.SendCommand("update_cron_job", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Delete removes a cron job
func (c *CronClient) Delete(cronID string) error {
	_, err := c.client.SendCommand("delete_cron_job", map[string]interface{}{
		"database": c.client.database,
		"cron_id":  cronID,
	})
	return err
}

// Toggle enables or disables a cron job
func (c *CronClient) Toggle(cronID string, enabled bool) error {
	_, err := c.client.SendCommand("toggle_cron_job", map[string]interface{}{
		"database": c.client.database,
		"cron_id":  cronID,
		"enabled":  enabled,
	})
	return err
}
