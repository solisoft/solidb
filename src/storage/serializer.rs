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
pub(crate) const KEY_FIELD: &str = "_key";
pub(crate) const ID_FIELD: &str = "_id";
pub(crate) const REV_FIELD: &str = "_rev";
pub(crate) const CREATED_AT_FIELD: &str = "_created_at";
pub(crate) const UPDATED_AT_FIELD: &str = "_updated_at";

/// [`DocumentWithVersion`] as stored, borrowed from the RocksDB value. Bincode
/// is positional, so the fields mirror that struct's order; the timestamps
/// are the strings chrono's `Serialize` wrote.
///
/// The five system strings are read as raw bytes (bincode writes a string and
/// a byte slice identically, length then bytes) and validated only when a
/// caller uses them: a projected read that returns none of them used to
/// UTF-8-check all five per document, ~4% of an uncached 50-row read.
#[derive(Deserialize)]
pub(crate) struct StoredDocRef<'a> {
    #[allow(dead_code)]
    version: u8,
    #[serde(with = "serde_bytes")]
    pub(crate) key: &'a [u8],
    #[serde(with = "serde_bytes")]
    pub(crate) id: &'a [u8],
    #[serde(with = "serde_bytes")]
    pub(crate) rev: &'a [u8],
    #[serde(with = "serde_bytes")]
    pub(crate) created_at: &'a [u8],
    #[serde(with = "serde_bytes")]
    pub(crate) updated_at: &'a [u8],
    /// The document's fields, MessagePack.
    #[serde(with = "serde_bytes")]
    pub(crate) data: &'a [u8],
}

impl<'a> StoredDocRef<'a> {
    /// The header of a current-format (v2) stored document; `None` for any
    /// other format.
    ///
    /// Read by hand: bincode 1's default encoding is a `u8`, then each string
    /// or byte field as a little-endian `u64` length and its bytes, and going
    /// through serde for six slices cost more than the copy it feeds on a raw
    /// scan. `StoredDocRef` derives `Deserialize` for the same layout, and a
    /// test holds the two to the same result.
    pub(crate) fn parse(bytes: &'a [u8]) -> Option<Self> {
        fn field<'b>(cur: &mut &'b [u8]) -> Option<&'b [u8]> {
            let len = u64::from_le_bytes(cur.get(..8)?.try_into().ok()?);
            let end = 8usize.checked_add(usize::try_from(len).ok()?)?;
            let value = cur.get(8..end)?;
            *cur = &cur[end..];
            Some(value)
        }
        let (&DOC_FORMAT_VERSION, rest) = bytes.split_first()? else {
            return None;
        };
        let (&version, mut cur) = rest.split_first()?;
        Some(Self {
            version,
            key: field(&mut cur)?,
            id: field(&mut cur)?,
            rev: field(&mut cur)?,
            created_at: field(&mut cur)?,
            updated_at: field(&mut cur)?,
            data: field(&mut cur)?,
        })
    }
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
pub(crate) fn stored_timestamp_to_rfc3339(stored: &str) -> String {
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
    // One pass that steps over the fields not asked for by their length.
    if let Some(value) = super::raw_rows::project(bytes, fields) {
        return Ok(value);
    }
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

/// [`DocumentWithVersion`]'s fields before `data`, borrowed. Bincode is
/// positional, so this writes the same bytes as the owned struct.
#[derive(Serialize)]
struct StoredHeader<'a> {
    version: u8,
    key: &'a str,
    id: &'a str,
    rev: &'a str,
    created_at: &'a chrono::DateTime<chrono::Utc>,
    updated_at: &'a chrono::DateTime<chrono::Utc>,
}

