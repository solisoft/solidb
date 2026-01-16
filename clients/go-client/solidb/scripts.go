package solidb

// ScriptsClient provides access to Lua scripts management API
type ScriptsClient struct {
	client *Client
}

// Create creates a new Lua script
func (s *ScriptsClient) Create(name, path string, methods []string, code string, description, collection *string) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database": s.client.database,
		"name":     name,
		"path":     path,
		"methods":  methods,
		"code":     code,
	}
	if description != nil {
		args["description"] = *description
	}
	if collection != nil {
		args["collection"] = *collection
	}
	res, err := s.client.SendCommand("create_script", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// List returns all scripts in the database
func (s *ScriptsClient) List() ([]interface{}, error) {
	res, err := s.client.SendCommand("list_scripts", map[string]interface{}{
		"database": s.client.database,
	})
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// Get retrieves a script by ID
func (s *ScriptsClient) Get(scriptID string) (map[string]interface{}, error) {
	res, err := s.client.SendCommand("get_script", map[string]interface{}{
		"database":  s.client.database,
		"script_id": scriptID,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Update modifies an existing script
func (s *ScriptsClient) Update(scriptID string, updates map[string]interface{}) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":  s.client.database,
		"script_id": scriptID,
		"updates":   updates,
	}
	res, err := s.client.SendCommand("update_script", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Delete removes a script
func (s *ScriptsClient) Delete(scriptID string) error {
	_, err := s.client.SendCommand("delete_script", map[string]interface{}{
		"database":  s.client.database,
		"script_id": scriptID,
	})
	return err
}

// GetStats retrieves script execution statistics
func (s *ScriptsClient) GetStats() (map[string]interface{}, error) {
	res, err := s.client.SendCommand("get_script_stats", nil)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}
