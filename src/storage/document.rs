use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Represents a JSON document in the database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// Unique key for the document
    #[serde(rename = "_key")]
    pub key: String,

    /// Full document ID (collection/key)
    #[serde(rename = "_id")]
    pub id: String,

    /// Revision for optimistic concurrency control
    #[serde(rename = "_rev")]
    pub rev: String,

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
    /// Generate a new revision ID
    fn generate_rev() -> String {
        Uuid::new_v4().to_string()
    }

    /// Create a new document with auto-generated key
    pub fn new(collection_name: &str, mut data: Value) -> Self {
        let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let id = format!("{}/{}", collection_name, key);
        let now = Utc::now();

        // Remove system fields to prevent duplication
        if let Some(obj) = data.as_object_mut() {
            obj.remove("_key");
            obj.remove("_id");
            obj.remove("_rev");
            obj.remove("_created_at");
            obj.remove("_updated_at");
        }

        Self {
            key,
            id,
            rev: Self::generate_rev(),
            created_at: now,
            updated_at: now,
            data,
        }
    }

    /// Create a document with a specific key
    pub fn with_key(collection_name: &str, key: String, mut data: Value) -> Self {
        let id = format!("{}/{}", collection_name, key);
        let now = Utc::now();

        // Remove system fields to prevent duplication
        if let Some(obj) = data.as_object_mut() {
            obj.remove("_key");
            obj.remove("_id");
            obj.remove("_rev");
            obj.remove("_created_at");
            obj.remove("_updated_at");
        }

        Self {
            key,
            id,
            rev: Self::generate_rev(),
            created_at: now,
            updated_at: now,
            data,
        }
    }

    /// Update the document data (merges with existing data)
    /// Generates a new revision on every update
    pub fn update(&mut self, data: Value) {
        // Merge new data with existing data
        if let (Some(existing), Some(new)) = (self.data.as_object_mut(), data.as_object()) {
            for (key, value) in new {
                if !key.starts_with('_') {
                    existing.insert(key.clone(), value.clone());
                }
            }
        } else {
            // If not objects, replace entirely but ensure no system fields if it becomes an object
            let mut new_data = data;
            if let Some(obj) = new_data.as_object_mut() {
                obj.remove("_key");
                obj.remove("_id");
                obj.remove("_rev");
                obj.remove("_created_at");
                obj.remove("_updated_at");
            }
            self.data = new_data;
        }
        self.rev = Self::generate_rev();
        self.updated_at = Utc::now();
    }

    /// Get the current revision
    pub fn revision(&self) -> &str {
        &self.rev
    }

    /// Get a field from the document
    pub fn get(&self, field: &str) -> Option<Value> {
        // Handle special fields
        match field {
            "_key" => Some(Value::String(self.key.clone())),
            "_id" => Some(Value::String(self.id.clone())),
            "_rev" => Some(Value::String(self.rev.clone())),
            _ => self.data.get(field).cloned(),
        }
    }

    /// Convert to JSON value including metadata
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}