/// The stored form of `doc`: [`DOC_FORMAT_VERSION`], then
/// [`DocumentWithVersion`] in bincode, its `data` the document's fields in
/// MessagePack.
///
/// Written into one buffer, sized up front: the header from borrowed
/// fields, then `data`'s length and the MessagePack straight after it. It
/// used to copy the key, id and revision into a `DocumentWithVersion`,
/// encode the fields into a buffer of their own, and copy that into a
/// second one grown as it went — on every document write. The bytes are the
/// same (`serialize_doc_matches_the_struct_encoding`).
pub fn serialize_doc(doc: &Document) -> DbResult<Vec<u8>> {
    serialize_stored(
        &doc.key,
        &doc.id,
        &doc.rev,
        &doc.created_at,
        &doc.updated_at,
        |bytes| write_msgpack(bytes, &doc.data),
    )
}

/// The stored form of a document whose fields are already MessagePack, as
/// [`write_msgpack`] writes them (`storage::raw_write` builds them from a
/// client's bytes without decoding them), its timestamps already in their
/// stored form ([`stored_timestamp`]): bincode writes a `DateTime` as the
/// string its `Serialize` collects, so the bytes are the same.
pub(crate) fn serialize_doc_raw(
    key: &str,
    id: &str,
    rev: &str,
    created_at: &str,
    updated_at: &str,
    data: &[u8],
) -> DbResult<Vec<u8>> {
    let header = StoredHeaderText {
        version: DOC_FORMAT_VERSION,
        key,
        id,
        rev,
        created_at,
        updated_at,
    };
    let mut bytes = Vec::with_capacity(
        64 + key.len() + id.len() + rev.len() + created_at.len() + updated_at.len() + data.len(),
    );
    bytes.push(DOC_FORMAT_VERSION);
    bincode::serialize_into(&mut bytes, &header)
        .map_err(|e| DbError::InternalError(format!("Serialization failed: {}", e)))?;
    bytes.extend_from_slice(&(data.len() as u64).to_le_bytes());
    bytes.extend_from_slice(data);
    Ok(bytes)
}

/// The stored form of a document from its API value (the shape
/// `Document::to_value` gives: its fields and the five system attributes),
/// written without building a `Document`: the fields are encoded once and
/// the timestamps change only their offset suffix. The bytes are those of
/// `serialize_doc` on the `Document` the value describes, in `collection`
/// under `key` (`serialize_api_value_matches_the_document_path`).
///
/// `None` when the value is not an object carrying `_rev`, `_created_at`
/// and `_updated_at` as strings: the caller builds the `Document` instead.
/// A transaction stages its writes as such values; rebuilding a `Document`
/// from each (timestamps parsed, fields cloned) to encode it was a large
/// part of a commit's CPU.
pub(crate) fn serialize_api_value(
    collection: &str,
    key: &str,
    value: &serde_json::Value,
) -> DbResult<Option<Vec<u8>>> {
    let Some(map) = value.as_object() else {
        return Ok(None);
    };
    let text = |field: &str| map.get(field).and_then(serde_json::Value::as_str);
    let (Some(rev), Some(created_at), Some(updated_at)) = (
        text(REV_FIELD),
        text(CREATED_AT_FIELD),
        text(UPDATED_AT_FIELD),
    ) else {
        return Ok(None);
    };
    let (Some(created_at), Some(updated_at)) = (
        api_timestamp_to_stored(created_at),
        api_timestamp_to_stored(updated_at),
    ) else {
        return Ok(None);
    };
    let id = format!("{}/{}", collection, key);
    let fields = || {
        map.iter()
            .filter(|(name, _)| !crate::storage::document::is_system_field(name))
    };
    let mut data = Vec::with_capacity(256);
    let _ = rmp::encode::write_map_len(&mut data, fields().count() as u32);
    for (name, field) in fields() {
        let _ = rmp::encode::write_str(&mut data, name);
        write_msgpack(&mut data, field);
    }
    serialize_doc_raw(key, &id, rev, &created_at, &updated_at, &data).map(Some)
}

