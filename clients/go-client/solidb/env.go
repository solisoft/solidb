package solidb

// EnvClient provides access to environment variables management API
type EnvClient struct {
	client *Client
}

// List returns all environment variables
func (e *EnvClient) List() (map[string]interface{}, error) {
	res, err := e.client.SendCommand("list_env_vars", map[string]interface{}{
		"database": e.client.database,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Get retrieves an environment variable by key
func (e *EnvClient) Get(key string) (interface{}, error) {
	res, err := e.client.SendCommand("get_env_var", map[string]interface{}{
		"database": e.client.database,
		"key":      key,
	})
	if err != nil {
		return nil, err
	}
	return res, nil
}

// Set creates or updates an environment variable
func (e *EnvClient) Set(key string, value interface{}) error {
	_, err := e.client.SendCommand("set_env_var", map[string]interface{}{
		"database": e.client.database,
		"key":      key,
		"value":    value,
	})
	return err
}

// Delete removes an environment variable
func (e *EnvClient) Delete(key string) error {
	_, err := e.client.SendCommand("delete_env_var", map[string]interface{}{
		"database": e.client.database,
		"key":      key,
	})
	return err
}

// SetBulk sets multiple environment variables at once
func (e *EnvClient) SetBulk(vars map[string]interface{}) error {
	_, err := e.client.SendCommand("set_env_vars_bulk", map[string]interface{}{
		"database": e.client.database,
		"vars":     vars,
	})
	return err
}
