use crate::error::{DbError, DbResult};
use crate::storage::document::Document;
use serde::{Deserialize, Serialize};

pub const DOC_FORMAT_VERSION: u8 = 2;

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
        // Use MessagePack for data field - ~20-30% smaller than JSON
        let data_bytes = rmp_serde::to_vec(&doc.data).unwrap_or_default();
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
        // Try MessagePack first (new format), fall back to JSON (legacy)
        let data: serde_json::Value = rmp_serde::from_slice(&self.data)
            .or_else(|_| serde_json::from_slice(&self.data))
            .unwrap_or_default();
        Document {
            key: self.key.clone(),
            id: self.id.clone(),
            rev: self.rev.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            data,
        }
    }

    pub fn into_doc(self) -> Document {
        // Try MessagePack first (new format), fall back to JSON (legacy)
        let data: serde_json::Value = rmp_serde::from_slice(&self.data)
            .or_else(|_| serde_json::from_slice(&self.data))
            .unwrap_or_default();
        Document {
            key: self.key,
            id: self.id,
            rev: self.rev,
            created_at: self.created_at,
            updated_at: self.updated_at,
            data,
        }
    }
}

// Static field names to avoid repeated heap allocations
const KEY_FIELD: &str = "_key";
const ID_FIELD: &str = "_id";
const REV_FIELD: &str = "_rev";
const CREATED_AT_FIELD: &str = "_created_at";
const UPDATED_AT_FIELD: &str = "_updated_at";

/// [`DocumentWithVersion`] as stored, borrowed from the RocksDB value. Bincode
/// is positional, so the fields mirror that struct's order; the timestamps
/// are the strings chrono's `Serialize` wrote.
#[derive(Deserialize)]
///
/// The five system strings are read as raw bytes (bincode writes a string and
/// a byte slice identically, length then bytes) and validated only when a
/// caller uses them: a projected read that returns none of them used to
/// UTF-8-check all five per document, ~4% of an uncached 50-row read.
struct StoredDocRef<'a> {
    #[allow(dead_code)]
    version: u8,
    #[serde(with = "serde_bytes")]
    key: &'a [u8],
    #[serde(with = "serde_bytes")]
    id: &'a [u8],
    #[serde(with = "serde_bytes")]
    rev: &'a [u8],
    #[serde(with = "serde_bytes")]
    created_at: &'a [u8],
    #[serde(with = "serde_bytes")]
    updated_at: &'a [u8],
    #[serde(with = "serde_bytes")]
    data: &'a [u8],
}

impl StoredDocRef<'_> {
    /// A system field's API value, or `None` if its bytes are not UTF-8
    /// (the caller then takes the checked decode, which reports it).
    fn system_field(&self, name: &str) -> Option<Option<serde_json::Value>> {
        let text = |b: &[u8]| std::str::from_utf8(b).ok().map(str::to_owned);
        let stamp = |b: &[u8]| std::str::from_utf8(b).ok().map(stored_timestamp_to_rfc3339);
        Some(Some(
            match name {
                KEY_FIELD => text(self.key)?,
                ID_FIELD => text(self.id)?,
                REV_FIELD => text(self.rev)?,
                CREATED_AT_FIELD => stamp(self.created_at)?,
                UPDATED_AT_FIELD => stamp(self.updated_at)?,
                _ => return Some(None),
            }
            .into(),
        ))
    }
}

/// The API form of a stored timestamp, `DateTime::to_rfc3339`, without
/// parsing it.
///
/// chrono serializes a `DateTime<Utc>` with `write_rfc3339(.., AutoSi,
/// use_z: true)` and `to_rfc3339` calls the same writer with `use_z: false`:
/// for a UTC offset the two differ only in the suffix, `Z` against `+00:00`.
/// Parsing and re-formatting two dates per document was ~6% of an uncached
/// 50-row scan. Anything else still goes through chrono.
fn stored_timestamp_to_rfc3339(stored: &str) -> String {
    if let Some(base) = stored.strip_suffix('Z') {
        let mut out = String::with_capacity(base.len() + 6);
        out.push_str(base);
        out.push_str("+00:00");
        return out;
    }
    chrono::DateTime::parse_from_rfc3339(stored)
        .map(|dt| dt.with_timezone(&chrono::Utc).to_rfc3339())
        .unwrap_or_else(|_| stored.to_string())
}

