package solidb

// ColumnarClient provides access to columnar storage management API
type ColumnarClient struct {
	client *Client
}

// Create creates a new columnar table
func (c *ColumnarClient) Create(name string, columns []map[string]interface{}) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database": c.client.database,
		"name":     name,
		"columns":  columns,
	}
	res, err := c.client.SendCommand("create_columnar_table", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// List returns all columnar tables
func (c *ColumnarClient) List() ([]interface{}, error) {
	res, err := c.client.SendCommand("list_columnar_tables", map[string]interface{}{
		"database": c.client.database,
	})
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// Get retrieves a columnar table by name
func (c *ColumnarClient) Get(name string) (map[string]interface{}, error) {
	res, err := c.client.SendCommand("get_columnar_table", map[string]interface{}{
		"database": c.client.database,
		"name":     name,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Delete removes a columnar table
func (c *ColumnarClient) Delete(name string) error {
	_, err := c.client.SendCommand("delete_columnar_table", map[string]interface{}{
		"database": c.client.database,
		"name":     name,
	})
	return err
}

// Insert inserts rows into a columnar table
func (c *ColumnarClient) Insert(name string, rows []map[string]interface{}) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database": c.client.database,
		"name":     name,
		"rows":     rows,
	}
	res, err := c.client.SendCommand("columnar_insert", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Query executes a query on a columnar table
func (c *ColumnarClient) Query(name, query string, params map[string]interface{}) ([]interface{}, error) {
	args := map[string]interface{}{
		"database": c.client.database,
		"name":     name,
		"query":    query,
	}
	if params != nil {
		args["params"] = params
	}
	res, err := c.client.SendCommand("columnar_query", args)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// Aggregate performs aggregation on a columnar table
func (c *ColumnarClient) Aggregate(name string, aggregation map[string]interface{}) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":    c.client.database,
		"name":        name,
		"aggregation": aggregation,
	}
	res, err := c.client.SendCommand("columnar_aggregate", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// CreateIndex creates an index on a columnar table
func (c *ColumnarClient) CreateIndex(tableName, indexName, column string, indexType *string) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":   c.client.database,
		"table_name": tableName,
		"index_name": indexName,
		"column":     column,
	}
	if indexType != nil {
		args["index_type"] = *indexType
	}
	res, err := c.client.SendCommand("columnar_create_index", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// ListIndexes returns all indexes on a columnar table
func (c *ColumnarClient) ListIndexes(tableName string) ([]interface{}, error) {
	res, err := c.client.SendCommand("columnar_list_indexes", map[string]interface{}{
		"database":   c.client.database,
		"table_name": tableName,
	})
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// DeleteIndex deletes an index from a columnar table
func (c *ColumnarClient) DeleteIndex(tableName, indexName string) error {
	_, err := c.client.SendCommand("columnar_delete_index", map[string]interface{}{
		"database":   c.client.database,
		"table_name": tableName,
		"index_name": indexName,
	})
	return err
}

// AddColumn adds a column to a columnar table
func (c *ColumnarClient) AddColumn(tableName, columnName, columnType string, defaultValue interface{}) error {
	args := map[string]interface{}{
		"database":    c.client.database,
		"table_name":  tableName,
		"column_name": columnName,
		"column_type": columnType,
	}
	if defaultValue != nil {
		args["default_value"] = defaultValue
	}
	_, err := c.client.SendCommand("columnar_add_column", args)
	return err
}

// DropColumn removes a column from a columnar table
func (c *ColumnarClient) DropColumn(tableName, columnName string) error {
	_, err := c.client.SendCommand("columnar_drop_column", map[string]interface{}{
		"database":    c.client.database,
		"table_name":  tableName,
		"column_name": columnName,
	})
	return err
}

// Stats returns statistics for a columnar table
func (c *ColumnarClient) Stats(tableName string) (map[string]interface{}, error) {
	res, err := c.client.SendCommand("columnar_stats", map[string]interface{}{
		"database":   c.client.database,
		"table_name": tableName,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}
