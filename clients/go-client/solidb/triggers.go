package solidb

// TriggersClient provides access to database triggers management API
type TriggersClient struct {
	client *Client
}

// List returns all triggers in the database
func (t *TriggersClient) List() ([]interface{}, error) {
	res, err := t.client.SendCommand("list_triggers", map[string]interface{}{
		"database": t.client.database,
	})
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// ListByCollection returns triggers for a specific collection
func (t *TriggersClient) ListByCollection(collection string) ([]interface{}, error) {
	res, err := t.client.SendCommand("list_triggers_by_collection", map[string]interface{}{
		"database":   t.client.database,
		"collection": collection,
	})
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// Create creates a new trigger
func (t *TriggersClient) Create(name, collection, event, timing, scriptPath string, enabled *bool) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":    t.client.database,
		"name":        name,
		"collection":  collection,
		"event":       event,
		"timing":      timing,
		"script_path": scriptPath,
	}
	if enabled != nil {
		args["enabled"] = *enabled
	}
	res, err := t.client.SendCommand("create_trigger", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Get retrieves a trigger by ID
func (t *TriggersClient) Get(triggerID string) (map[string]interface{}, error) {
	res, err := t.client.SendCommand("get_trigger", map[string]interface{}{
		"database":   t.client.database,
		"trigger_id": triggerID,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Update modifies an existing trigger
func (t *TriggersClient) Update(triggerID string, updates map[string]interface{}) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":   t.client.database,
		"trigger_id": triggerID,
		"updates":    updates,
	}
	res, err := t.client.SendCommand("update_trigger", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Delete removes a trigger
func (t *TriggersClient) Delete(triggerID string) error {
	_, err := t.client.SendCommand("delete_trigger", map[string]interface{}{
		"database":   t.client.database,
		"trigger_id": triggerID,
	})
	return err
}

// Toggle enables or disables a trigger
func (t *TriggersClient) Toggle(triggerID string, enabled bool) error {
	_, err := t.client.SendCommand("toggle_trigger", map[string]interface{}{
		"database":   t.client.database,
		"trigger_id": triggerID,
		"enabled":    enabled,
	})
	return err
}
