package solidb

// IndexesClient provides access to advanced index management API
type IndexesClient struct {
	client *Client
}

// Rebuild rebuilds an index
func (i *IndexesClient) Rebuild(collection, indexName string) error {
	_, err := i.client.SendCommand("rebuild_index", map[string]interface{}{
		"database":   i.client.database,
		"collection": collection,
		"index_name": indexName,
	})
	return err
}

// RebuildAll rebuilds all indexes in a collection
func (i *IndexesClient) RebuildAll(collection string) error {
	_, err := i.client.SendCommand("rebuild_all_indexes", map[string]interface{}{
		"database":   i.client.database,
		"collection": collection,
	})
	return err
}

// HybridSearch performs hybrid search combining multiple search types
func (i *IndexesClient) HybridSearch(collection string, query map[string]interface{}) ([]interface{}, error) {
	args := map[string]interface{}{
		"database":   i.client.database,
		"collection": collection,
	}
	for k, v := range query {
		args[k] = v
	}
	res, err := i.client.SendCommand("hybrid_search", args)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// Analyze analyzes an index
func (i *IndexesClient) Analyze(collection, indexName string) (map[string]interface{}, error) {
	res, err := i.client.SendCommand("analyze_index", map[string]interface{}{
		"database":   i.client.database,
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

// GetUsageStats returns index usage statistics
func (i *IndexesClient) GetUsageStats(collection string) (map[string]interface{}, error) {
	res, err := i.client.SendCommand("index_usage_stats", map[string]interface{}{
		"database":   i.client.database,
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
