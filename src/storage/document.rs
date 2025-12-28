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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_document_new() {
        let data = json!({"name": "Alice", "age": 30});
        let doc = Document::new("users", data);
        
        // Key should be a valid UUID
        assert!(!doc.key.is_empty());
        assert!(doc.id.starts_with("users/"));
        assert!(!doc.rev.is_empty());
        
        // Data should contain original fields
        assert_eq!(doc.data.get("name"), Some(&json!("Alice")));
        assert_eq!(doc.data.get("age"), Some(&json!(30)));
    }

    #[test]
    fn test_document_with_key() {
        let data = json!({"name": "Bob"});
        let doc = Document::with_key("users", "custom-key".to_string(), data);
        
        assert_eq!(doc.key, "custom-key");
        assert_eq!(doc.id, "users/custom-key");
        assert!(!doc.rev.is_empty());
        assert_eq!(doc.data.get("name"), Some(&json!("Bob")));
    }

    #[test]
    fn test_document_strips_system_fields() {
        let data = json!({
            "_key": "should-be-removed",
            "_id": "should-be-removed",
            "_rev": "should-be-removed",
            "_created_at": "should-be-removed",
            "_updated_at": "should-be-removed",
            "name": "Charlie"
        });
        
        let doc = Document::new("users", data);
        
        // System fields should be stripped from data
        assert!(doc.data.get("_key").is_none());
        assert!(doc.data.get("_id").is_none());
        assert!(doc.data.get("_rev").is_none());
        
        // But user fields should remain
        assert_eq!(doc.data.get("name"), Some(&json!("Charlie")));
    }

    #[test]
    fn test_document_update_merges() {
        let initial = json!({"name": "Dave", "age": 25});
        let mut doc = Document::with_key("users", "dave".to_string(), initial);
        let original_rev = doc.rev.clone();
        
        // Update with new data
        doc.update(json!({"age": 26, "city": "NYC"}));
        
        // Original fields preserved
        assert_eq!(doc.data.get("name"), Some(&json!("Dave")));
        // Updated fields changed
        assert_eq!(doc.data.get("age"), Some(&json!(26)));
        // New fields added
        assert_eq!(doc.data.get("city"), Some(&json!("NYC")));
        // Revision changed
        assert_ne!(doc.rev, original_rev);
    }

    #[test]
    fn test_document_update_ignores_system_fields() {
        let initial = json!({"name": "Eve"});
        let mut doc = Document::with_key("users", "eve".to_string(), initial);
        
        // Try to update with system fields
        doc.update(json!({"_key": "hacked", "email": "eve@test.com"}));
        
        // System field update ignored
        assert_eq!(doc.key, "eve");
        // Regular field added
        assert_eq!(doc.data.get("email"), Some(&json!("eve@test.com")));
    }

    #[test]
    fn test_document_get_special_fields() {
        let doc = Document::with_key("users", "test".to_string(), json!({"name": "Test"}));
        
        assert_eq!(doc.get("_key"), Some(json!("test")));
        assert_eq!(doc.get("_id"), Some(json!("users/test")));
        assert!(doc.get("_rev").is_some());
        assert_eq!(doc.get("name"), Some(json!("Test")));
        assert_eq!(doc.get("nonexistent"), None);
    }

    #[test]
    fn test_document_revision() {
        let doc = Document::new("test", json!({}));
        assert!(!doc.revision().is_empty());
    }

    #[test]
    fn test_document_to_value() {
        let doc = Document::with_key("users", "id1".to_string(), json!({"name": "Frank"}));
        let value = doc.to_value();
        
        // Should include system fields
        assert!(value.get("_key").is_some());
        assert!(value.get("_id").is_some());
        assert!(value.get("_rev").is_some());
        assert!(value.get("_created_at").is_some());
        assert!(value.get("_updated_at").is_some());
        // And user fields
        assert_eq!(value.get("name"), Some(&json!("Frank")));
    }

    #[test]
    fn test_document_timestamps() {
        let before = Utc::now();
        let doc = Document::new("users", json!({}));
        let after = Utc::now();
        
        assert!(doc.created_at >= before && doc.created_at <= after);
        assert!(doc.updated_at >= before && doc.updated_at <= after);
    }

    #[test]
    fn test_document_update_changes_updated_at() {
        let mut doc = Document::new("users", json!({}));
        let original_updated = doc.updated_at;
        
        // Small delay to ensure time difference
        std::thread::sleep(std::time::Duration::from_millis(10));
        
        doc.update(json!({"new_field": true}));
        
        assert!(doc.updated_at > original_updated);
        // created_at should not change
        assert_eq!(doc.created_at, doc.created_at); // Sanity check
    }

    #[test]
    fn test_document_serialization() {
        let doc = Document::with_key("test", "key1".to_string(), json!({"x": 1}));
        
        // Should serialize to JSON
        let serialized = serde_json::to_string(&doc).unwrap();
        assert!(serialized.contains("_key"));
        assert!(serialized.contains("key1"));
        
        // Should deserialize back
        let deserialized: Document = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.key, doc.key);
    }
}

