package solidb

// RolesClient provides access to RBAC roles management API
type RolesClient struct {
	client *Client
}

// Permission represents a role permission
type Permission struct {
	Action   string  `msgpack:"action"`
	Scope    string  `msgpack:"scope"`
	Database *string `msgpack:"database,omitempty"`
}

// List returns all roles
func (r *RolesClient) List() ([]interface{}, error) {
	res, err := r.client.SendCommand("list_roles", nil)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// Create creates a new role
func (r *RolesClient) Create(name string, permissions []map[string]interface{}, description *string) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"name":        name,
		"permissions": permissions,
	}
	if description != nil {
		args["description"] = *description
	}
	res, err := r.client.SendCommand("create_role", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Get retrieves a role by name
func (r *RolesClient) Get(name string) (map[string]interface{}, error) {
	res, err := r.client.SendCommand("get_role", map[string]interface{}{
		"role_name": name,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Update modifies an existing role
func (r *RolesClient) Update(name string, permissions []map[string]interface{}, description *string) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"role_name":   name,
		"permissions": permissions,
	}
	if description != nil {
		args["description"] = *description
	}
	res, err := r.client.SendCommand("update_role", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Delete removes a role
func (r *RolesClient) Delete(name string) error {
	_, err := r.client.SendCommand("delete_role", map[string]interface{}{
		"role_name": name,
	})
	return err
}