/// A map key borrowed from the input when the format allows it (serde's own
/// `Cow<str>` impl always allocates).
struct MapKey<'de>(std::borrow::Cow<'de, str>);

impl<'de> Deserialize<'de> for MapKey<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct KeyVisitor;
        impl<'de> serde::de::Visitor<'de> for KeyVisitor {
            type Value = MapKey<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a string key")
            }
            fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E> {
                Ok(MapKey(std::borrow::Cow::Borrowed(v)))
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
                Ok(MapKey(std::borrow::Cow::Owned(v.to_owned())))
            }
            fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
                Ok(MapKey(std::borrow::Cow::Owned(v)))
            }
        }
        d.deserialize_str(KeyVisitor)
    }
}

/// Decodes a MessagePack map keeping only the keys in `fields`; the other
/// values are skipped without being built.
struct ProjectedMap<'f> {
    fields: &'f [String],
}

impl<'de> serde::de::DeserializeSeed<'de> for ProjectedMap<'_> {
    type Value = serde_json::Map<String, serde_json::Value>;

    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_map(self)
    }
}

impl<'de> serde::de::Visitor<'de> for ProjectedMap<'_> {
    type Value = serde_json::Map<String, serde_json::Value>;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a document map")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut out = serde_json::Map::new();
        while let Some(MapKey(key)) = map.next_key()? {
            if self.fields.iter().any(|f| f.as_str() == key.as_ref()) {
                out.insert(key.into_owned(), map.next_value()?);
            } else {
                map.next_value::<serde::de::IgnoredAny>()?;
            }
        }
        Ok(out)
    }
}

/// [`deserialize_doc_as_value`] restricted to `fields` (top-level data fields
/// and system fields alike). Same values for the fields it keeps; anything
/// the fast path cannot read falls back to the full decode and drops the rest.
pub fn deserialize_doc_projected(bytes: &[u8], fields: &[String]) -> DbResult<serde_json::Value> {
    if bytes.first() == Some(&2) {
        if let Ok(stored) = bincode::deserialize::<StoredDocRef>(&bytes[1..]) {
            let mut de = rmp_serde::Deserializer::from_read_ref(stored.data);
            if let Ok(mut map) =
                serde::de::DeserializeSeed::deserialize(ProjectedMap { fields }, &mut de)
            {
                // System fields last, overriding a same-named data field, as
                // the full decode does.
                let mut valid = true;
                for field in fields {
                    match stored.system_field(field) {
                        Some(Some(value)) => {
                            map.insert(field.clone(), value);
                        }
                        Some(None) => {}
                        None => valid = false,
                    }
                }
                if valid {
                    return Ok(serde_json::Value::Object(map));
                }
            }
        }
    }
    let value = deserialize_doc_as_value(bytes)?;
    Ok(match value {
        serde_json::Value::Object(mut map) => {
            map.retain(|k, _| fields.iter().any(|f| f == k));
            serde_json::Value::Object(map)
        }
        other => other,
    })
}

