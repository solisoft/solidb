package solidb

// UsersClient provides access to user management API
type UsersClient struct {
	client *Client
}

// List returns all users
func (u *UsersClient) List() ([]interface{}, error) {
	res, err := u.client.SendCommand("list_users", nil)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// Create creates a new user
func (u *UsersClient) Create(username, password string, roles []string) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"username": username,
		"password": password,
	}
	if roles != nil {
		args["roles"] = roles
	}
	res, err := u.client.SendCommand("create_user", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Get retrieves a user by username
func (u *UsersClient) Get(username string) (map[string]interface{}, error) {
	res, err := u.client.SendCommand("get_user", map[string]interface{}{
		"username": username,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Delete removes a user
func (u *UsersClient) Delete(username string) error {
	_, err := u.client.SendCommand("delete_user", map[string]interface{}{
		"username": username,
	})
	return err
}

// GetRoles returns roles assigned to a user
func (u *UsersClient) GetRoles(username string) ([]interface{}, error) {
	res, err := u.client.SendCommand("get_user_roles", map[string]interface{}{
		"username": username,
	})
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// AssignRole assigns a role to a user
func (u *UsersClient) AssignRole(username, role string, database *string) error {
	args := map[string]interface{}{
		"username": username,
		"role":     role,
	}
	if database != nil {
		args["database"] = *database
	}
	_, err := u.client.SendCommand("assign_role", args)
	return err
}

// RevokeRole removes a role from a user
func (u *UsersClient) RevokeRole(username, role string, database *string) error {
	args := map[string]interface{}{
		"username": username,
		"role":     role,
	}
	if database != nil {
		args["database"] = *database
	}
	_, err := u.client.SendCommand("revoke_role", args)
	return err
}

// Me returns the current authenticated user
func (u *UsersClient) Me() (map[string]interface{}, error) {
	res, err := u.client.SendCommand("get_current_user", nil)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// MyPermissions returns permissions of the current user
func (u *UsersClient) MyPermissions() ([]interface{}, error) {
	res, err := u.client.SendCommand("get_my_permissions", nil)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// ChangePassword changes a user's password
func (u *UsersClient) ChangePassword(username, oldPassword, newPassword string) error {
	_, err := u.client.SendCommand("change_password", map[string]interface{}{
		"username":     username,
		"old_password": oldPassword,
		"new_password": newPassword,
	})
	return err
}
