package solidb

// ClusterClient provides access to cluster management API
type ClusterClient struct {
	client *Client
}

// Status returns the cluster status
func (c *ClusterClient) Status() (map[string]interface{}, error) {
	res, err := c.client.SendCommand("cluster_status", nil)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// Info returns detailed cluster information
func (c *ClusterClient) Info() (map[string]interface{}, error) {
	res, err := c.client.SendCommand("cluster_info", nil)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// RemoveNode removes a node from the cluster
func (c *ClusterClient) RemoveNode(nodeID string) error {
	_, err := c.client.SendCommand("cluster_remove_node", map[string]interface{}{
		"node_id": nodeID,
	})
	return err
}

// Rebalance triggers cluster rebalancing
func (c *ClusterClient) Rebalance() error {
	_, err := c.client.SendCommand("cluster_rebalance", nil)
	return err
}

// Cleanup performs cluster cleanup operations
func (c *ClusterClient) Cleanup() error {
	_, err := c.client.SendCommand("cluster_cleanup", nil)
	return err
}

// Reshard triggers cluster resharding
func (c *ClusterClient) Reshard(numShards int) error {
	_, err := c.client.SendCommand("cluster_reshard", map[string]interface{}{
		"num_shards": numShards,
	})
	return err
}

// GetNodes returns all cluster nodes
func (c *ClusterClient) GetNodes() ([]interface{}, error) {
	res, err := c.client.SendCommand("cluster_get_nodes", nil)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// GetShards returns all shards information
func (c *ClusterClient) GetShards() ([]interface{}, error) {
	res, err := c.client.SendCommand("cluster_get_shards", nil)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}
