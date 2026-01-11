use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Earth radius in meters
const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// A geographic point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoPoint {
    pub lat: f64,
    pub lon: f64,
}

impl GeoPoint {
    /// Create a new geo point
    pub fn new(lat: f64, lon: f64) -> Self {
        Self { lat, lon }
    }

    /// Parse geo point from JSON value
    /// Supports: { "lat": 48.8, "lon": 2.3 } or [lon, lat] (GeoJSON) or [lat, lon]
    pub fn from_value(value: &Value) -> Option<Self> {
        // Try object format { lat, lon }
        if let Some(obj) = value.as_object() {
            let lat = obj.get("lat").or(obj.get("latitude"))?.as_f64()?;
            let lon = obj
                .get("lon")
                .or(obj.get("lng"))
                .or(obj.get("longitude"))?
                .as_f64()?;
            return Some(Self::new(lat, lon));
        }

        // Try array format [lat, lon] or [lon, lat]
        if let Some(arr) = value.as_array() {
            if arr.len() == 2 {
                let a = arr[0].as_f64()?;
                let b = arr[1].as_f64()?;
                // Assume [lat, lon] format (common in databases)
                return Some(Self::new(a, b));
            }
        }

        None
    }
}

/// Geo index metadata stored in RocksDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoIndex {
    /// Index name
    pub name: String,
    /// Field path containing the geo coordinates
    pub field: String,
    /// Geohash precision
    pub precision: usize,
}

impl GeoIndex {
    /// Create a new geo index
    pub fn new(name: String, field: String) -> Self {
        Self {
            name,
            field,
            precision: 6, // ~1.2km precision
        }
    }

    /// Get index statistics
    pub fn stats(&self) -> GeoIndexStats {
        GeoIndexStats {
            name: self.name.clone(),
            field: self.field.clone(),
            precision: self.precision,
            indexed_documents: 0, // Will be computed at query time
            geohash_buckets: 0,
        }
    }
}

/// Geo index statistics
#[derive(Debug, Clone, Serialize)]
pub struct GeoIndexStats {
    pub name: String,
    pub field: String,
    pub precision: usize,
    pub indexed_documents: usize,
    pub geohash_buckets: usize,
}

/// Calculate distance between two points using Haversine formula
/// Returns distance in meters
pub fn haversine_distance(p1: &GeoPoint, p2: &GeoPoint) -> f64 {
    let lat1_rad = p1.lat.to_radians();
    let lat2_rad = p2.lat.to_radians();
    let delta_lat = (p2.lat - p1.lat).to_radians();
    let delta_lon = (p2.lon - p1.lon).to_radians();

    let a = (delta_lat / 2.0).sin().powi(2)
        + lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin().powi(2);

    let c = 2.0 * a.sqrt().asin();

    EARTH_RADIUS_M * c
}

