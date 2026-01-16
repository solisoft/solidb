package solidb

// GeoClient provides access to geo-spatial index and query API
type GeoClient struct {
	client *Client
}

// CreateIndex creates a geo-spatial index
func (g *GeoClient) CreateIndex(collection, name string, fields []string, geoJSON *bool) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":   g.client.database,
		"collection": collection,
		"name":       name,
		"fields":     fields,
	}
	if geoJSON != nil {
		args["geo_json"] = *geoJSON
	}
	res, err := g.client.SendCommand("create_geo_index", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

// ListIndexes returns all geo indexes in a collection
func (g *GeoClient) ListIndexes(collection string) ([]interface{}, error) {
	res, err := g.client.SendCommand("list_geo_indexes", map[string]interface{}{
		"database":   g.client.database,
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

// DeleteIndex deletes a geo-spatial index
func (g *GeoClient) DeleteIndex(collection, indexName string) error {
	_, err := g.client.SendCommand("delete_geo_index", map[string]interface{}{
		"database":   g.client.database,
		"collection": collection,
		"index_name": indexName,
	})
	return err
}

// Near finds documents near a point
func (g *GeoClient) Near(collection string, latitude, longitude, radius float64, limit *int) ([]interface{}, error) {
	args := map[string]interface{}{
		"database":   g.client.database,
		"collection": collection,
		"latitude":   latitude,
		"longitude":  longitude,
		"radius":     radius,
	}
	if limit != nil {
		args["limit"] = *limit
	}
	res, err := g.client.SendCommand("geo_near", args)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// Within finds documents within a polygon or circle
func (g *GeoClient) Within(collection string, geometry map[string]interface{}) ([]interface{}, error) {
	args := map[string]interface{}{
		"database":   g.client.database,
		"collection": collection,
		"geometry":   geometry,
	}
	res, err := g.client.SendCommand("geo_within", args)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

// Distance calculates distance between two points
func (g *GeoClient) Distance(lat1, lon1, lat2, lon2 float64) (float64, error) {
	res, err := g.client.SendCommand("geo_distance", map[string]interface{}{
		"lat1": lat1,
		"lon1": lon1,
		"lat2": lat2,
		"lon2": lon2,
	})
	if err != nil {
		return 0, err
	}
	if f, ok := res.(float64); ok {
		return f, nil
	}
	return 0, nil
}

// Intersects finds documents whose geometry intersects with given geometry
func (g *GeoClient) Intersects(collection string, geometry map[string]interface{}) ([]interface{}, error) {
	args := map[string]interface{}{
		"database":   g.client.database,
		"collection": collection,
		"geometry":   geometry,
	}
	res, err := g.client.SendCommand("geo_intersects", args)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}