/// A timestamp's API form (`DateTime::to_rfc3339`) as it is stored
/// ([`stored_timestamp`]): chrono writes both with the same digits, and
/// they differ only in how they name UTC, `+00:00` against `Z`. Anything
/// else is parsed and written again; `None` if it is not RFC 3339.
fn api_timestamp_to_stored(api: &str) -> Option<String> {
    if let Some(stem) = api.strip_suffix("+00:00") {
        return Some(format!("{stem}Z"));
    }
    chrono::DateTime::parse_from_rfc3339(api)
        .ok()
        .map(|t| stored_timestamp(&t.with_timezone(&chrono::Utc)))
}

/// [`StoredHeader`] with its timestamps as the strings they serialize to.
#[derive(Serialize)]
struct StoredHeaderText<'a> {
    version: u8,
    key: &'a str,
    id: &'a str,
    rev: &'a str,
    created_at: &'a str,
    updated_at: &'a str,
}

/// A timestamp as it is stored: what chrono's `Serialize` writes for a
/// `DateTime<Utc>` (RFC 3339, fraction digits as needed, `Z`).
pub(crate) fn stored_timestamp(at: &chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
}

/// The header from borrowed fields, then `data`'s length and the bytes
/// `write_data` appends.
fn serialize_stored(
    key: &str,
    id: &str,
    rev: &str,
    created_at: &chrono::DateTime<chrono::Utc>,
    updated_at: &chrono::DateTime<chrono::Utc>,
    write_data: impl FnOnce(&mut Vec<u8>),
) -> DbResult<Vec<u8>> {
    let header = StoredHeader {
        version: DOC_FORMAT_VERSION,
        key,
        id,
        rev,
        created_at,
        updated_at,
    };
    // Two version bytes, six lengths, two timestamps of ~32 bytes, the
    // three strings, and room for a typical document's fields.
    let mut bytes = Vec::with_capacity(128 + key.len() + id.len() + rev.len() + 512);
    bytes.push(DOC_FORMAT_VERSION);
    bincode::serialize_into(&mut bytes, &header)
        .map_err(|e| DbError::InternalError(format!("Serialization failed: {}", e)))?;

    // `data`: bincode writes a byte string as its length (u64, little
    // endian) and the bytes. The length is filled in once they are written.
    let len_at = bytes.len();
    bytes.extend_from_slice(&[0u8; 8]);
    write_data(&mut bytes);
    let data_len = (bytes.len() - len_at - 8) as u64;
    bytes[len_at..len_at + 8].copy_from_slice(&data_len.to_le_bytes());
    Ok(bytes)
}

/// `value` in MessagePack, byte for byte what `rmp_serde::encode::write`
/// writes for it, without going through serde: the same `rmp` primitive per
/// node (`serde_json`'s `Serialize` maps a positive integer to `u64`, a
/// negative one to `i64` and anything else to `f64`; a map's keys are
/// strings, in the map's order). Writing to a `Vec` cannot fail.
pub(crate) fn write_msgpack(out: &mut Vec<u8>, value: &serde_json::Value) {
    use rmp::encode;
    use serde_json::Value;
    match value {
        Value::Null => {
            let _ = encode::write_nil(out);
        }
        Value::Bool(b) => {
            let _ = encode::write_bool(out, *b);
        }
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                let _ = encode::write_uint(out, u);
            } else if let Some(i) = n.as_i64() {
                let _ = encode::write_sint(out, i);
            } else if let Some(f) = n.as_f64() {
                let _ = encode::write_f64(out, f);
            }
        }
        Value::String(s) => {
            let _ = encode::write_str(out, s);
        }
        Value::Array(items) => {
            let _ = encode::write_array_len(out, items.len() as u32);
            for item in items {
                write_msgpack(out, item);
            }
        }
        Value::Object(fields) => {
            let _ = encode::write_map_len(out, fields.len() as u32);
            for (key, item) in fields {
                let _ = encode::write_str(out, key);
                write_msgpack(out, item);
            }
        }
    }
}