/// Deserialize directly to a serde_json::Value, skipping the intermediate Document allocation.
/// Used by scan_values for the fast query path.
pub fn deserialize_doc_as_value(bytes: &[u8]) -> DbResult<serde_json::Value> {
    if bytes.is_empty() {
        return Err(DbError::DocumentNotFound("empty bytes".to_string()));
    }

    match bytes[0] {
        2 => {
            // Borrowed view: no timestamp parse, no intermediate Strings.
            if let Ok(stored) = bincode::deserialize::<StoredDocRef>(&bytes[1..]) {
                let data: serde_json::Value =
                    rmp_serde::from_slice(stored.data).unwrap_or_default();
                if let serde_json::Value::Object(mut map) = data {
                    let mut valid = true;
                    for name in [
                        KEY_FIELD,
                        ID_FIELD,
                        REV_FIELD,
                        CREATED_AT_FIELD,
                        UPDATED_AT_FIELD,
                    ] {
                        match stored.system_field(name) {
                            Some(Some(value)) => {
                                map.insert(name.to_owned(), value);
                            }
                            _ => valid = false,
                        }
                    }
                    if valid {
                        return Ok(serde_json::Value::Object(map));
                    }
                }
            }
            let dwv: DocumentWithVersion = bincode::deserialize(&bytes[1..])
                .map_err(|e| DbError::InternalError(format!("Deserialization failed: {}", e)))?;
            // V2 format always uses MessagePack for data - no JSON fallback needed
            let data: serde_json::Value = rmp_serde::from_slice(&dwv.data).unwrap_or_default();
            if let serde_json::Value::Object(mut map) = data {
                map.insert(KEY_FIELD.to_owned(), serde_json::Value::String(dwv.key));
                map.insert(ID_FIELD.to_owned(), serde_json::Value::String(dwv.id));
                map.insert(REV_FIELD.to_owned(), serde_json::Value::String(dwv.rev));
                map.insert(
                    CREATED_AT_FIELD.to_owned(),
                    serde_json::Value::String(dwv.created_at.to_rfc3339()),
                );
                map.insert(
                    UPDATED_AT_FIELD.to_owned(),
                    serde_json::Value::String(dwv.updated_at.to_rfc3339()),
                );
                Ok(serde_json::Value::Object(map))
            } else {
                // Fallback: build via Document
                Ok(DocumentWithVersion {
                    version: 2,
                    key: dwv.key,
                    id: dwv.id,
                    rev: dwv.rev,
                    created_at: dwv.created_at,
                    updated_at: dwv.updated_at,
                    data: Vec::new(),
                }
                .into_doc()
                .into_value())
            }
        }
        _ => {
            // Legacy formats: go through Document
            deserialize_doc(bytes).map(|doc| doc.into_value())
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
        2 => bincode::deserialize::<DocumentWithVersion>(&bytes[1..])
            .map_err(|e| DbError::InternalError(format!("Deserialization failed: {}", e)))
            .map(|doc_with_version| doc_with_version.into_doc()),
        1 => {
            // Version 1: Legacy format with JSON in data field
            bincode::deserialize::<DocumentWithVersion>(&bytes[1..])
                .map_err(|e| {
                    DbError::InternalError(format!("Legacy v1 deserialization failed: {}", e))
                })
                .map(|doc_with_version| {
                    // For v1, data field contains JSON - handle in to_doc_legacy_v1
                    to_doc_legacy_v1(&doc_with_version)
                })
        }
        _ => {
            // Pre-versioned format: pure JSON
            let doc: Document = serde_json::from_slice(bytes).map_err(|e| {
                DbError::InternalError(format!("Legacy JSON deserialization failed: {}", e))
            })?;
            Ok(doc)
        }
    }
}

/// Convert DocumentWithVersion (v1 format with JSON data) to Document
fn to_doc_legacy_v1(doc_with_version: &DocumentWithVersion) -> Document {
    // Version 1 used JSON for the data field
    let data: serde_json::Value =
        serde_json::from_slice(&doc_with_version.data).unwrap_or_default();
    Document {
        key: doc_with_version.key.clone(),
        id: doc_with_version.id.clone(),
        rev: doc_with_version.rev.clone(),
        created_at: doc_with_version.created_at,
        updated_at: doc_with_version.updated_at,
        data,
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
        let optimized_bytes = serialize_doc(&doc).unwrap();

        // Compare data field only (JSON vs MessagePack)
        let data_json_bytes = serde_json::to_vec(&doc.data).unwrap();
        let data_msgpack_bytes = rmp_serde::to_vec(&doc.data).unwrap();

        println!("Full JSON size: {} bytes", json_bytes.len());
        println!("Optimized size: {} bytes", optimized_bytes.len());
        println!("Data field JSON size: {} bytes", data_json_bytes.len());
        println!(
            "Data field MessagePack size: {} bytes",
            data_msgpack_bytes.len()
        );
        println!(
            "MessagePack data reduction: {:.1}%",
            100.0 * (1.0 - data_msgpack_bytes.len() as f64 / data_json_bytes.len() as f64)
        );
        println!(
            "Overall size reduction: {:.1}%",
            100.0 * (1.0 - optimized_bytes.len() as f64 / json_bytes.len() as f64)
        );

        assert!(
            data_msgpack_bytes.len() < data_json_bytes.len(),
            "MessagePack should be smaller than JSON for data field"
        );
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
    fn test_legacy_v1_format_compatibility() {
        // Test that version 1 documents (with JSON data field) can still be read
        let doc = create_test_doc();

        // Manually create a v1 format document (JSON in data field)
        let data_bytes = serde_json::to_vec(&doc.data).unwrap();
        let doc_v1 = DocumentWithVersion {
            version: 1, // Version 1 uses JSON
            key: doc.key.clone(),
            id: doc.id.clone(),
            rev: doc.rev.clone(),
            created_at: doc.created_at,
            updated_at: doc.updated_at,
            data: data_bytes,
        };

        let mut bytes_v1 = vec![1u8]; // Version byte
        bincode::serialize_into(&mut bytes_v1, &doc_v1).unwrap();

        assert!(needs_migration(&bytes_v1));

        let deserialized = deserialize_doc(&bytes_v1).unwrap();

        assert_eq!(doc.key, deserialized.key);
        assert_eq!(doc.data, deserialized.data);
        assert_eq!(doc.id, deserialized.id);
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
                            "float": 3.5,
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

    /// The borrowed fast path must produce exactly what the `Document` path
    /// produces, for every sub-second precision chrono can emit.
    #[test]
    fn fast_value_path_matches_document_path() {
        use chrono::{TimeZone, Timelike};
        for nanos in [0u32, 5_000_000, 123_456_000, 123_456_789, 1] {
            let mut doc = create_test_doc();
            let ts = chrono::Utc
                .with_ymd_and_hms(2026, 9, 28, 8, 15, 6)
                .unwrap()
                .with_nanosecond(nanos)
                .unwrap();
            doc.created_at = ts;
            doc.updated_at = ts + chrono::Duration::seconds(1);
            let bytes = serialize_doc(&doc).unwrap();
            let fast = deserialize_doc_as_value(&bytes).unwrap();
            let slow = deserialize_doc(&bytes).unwrap().into_value();
            assert_eq!(fast, slow, "nanos = {nanos}");
            assert_eq!(fast["_created_at"], ts.to_rfc3339());
        }
    }

    /// A projected decode keeps exactly the requested fields, with the same
    /// values as the full decode.
    #[test]
    fn projected_decode_matches_full_decode() {
        let doc = create_test_doc();
        let bytes = serialize_doc(&doc).unwrap();
        let full = deserialize_doc_as_value(&bytes).unwrap();
        let fields: Vec<String> = ["name", "tags", "_key", "_created_at", "absent"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let projected = deserialize_doc_projected(&bytes, &fields).unwrap();
        let obj = projected.as_object().unwrap();
        assert_eq!(obj.len(), 4, "{projected}");
        for f in ["name", "tags", "_key", "_created_at"] {
            assert_eq!(obj[f], full[f], "{f}");
        }
        assert!(obj.get("absent").is_none());
    }
}
