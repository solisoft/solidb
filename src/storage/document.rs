use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Represents a JSON document in the database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// Unique key for the document
    #[serde(rename = "_key")]
    pub key: String,

    /// Full document ID (collection/key)
    #[serde(rename = "_id")]
    pub id: String,

    /// Creation timestamp
    #[serde(rename = "_created_at")]
    pub created_at: DateTime<Utc>,

    /// Last update timestamp
    #[serde(rename = "_updated_at")]
    pub updated_at: DateTime<Utc>,

    /// The actual document data
    #[serde(flatten)]
    pub data: Value,
}

impl Document {
    /// Create a new document with auto-generated key
    pub fn new(collection_name: &str, data: Value) -> Self {
        let key = Uuid::new_v4().to_string();
        let id = format!("{}/{}", collection_name, key);
        let now = Utc::now();

        Self {
            key,
            id,
            created_at: now,
            updated_at: now,
            data,
        }
    }

    /// Create a document with a specific key
    pub fn with_key(collection_name: &str, key: String, data: Value) -> Self {
        let id = format!("{}/{}", collection_name, key);
        let now = Utc::now();

        Self {
            key,
            id,
            created_at: now,
            updated_at: now,
            data,
        }
    }

    /// Update the document data (merges with existing data)
    pub fn update(&mut self, data: Value) {
        // Merge new data with existing data
        if let (Some(existing), Some(new)) = (self.data.as_object_mut(), data.as_object()) {
            for (key, value) in new {
                existing.insert(key.clone(), value.clone());
            }
        } else {
            // If not objects, replace entirely
            self.data = data;
        }
        self.updated_at = Utc::now();
    }

    /// Get a field from the document
    pub fn get(&self, field: &str) -> Option<Value> {
        // Handle special fields
        match field {
            "_key" => Some(Value::String(self.key.clone())),
            "_id" => Some(Value::String(self.id.clone())),
            _ => self.data.get(field).cloned(),
        }
    }

    /// Convert to JSON value including metadata
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}