/// A current-format document read off its header by hand, its two
/// timestamps in the form every writer stores. `None` for anything else,
/// which the caller reads through bincode and chrono as before: that way
/// was about 3% of a transaction that reads two documents by key.
fn stored_document(bytes: &[u8]) -> Option<Document> {
    let stored = StoredDocRef::parse(bytes)?;
    let text = |b: &[u8]| std::str::from_utf8(b).ok().map(str::to_owned);
    Some(Document {
        created_at: parse_stored_timestamp(stored.created_at)?,
        updated_at: parse_stored_timestamp(stored.updated_at)?,
        data: rmp_serde::from_slice(stored.data).ok()?,
        key: text(stored.key)?,
        id: text(stored.id)?,
        rev: text(stored.rev)?,
    })
}

/// `YYYY-MM-DDTHH:MM:SS[.fraction]Z`, the form chrono serializes a
/// `DateTime<Utc>` in. `None` for any other spelling (an offset, a leap
/// second, a year outside four digits), which chrono parses.
fn parse_stored_timestamp(b: &[u8]) -> Option<chrono::DateTime<chrono::Utc>> {
    fn number(b: &[u8]) -> Option<u32> {
        b.iter().try_fold(0u32, |n, &c| {
            c.is_ascii_digit().then(|| n * 10 + u32::from(c - b'0'))
        })
    }
    let (&b'Z', b) = b.split_last()? else {
        return None;
    };
    if b.len() < 19
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let nanos = match &b[19..] {
        [] => 0,
        [b'.', fraction @ ..] if (1..=9).contains(&fraction.len()) => {
            number(fraction)? * 10u32.pow(9 - fraction.len() as u32)
        }
        _ => return None,
    };
    let date = chrono::NaiveDate::from_ymd_opt(
        number(&b[..4])? as i32,
        number(&b[5..7])?,
        number(&b[8..10])?,
    )?;
    let second = number(&b[17..19])?;
    if second > 59 {
        return None;
    }
    let time = date.and_hms_nano_opt(number(&b[11..13])?, number(&b[14..16])?, second, nanos)?;
    Some(chrono::DateTime::from_naive_utc_and_offset(
        time,
        chrono::Utc,
    ))
}

