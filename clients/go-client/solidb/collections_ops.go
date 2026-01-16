package solidb

// CollectionsClient provides access to advanced collection management API
type CollectionsClient struct {
	client *Client
}

// Truncate removes all documents from a collection
func (c *CollectionsClient) Truncate(collection string) error {
	_, err := c.client.SendCommand("truncate_collection", map[string]interface{}{
		"database":   c.client.database,
		"collection": collection,
	})
	return err
}

// Compact triggers compaction on a collection
func (c *CollectionsClient) Compact(collection string) error {
	_, err := c.client.SendCommand("compact_collection", map[string]interface{}{
		"database":   c.client.database,
		"collection": collection,
	})
	return err
}

// Stats returns collection statistics
func (c *CollectionsClient) Stats(collection string) (map[string]interface{}, error) {
	res, err := c.client.SendCommand("collection_stats", map[string]interface{}{
		"database":   c.client.database,
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

// Prune removes old documents based on criteria
func (c *CollectionsClient) Prune(collection string, olderThan *string, field *string) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":   c.client.database,
		"collection": collection,
	}
	if olderThan != nil {
		args["older_than"] = *olderThan
	}
	if field != nil {
		args["field"] = *field
	}
	res, err := c.client.SendCommand("prune_collection", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Recount recounts documents in a collection
func (c *CollectionsClient) Recount(collection string) (map[string]interface{}, error) {
	res, err := c.client.SendCommand("recount_collection", map[string]interface{}{
		"database":   c.client.database,
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

// Repair repairs a collection
func (c *CollectionsClient) Repair(collection string) (map[string]interface{}, error) {
	res, err := c.client.SendCommand("repair_collection", map[string]interface{}{
		"database":   c.client.database,
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

// SetSchema sets the JSON schema for a collection
func (c *CollectionsClient) SetSchema(collection string, schema map[string]interface{}) error {
	_, err := c.client.SendCommand("set_collection_schema", map[string]interface{}{
		"database":   c.client.database,
		"collection": collection,
		"schema":     schema,
	})
	return err
}

// GetSchema retrieves the JSON schema for a collection
func (c *CollectionsClient) GetSchema(collection string) (map[string]interface{}, error) {
	res, err := c.client.SendCommand("get_collection_schema", map[string]interface{}{
		"database":   c.client.database,
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

// DeleteSchema removes the JSON schema from a collection
func (c *CollectionsClient) DeleteSchema(collection string) error {
	_, err := c.client.SendCommand("delete_collection_schema", map[string]interface{}{
		"database":   c.client.database,
		"collection": collection,
	})
	return err
}

// Export exports collection data
func (c *CollectionsClient) Export(collection, format string) (interface{}, error) {
	res, err := c.client.SendCommand("export_collection", map[string]interface{}{
		"database":   c.client.database,
		"collection": collection,
		"format":     format,
	})
	if err != nil {
		return nil, err
	}
	return res, nil
}

// Import imports data into a collection
func (c *CollectionsClient) Import(collection string, data interface{}, format string) (map[string]interface{}, error) {
	res, err := c.client.SendCommand("import_collection", map[string]interface{}{
		"database":   c.client.database,
		"collection": collection,
		"data":       data,
		"format":     format,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// GetSharding returns sharding configuration for a collection
func (c *CollectionsClient) GetSharding(collection string) (map[string]interface{}, error) {
	res, err := c.client.SendCommand("get_collection_sharding", map[string]interface{}{
		"database":   c.client.database,
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

// SetSharding sets sharding configuration for a collection
func (c *CollectionsClient) SetSharding(collection string, config map[string]interface{}) error {
	args := map[string]interface{}{
		"database":   c.client.database,
		"collection": collection,
	}
	for k, v := range config {
		args[k] = v
	}
	_, err := c.client.SendCommand("set_collection_sharding", args)
	return err
}