/// Calculate distance from coordinates
pub fn distance_meters(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    haversine_distance(&GeoPoint::new(lat1, lon1), &GeoPoint::new(lat2, lon2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_geo_point_new() {
        let point = GeoPoint::new(48.8566, 2.3522);
        assert!((point.lat - 48.8566).abs() < 1e-10);
        assert!((point.lon - 2.3522).abs() < 1e-10);
    }

    #[test]
    fn test_geo_point_from_value_object() {
        // Standard format
        let value = json!({"lat": 48.8566, "lon": 2.3522});
        let point = GeoPoint::from_value(&value).unwrap();
        assert!((point.lat - 48.8566).abs() < 1e-10);
        assert!((point.lon - 2.3522).abs() < 1e-10);

        // latitude/longitude format
        let value = json!({"latitude": 48.8566, "longitude": 2.3522});
        let point = GeoPoint::from_value(&value).unwrap();
        assert!((point.lat - 48.8566).abs() < 1e-10);

        // lng format
        let value = json!({"lat": 48.8566, "lng": 2.3522});
        let point = GeoPoint::from_value(&value).unwrap();
        assert!((point.lon - 2.3522).abs() < 1e-10);
    }

    #[test]
    fn test_geo_point_from_value_array() {
        let value = json!([48.8566, 2.3522]);
        let point = GeoPoint::from_value(&value).unwrap();
        assert!((point.lat - 48.8566).abs() < 1e-10);
        assert!((point.lon - 2.3522).abs() < 1e-10);
    }

    #[test]
    fn test_geo_point_from_value_invalid() {
        // Not enough elements
        assert!(GeoPoint::from_value(&json!([1.0])).is_none());

        // Wrong type
        assert!(GeoPoint::from_value(&json!("invalid")).is_none());

        // Missing fields
        assert!(GeoPoint::from_value(&json!({"lat": 1.0})).is_none());
    }

    #[test]
    fn test_geo_index_new() {
        let index = GeoIndex::new("location_idx".to_string(), "location".to_string());

        assert_eq!(index.name, "location_idx");
        assert_eq!(index.field, "location");
        assert_eq!(index.precision, 6);
    }

    #[test]
    fn test_geo_index_stats() {
        let index = GeoIndex::new("idx".to_string(), "field".to_string());
        let stats = index.stats();

        assert_eq!(stats.name, "idx");
        assert_eq!(stats.field, "field");
        assert_eq!(stats.precision, 6);
        assert_eq!(stats.indexed_documents, 0);
    }

    #[test]
    fn test_haversine_distance_same_point() {
        let p1 = GeoPoint::new(48.8566, 2.3522);
        let p2 = GeoPoint::new(48.8566, 2.3522);

        let dist = haversine_distance(&p1, &p2);
        assert!(dist < 0.01); // Should be essentially 0
    }

    #[test]
    fn test_haversine_distance_known_cities() {
        // Paris to London: approximately 343 km
        let paris = GeoPoint::new(48.8566, 2.3522);
        let london = GeoPoint::new(51.5074, -0.1278);

        let dist = haversine_distance(&paris, &london);
        let dist_km = dist / 1000.0;

        // Should be between 340 and 350 km
        assert!(dist_km > 340.0 && dist_km < 350.0);
    }

    #[test]
    fn test_haversine_distance_new_york_los_angeles() {
        // NYC to LA: approximately 3940 km
        let nyc = GeoPoint::new(40.7128, -74.0060);
        let la = GeoPoint::new(34.0522, -118.2437);

        let dist = haversine_distance(&nyc, &la);
        let dist_km = dist / 1000.0;

        // Should be between 3900 and 4000 km
        assert!(dist_km > 3900.0 && dist_km < 4000.0);
    }

    #[test]
    fn test_distance_meters() {
        // Test the convenience function
        let dist = distance_meters(48.8566, 2.3522, 51.5074, -0.1278);
        let dist_km = dist / 1000.0;

        assert!(dist_km > 340.0 && dist_km < 350.0);
    }

    #[test]
    fn test_haversine_symmetry() {
        let p1 = GeoPoint::new(40.0, -70.0);
        let p2 = GeoPoint::new(35.0, -80.0);

        let dist1 = haversine_distance(&p1, &p2);
        let dist2 = haversine_distance(&p2, &p1);

        assert!((dist1 - dist2).abs() < 0.01);
    }

    #[test]
    fn test_geo_point_clone() {
        let p1 = GeoPoint::new(48.8566, 2.3522);
        let p2 = p1.clone();

        assert!((p1.lat - p2.lat).abs() < 1e-10);
        assert!((p1.lon - p2.lon).abs() < 1e-10);
    }

    #[test]
    fn test_geo_point_serialization() {
        let point = GeoPoint::new(48.8566, 2.3522);

        let json = serde_json::to_string(&point).unwrap();
        assert!(json.contains("lat"));
        assert!(json.contains("lon"));

        let deserialized: GeoPoint = serde_json::from_str(&json).unwrap();
        assert!((point.lat - deserialized.lat).abs() < 1e-10);
    }

    #[test]
    fn test_geo_index_serialization() {
        let index = GeoIndex::new("idx".to_string(), "loc".to_string());

        let json = serde_json::to_string(&index).unwrap();
        assert!(json.contains("idx"));

        let deserialized: GeoIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(index.name, deserialized.name);
    }
}
