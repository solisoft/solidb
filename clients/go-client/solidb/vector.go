package solidb

// VectorClient provides access to vector index and search API
type VectorClient struct {
	client *Client
}

// CreateIndex creates a vector index
func (v *VectorClient) CreateIndex(collection, name, field string, dimensions int, metric *string, options map[string]interface{}) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":   v.client.database,
		"collection": collection,
		"name":       name,
		"field":      field,
		"dimensions": dimensions,
	}
	if metric != nil {
		args["metric"] = *metric
	}
	if options != nil {
		for k, val := range options {
			args[k] = val
		}
	}
	res, err := v.client.SendCommand("create_vector_index", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// ListIndexes returns all vector indexes in a collection
func (v *VectorClient) ListIndexes(collection string) ([]interface{}, error) {
	res, err := v.client.SendCommand("list_vector_indexes", map[string]interface{}{
		"database":   v.client.database,
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

// DeleteIndex deletes a vector index
func (v *VectorClient) DeleteIndex(collection, indexName string) error {
	_, err := v.client.SendCommand("delete_vector_index", map[string]interface{}{
		"database":   v.client.database,
		"collection": collection,
		"index_name": indexName,
	})
	return err
}

// Search performs a vector similarity search
func (v *VectorClient) Search(collection string, vector []float64, limit int, filter map[string]interface{}) ([]interface{}, error) {
	args := map[string]interface{}{
		"database":   v.client.database,
		"collection": collection,
		"vector":     vector,
		"limit":      limit,
	}
	if filter != nil {
		args["filter"] = filter
	}
	res, err := v.client.SendCommand("vector_search", args)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// SearchByDocument performs vector search using a document's vector field
func (v *VectorClient) SearchByDocument(collection, docKey, field string, limit int, filter map[string]interface{}) ([]interface{}, error) {
	args := map[string]interface{}{
		"database":   v.client.database,
		"collection": collection,
		"doc_key":    docKey,
		"field":      field,
		"limit":      limit,
	}
	if filter != nil {
		args["filter"] = filter
	}
	res, err := v.client.SendCommand("vector_search_by_doc", args)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// Quantize quantizes vectors for storage efficiency
func (v *VectorClient) Quantize(collection, indexName string, quantization string) error {
	_, err := v.client.SendCommand("vector_quantize", map[string]interface{}{
		"database":     v.client.database,
		"collection":   collection,
		"index_name":   indexName,
		"quantization": quantization,
	})
	return err
}

// Dequantize restores full precision vectors
func (v *VectorClient) Dequantize(collection, indexName string) error {
	_, err := v.client.SendCommand("vector_dequantize", map[string]interface{}{
		"database":   v.client.database,
		"collection": collection,
		"index_name": indexName,
	})
	return err
}

// GetIndexInfo returns information about a vector index
func (v *VectorClient) GetIndexInfo(collection, indexName string) (map[string]interface{}, error) {
	res, err := v.client.SendCommand("vector_index_info", map[string]interface{}{
		"database":   v.client.database,
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
