package solidb

// ApiKeysClient provides access to API keys management API
type ApiKeysClient struct {
	client *Client
}

// List returns all API keys
func (a *ApiKeysClient) List() ([]interface{}, error) {
	res, err := a.client.SendCommand("list_api_keys", nil)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// Create creates a new API key
func (a *ApiKeysClient) Create(name string, permissions []map[string]interface{}, expiresAt *string) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"name":        name,
		"permissions": permissions,
	}
	if expiresAt != nil {
		args["expires_at"] = *expiresAt
	}
	res, err := a.client.SendCommand("create_api_key", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Get retrieves an API key by ID
func (a *ApiKeysClient) Get(keyID string) (map[string]interface{}, error) {
	res, err := a.client.SendCommand("get_api_key", map[string]interface{}{
		"key_id": keyID,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Delete removes an API key
func (a *ApiKeysClient) Delete(keyID string) error {
	_, err := a.client.SendCommand("delete_api_key", map[string]interface{}{
		"key_id": keyID,
	})
	return err
}

// Regenerate regenerates an API key
func (a *ApiKeysClient) Regenerate(keyID string) (map[string]interface{}, error) {
	res, err := a.client.SendCommand("regenerate_api_key", map[string]interface{}{
		"key_id": keyID,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}