pub fn deserialize_doc(bytes: &[u8]) -> DbResult<Document> {
    if bytes.is_empty() {
        return Err(DbError::DocumentNotFound("empty bytes".to_string()));
    }

    match bytes[0] {
        2 => {
            if let Some(doc) = stored_document(bytes) {
                return Ok(doc);
            }
            bincode::deserialize::<DocumentWithVersion>(&bytes[1..])
                .map_err(|e| DbError::InternalError(format!("Deserialization failed: {}", e)))
                .map(|doc_with_version| doc_with_version.into_doc())
        }
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

    /// What `stage_transaction_operations` wrote before (`txn_document`,
    /// then `serialize_doc`), against `serialize_api_value` on the same
    /// staged value: every kind of field, timestamps with and without a
    /// fraction, and after an update.
    /// The hand-written document read gives what bincode and chrono give,
    /// and declines whatever it does not read the same way.
    #[test]
    fn a_stored_document_reads_by_hand_as_through_bincode() {
        let stamps = [
            "2026-11-09T19:40:13.635839364Z",
            "2026-11-09T19:40:13Z",
            "2026-11-09T19:40:13.100Z",
            "2026-11-09T19:40:13.000001Z",
            "1970-01-01T00:00:00Z",
            "2024-02-29T23:59:59.999999999Z",
        ];
        for stamp in stamps {
            let t = chrono::DateTime::parse_from_rfc3339(stamp)
                .unwrap()
                .with_timezone(&chrono::Utc);
            assert_eq!(parse_stored_timestamp(stamp.as_bytes()), Some(t), "{stamp}");
            let mut doc = Document::with_key(
                "coll",
                "k1".to_string(),
                json!({"n": 1, "s": "é", "arr": [1, {"x": null}], "obj": {"k": -1}}),
            );
            doc.created_at = t;
            doc.updated_at = t;
            let bytes = serialize_doc(&doc).unwrap();
            let by_hand = stored_document(&bytes).expect("a current-format document");
            let through_bincode = bincode::deserialize::<DocumentWithVersion>(&bytes[1..])
                .unwrap()
                .into_doc();
            assert_eq!(by_hand.to_value(), through_bincode.to_value());
            assert_eq!(by_hand.created_at, through_bincode.created_at);
            assert_eq!(by_hand.updated_at, through_bincode.updated_at);
            assert_eq!(deserialize_doc(&bytes).unwrap().to_value(), doc.to_value());
        }
        // Spellings chrono reads and the hand parser leaves to it, or that
        // are no date at all.
        for other in [
            "2026-11-09T19:40:13+00:00",
            "2026-11-09T19:40:60Z",
            "2026-11-09 19:40:13Z",
            "2026-02-30T19:40:13Z",
            "2026-11-09T19:40:13.Z",
            "2026-11-09T19:40:13.1234567890Z",
            "2026-11-09T24:00:00Z",
            "+12026-11-09T19:40:13Z",
            "2026-11-0xT19:40:13Z",
            "",
            "Z",
        ] {
            assert_eq!(parse_stored_timestamp(other.as_bytes()), None, "{other}");
        }
    }

    #[test]
    fn serialize_api_value_matches_the_document_path() {
        let shapes = [
            json!({"n": 1}),
            json!({}),
            json!({"neg": -7, "big": 18446744073709551615u64, "f": 1.5, "s": "é", "null": null,
                   "t": true, "arr": [1, "a", {"x": [null]}], "obj": {"k": {"z": 0, "a": -1}}}),
            json!({"_from": "v/1", "_to": "v/2", "w": 0.25}),
        ];
        for data in shapes {
            let mut doc = Document::with_key("coll", "k1".to_string(), data.clone());
            let stamps = [
                "2026-11-09T19:40:13.635839364Z",
                "2026-11-09T19:40:13Z",
                "2026-11-09T19:40:13.100Z",
            ];
            for stamp in stamps {
                let t = chrono::DateTime::parse_from_rfc3339(stamp)
                    .unwrap()
                    .with_timezone(&chrono::Utc);
                doc.created_at = t;
                doc.updated_at = t;
                for updated in [false, true] {
                    let mut d = doc.clone();
                    if updated {
                        d.update(json!({"added": [1, 2], "n": 9}));
                    }
                    let value = d.to_value();
                    // The document path, as the commit took it.
                    let mut rebuilt = Document::with_key("coll", "k1".to_string(), value.clone());
                    rebuilt.rev = d.rev.clone();
                    rebuilt.created_at = d.created_at;
                    rebuilt.updated_at = d.updated_at;
                    let expected = serialize_doc(&rebuilt).unwrap();
                    let got = serialize_api_value("coll", "k1", &value).unwrap().unwrap();
                    assert_eq!(got, expected, "{value}");
                }
            }
        }
        // Not a staged document: the caller takes the document path.
        assert!(serialize_api_value("coll", "k", &json!([1]))
            .unwrap()
            .is_none());
        assert!(serialize_api_value("coll", "k", &json!({"n": 1}))
            .unwrap()
            .is_none());
    }

    /// What `serialize_doc` wrote before it was written in one buffer.
    fn struct_encoding(doc: &Document) -> Vec<u8> {
        let mut bytes = vec![DOC_FORMAT_VERSION];
        bincode::serialize_into(&mut bytes, &DocumentWithVersion::from_doc(doc)).unwrap();
        bytes
    }

    #[test]
    fn write_msgpack_matches_rmp_serde() {
        let mut values = vec![
            json!(null),
            json!(true),
            json!(false),
            json!(0.1),
            json!(-0.0),
            json!(1.0),
            json!(1e300),
            json!(f64::MIN_POSITIVE),
            json!(""),
            json!("ünïcode ✓"),
            json!([]),
            json!({}),
            json!({"b": 1, "a": [1, {"c": null}], "": "empty key"}),
        ];
        for n in [
            0i64,
            1,
            31,
            32,
            127,
            128,
            255,
            256,
            65_535,
            65_536,
            i32::MAX as i64,
            u32::MAX as i64,
            u32::MAX as i64 + 1,
            i64::MAX,
        ] {
            values.push(json!(n));
            values.push(json!(-n));
        }
        values.push(json!(i64::MIN));
        values.push(json!(u64::MAX));
        for len in [0usize, 15, 16, 31, 32, 255, 256, 65_535, 65_536] {
            values.push(json!("s".repeat(len)));
            values.push(serde_json::Value::Array(vec![json!(1); len]));
            values.push(serde_json::Value::Object(
                (0..len).map(|i| (format!("k{i:06}"), json!(i))).collect(),
            ));
        }
        for value in values {
            let mut ours = Vec::new();
            write_msgpack(&mut ours, &value);
            assert_eq!(ours, rmp_serde::to_vec(&value).unwrap(), "{value}");
        }
    }

    /// A raw document's stored bytes, timestamps given as text, are the
    /// document's.
    #[test]
    fn serialize_doc_raw_matches_serialize_doc() {
        let mut doc = create_test_doc();
        for nanos in [0u32, 1, 120_000_000, 123_456_000, 123_456_789, 999_999_999] {
            doc.created_at = chrono::DateTime::from_timestamp(1_700_000_000, nanos).unwrap();
            doc.updated_at = chrono::Utc::now();
            let mut data = Vec::new();
            write_msgpack(&mut data, &doc.data);
            let raw = serialize_doc_raw(
                &doc.key,
                &doc.id,
                &doc.rev,
                &stored_timestamp(&doc.created_at),
                &stored_timestamp(&doc.updated_at),
                &data,
            )
            .unwrap();
            assert_eq!(raw, serialize_doc(&doc).unwrap(), "{nanos}");
            // And that is the string serde writes.
            assert_eq!(
                serde_json::to_value(doc.created_at).unwrap(),
                serde_json::Value::String(stored_timestamp(&doc.created_at))
            );
        }
    }

    #[test]
    fn serialize_doc_matches_the_struct_encoding() {
        let docs = vec![
            create_test_doc(),
            Document::with_key("c", "k".to_string(), json!({})),
            Document::with_key("c", String::new(), json!(null)),
            Document::with_key("c", "ünïcode-ключ".to_string(), json!([1, "two", 3.5])),
            Document::with_key(
                "db:c",
                "big".to_string(),
                json!({
                    "s": "x".repeat(70_000),
                    "n": [u64::MAX, i64::MIN, 0.1, -0.0],
                    "deep": {"a": {"b": {"c": [null, true, false, {}]}}},
                }),
            ),
        ];
        for doc in docs {
            let bytes = serialize_doc(&doc).unwrap();
            assert_eq!(bytes, struct_encoding(&doc));
            let back = deserialize_doc(&bytes).unwrap();
            assert_eq!(back.to_value(), doc.to_value());
        }
    }

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

    /// The hand-written header reader agrees with bincode's.
    #[test]
    fn stored_header_parse_matches_bincode() {
        for data in [
            json!({}),
            json!({"a": [1, {"b": "é"}]}),
            create_test_doc().data,
        ] {
            let bytes = serialize_doc(&Document::with_key("c", "k".repeat(300), data)).unwrap();
            let manual = StoredDocRef::parse(&bytes).unwrap();
            let serde: StoredDocRef = bincode::deserialize(&bytes[1..]).unwrap();
            assert_eq!(
                (manual.version, manual.key, manual.id, manual.rev),
                (serde.version, serde.key, serde.id, serde.rev)
            );
            assert_eq!(
                (manual.created_at, manual.updated_at, manual.data),
                (serde.created_at, serde.updated_at, serde.data)
            );
        }
        assert!(StoredDocRef::parse(&[DOC_FORMAT_VERSION, 2, 0xff, 0xff]).is_none());
        assert!(StoredDocRef::parse(b"{}").is_none());
    }
}
