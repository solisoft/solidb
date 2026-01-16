package solidb

// TTLClient provides access to TTL (time-to-live) index management API
type TTLClient struct {
	client *Client
}

// CreateIndex creates a TTL index
func (t *TTLClient) CreateIndex(collection, name, field string, expireAfterSeconds int) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":             t.client.database,
		"collection":           collection,
		"name":                 name,
		"field":                field,
		"expire_after_seconds": expireAfterSeconds,
	}
	res, err := t.client.SendCommand("create_ttl_index", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// ListIndexes returns all TTL indexes in a collection
func (t *TTLClient) ListIndexes(collection string) ([]interface{}, error) {
	res, err := t.client.SendCommand("list_ttl_indexes", map[string]interface{}{
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

// DeleteIndex deletes a TTL index
func (t *TTLClient) DeleteIndex(collection, indexName string) error {
	_, err := t.client.SendCommand("delete_ttl_index", map[string]interface{}{
		"database":   t.client.database,
		"collection": collection,
		"index_name": indexName,
	})
	return err
}

// UpdateExpiration updates the expiration time of a TTL index
func (t *TTLClient) UpdateExpiration(collection, indexName string, expireAfterSeconds int) error {
	_, err := t.client.SendCommand("update_ttl_expiration", map[string]interface{}{
		"database":             t.client.database,
		"collection":           collection,
		"index_name":           indexName,
		"expire_after_seconds": expireAfterSeconds,
	})
	return err
}

// GetIndexInfo returns information about a TTL index
func (t *TTLClient) GetIndexInfo(collection, indexName string) (map[string]interface{}, error) {
	res, err := t.client.SendCommand("ttl_index_info", map[string]interface{}{
		"database":   t.client.database,
		"collection": collection,
		"index_name": indexName,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// RunCleanup manually triggers TTL cleanup
func (t *TTLClient) RunCleanup(collection string) (map[string]interface{}, error) {
	res, err := t.client.SendCommand("ttl_run_cleanup", map[string]interface{}{
		"database":   t.client.database,
		"collection": collection,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}
