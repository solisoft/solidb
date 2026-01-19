use crate::error::{DbError, DbResult};
use crate::storage::document::Document;
use serde::{Deserialize, Serialize};

pub const DOC_FORMAT_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentWithVersion {
    pub version: u8,
    #[serde(rename = "_key")]
    pub key: String,
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(rename = "_rev")]
    pub rev: String,
    #[serde(rename = "_created_at")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(rename = "_updated_at")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

impl DocumentWithVersion {
    pub fn from_doc(doc: &Document) -> Self {
        let data_bytes = serde_json::to_vec(&doc.data).unwrap_or_default();
        Self {
            version: DOC_FORMAT_VERSION,
            key: doc.key.clone(),
            id: doc.id.clone(),
            rev: doc.rev.clone(),
            created_at: doc.created_at,
            updated_at: doc.updated_at,
            data: data_bytes,
        }
    }

    pub fn to_doc(&self) -> Document {
        let data: serde_json::Value = serde_json::from_slice(&self.data).unwrap_or_default();
        Document {
            key: self.key.clone(),
            id: self.id.clone(),
            rev: self.rev.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            data,
        }
    }
}

pub fn serialize_doc(doc: &Document) -> DbResult<Vec<u8>> {
    let doc_with_version = DocumentWithVersion::from_doc(doc);
    let mut bytes = Vec::new();
    bytes.push(DOC_FORMAT_VERSION);
    bincode::serialize_into(&mut bytes, &doc_with_version)
        .map_err(|e| DbError::InternalError(format!("Serialization failed: {}", e)))?;
    Ok(bytes)
}

pub fn deserialize_doc(bytes: &[u8]) -> DbResult<Document> {
    if bytes.is_empty() {
        return Err(DbError::DocumentNotFound("empty bytes".to_string()));
    }

    match bytes[0] {
        1 => bincode::deserialize::<DocumentWithVersion>(&bytes[1..])
            .map_err(|e| DbError::InternalError(format!("Deserialization failed: {}", e)))
            .map(|doc_with_version| doc_with_version.to_doc()),
        _ => {
            let doc: Document = serde_json::from_slice(bytes).map_err(|e| {
                DbError::InternalError(format!("Legacy JSON deserialization failed: {}", e))
            })?;
            Ok(doc)
        }
    }
}

pub fn serialize_to_json(doc: &Document) -> DbResult<Vec<u8>> {
    serde_json::to_vec(doc)
        .map_err(|e| DbError::InternalError(format!("JSON serialization failed: {}", e)))
}

pub fn deserialize_from_json(bytes: &[u8]) -> DbResult<Document> {
    serde_json::from_slice(bytes)
        .map_err(|e| DbError::InternalError(format!("JSON deserialization failed: {}", e)))
}

pub fn needs_migration(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes[0] != DOC_FORMAT_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::document::Document;
    use serde_json::json;

    fn create_test_doc() -> Document {
        Document::with_key(
            "test_collection",
            "test-key-123".to_string(),
            json!({
                "name": "Alice",
                "age": 30,
                "active": true,
                "score": 98.5,
                "tags": ["user", "premium"],
                "metadata": {
                    "created_by": "admin",
                    "version": 1
                }
            }),
        )
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let doc = create_test_doc();
        let bytes = serialize_doc(&doc).unwrap();
        let deserialized = deserialize_doc(&bytes).unwrap();

        assert_eq!(doc.key, deserialized.key);
        assert_eq!(doc.id, deserialized.id);
        assert_eq!(doc.rev, deserialized.rev);
        assert_eq!(doc.data, deserialized.data);
    }

    #[test]
    fn test_serialization_size() {
        let doc = create_test_doc();

        let json_bytes = serde_json::to_vec(&doc).unwrap();
        let bincode_bytes = serialize_doc(&doc).unwrap();

        println!("JSON size: {} bytes", json_bytes.len());
        println!("Bincode size: {} bytes", bincode_bytes.len());
        println!(
            "Size reduction: {:.1}%",
            100.0 * (1.0 - bincode_bytes.len() as f64 / json_bytes.len() as f64)
        );

        assert!(bincode_bytes.len() < json_bytes.len());
    }

    #[test]
    fn test_legacy_json_migration() {
        let doc = create_test_doc();
        let json_bytes = serde_json::to_vec(&doc).unwrap();

        assert!(needs_migration(&json_bytes));

        let deserialized = deserialize_doc(&json_bytes).unwrap();

        assert_eq!(doc.key, deserialized.key);
        assert_eq!(doc.data, deserialized.data);
    }

    #[test]
    fn test_current_format_no_migration() {
        let doc = create_test_doc();
        let bincode_bytes = serialize_doc(&doc).unwrap();

        assert!(!needs_migration(&bincode_bytes));
    }

    #[test]
    fn test_empty_bytes_error() {
        let result = deserialize_doc(&[]);
        assert!(result.is_err());
        if let Err(DbError::DocumentNotFound(msg)) = result {
            assert_eq!(msg, "empty bytes");
        } else {
            panic!("Expected DocumentNotFound error");
        }
    }

    #[test]
    fn test_complex_document_roundtrip() {
        let complex_data = json!({
            "nested": {
                "deeply": {
                    "nested": {
                        "value": "string",
                        "number": 42,
                        "float": 3.14159,
                        "bool": false,
                        "null": null
                    }
                }
            },
            "array_of_objects": [
                {"id": 1, "name": "first"},
                {"id": 2, "name": "second"},
                {"id": 3, "name": "third"}
            ],
            "mixed_array": [1, "two", 3.0, true, null]
        });

        let doc = Document::with_key("test", "complex".to_string(), complex_data);
        let bytes = serialize_doc(&doc).unwrap();
        let deserialized = deserialize_doc(&bytes).unwrap();

        assert_eq!(doc.key, deserialized.key);
        assert_eq!(doc.data, deserialized.data);
    }

    #[test]
    fn test_special_characters() {
        let doc = Document::with_key(
            "test",
            "special".to_string(),
            json!({
                "unicode": "Hello 世界 🌍 Café",
                "quotes": "He said \"Hello\"",
                "newlines": "Line1\nLine2\r\nLine3",
                "tabs": "Col1\tCol2"
            }),
        );

        let bytes = serialize_doc(&doc).unwrap();
        let deserialized = deserialize_doc(&bytes).unwrap();

        assert_eq!(doc.data, deserialized.data);
    }
}
