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

// ForeignKeyOptions are the optional parts of a foreign key.
type ForeignKeyOptions struct {
	// OnDelete is "restrict" (the default), "cascade" or "set_null".
	OnDelete string
	// By is "key" (the default) or "id": what the field holds of the parent.
	By string
	// Name of the key; the server names it when empty.
	Name string
}

// ListForeignKeys returns the foreign keys declared on a collection
// ("foreign_keys") and those of other collections that refer to it
// ("referenced_by").
func (i *IndexesClient) ListForeignKeys(collection string) (map[string]interface{}, error) {
	res, err := i.client.SendCommand("list_foreign_keys", map[string]interface{}{
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

// CreateForeignKey declares that field of collection holds the _key (or,
// with By "id", the _id) of a document of references, a collection of the
// same database, or null. It is refused when stored documents already refer
// to a missing parent: CheckForeignKey lists them. It returns the key as
// declared.
func (i *IndexesClient) CreateForeignKey(collection, field, references string, opts *ForeignKeyOptions) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":   i.client.database,
		"collection": collection,
		"field":      field,
		"references": references,
	}
	if opts != nil {
		if opts.OnDelete != "" {
			args["on_delete"] = opts.OnDelete
		}
		if opts.By != "" {
			args["by"] = opts.By
		}
		if opts.Name != "" {
			args["name"] = opts.Name
		}
	}
	res, err := i.client.SendCommand("create_foreign_key", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// DeleteForeignKey drops a foreign key by its name.
func (i *IndexesClient) DeleteForeignKey(collection, name string) error {
	_, err := i.client.SendCommand("delete_foreign_key", map[string]interface{}{
		"database":   i.client.database,
		"collection": collection,
		"name":       name,
	})
	return err
}

// CheckForeignKey returns the documents of collection whose field names a
// document that is not in references: "count" and a "sample". by is "key",
// "id" or "" for the default.
func (i *IndexesClient) CheckForeignKey(collection, field, references, by string) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":   i.client.database,
		"collection": collection,
		"field":      field,
		"references": references,
	}
	if by != "" {
		args["by"] = by
	}
	res, err := i.client.SendCommand("check_foreign_key", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// ListCounters returns the counter fields of a collection ("counters"),
// each with its "state" ("next" is the next number).
func (i *IndexesClient) ListCounters(collection string) (map[string]interface{}, error) {
	res, err := i.client.SendCommand("list_counters", map[string]interface{}{
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

// CreateCounter declares that the database numbers field on insert: each
// insert gets the next number, never twice the same, with no gap. options
// may hold "start", "reset" ("day", "week", "month", "year"), "timezone",
// "prefix", "suffix", "padding", "durable" and "node"; nil for a plain
// count from 1. An insert then leaves the field out and reads it from the
// answer. It returns the counter as declared, with its state.
func (i *IndexesClient) CreateCounter(collection, field string, options map[string]interface{}) (map[string]interface{}, error) {
	counter := map[string]interface{}{}
	for k, v := range options {
		counter[k] = v
	}
	counter["field"] = field
	res, err := i.client.SendCommand("create_counter", map[string]interface{}{
		"database":   i.client.database,
		"collection": collection,
		"counter":    counter,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// DeleteCounter stops numbering a field. The numbers given stay.
func (i *IndexesClient) DeleteCounter(collection, field string) error {
	_, err := i.client.SendCommand("delete_counter", map[string]interface{}{
		"database":   i.client.database,
		"collection": collection,
		"field":      field,
	})
	return err
}

// SetCounterNext makes next the number the next insert gets. It is refused
// when a document already holds it.
func (i *IndexesClient) SetCounterNext(collection, field string, next uint64) (map[string]interface{}, error) {
	res, err := i.client.SendCommand("set_counter_next", map[string]interface{}{
		"database":   i.client.database,
		"collection": collection,
		"field":      field,
		"next":       next,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// MoveCounter hands the numbering of field to node (in a cluster, one node
// numbers), asked of the node that numbers now. With force, it takes the
// numbering for the node this client is connected to, for when the node
// that numbers is lost; node may then be empty.
func (i *IndexesClient) MoveCounter(collection, field, node string, force bool) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":   i.client.database,
		"collection": collection,
		"field":      field,
		"force":      force,
	}
	if node != "" {
		args["node"] = node
	}
	res, err := i.client.SendCommand("move_counter", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}
