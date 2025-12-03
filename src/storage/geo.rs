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
            let lon = obj.get("lon").or(obj.get("lng")).or(obj.get("longitude"))?.as_f64()?;
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
