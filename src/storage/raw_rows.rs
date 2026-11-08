//! Query rows written straight from stored bytes.
//!
//! A document's fields are stored as MessagePack, keys sorted (they are
//! serialized from a `serde_json::Map`, a `BTreeMap`). The driver answers in
//! MessagePack too. For the simplest queries — `FOR v IN c [LIMIT …] RETURN v`,
//! `RETURN v.f`, `RETURN {k: v.f, …}` — decoding every value into a
//! `serde_json::Value` only to encode it again is almost all of the work: this
//! module copies the value bytes instead, and writes what re-encoding would
//! have written (tests check it byte for byte).
//!
//! Anything unexpected — another storage format, a document that is not a
//! map, malformed MessagePack, a system string that is not UTF-8 — returns
//! `None`, and the caller runs the query the ordinary way.

use super::serializer::{
    stored_timestamp_to_rfc3339, StoredDocRef, CREATED_AT_FIELD, ID_FIELD, KEY_FIELD, REV_FIELD,
    UPDATED_AT_FIELD,
};

/// What each row is.
#[derive(Debug, Clone)]
pub enum RawShape {
    /// The whole document, system fields included (`RETURN v`).
    Whole,
    /// One field's value (`RETURN v.f`).
    Single(String),
    /// An object `{key: v.field, …}`, pairs sorted by output key.
    Fields(Vec<(String, String)>),
}

const SYSTEM_FIELDS: [&str; 5] = [
    KEY_FIELD,
    ID_FIELD,
    REV_FIELD,
    CREATED_AT_FIELD,
    UPDATED_AT_FIELD,
];

/// Offset just past the MessagePack value starting at `pos`, without
/// decoding it. Iterative, so nesting depth cannot exhaust the stack.
pub(crate) fn skip_value(buf: &[u8], mut pos: usize) -> Option<usize> {
    let mut pending: usize = 1;
    while pending > 0 {
        pending -= 1;
        let marker = *buf.get(pos)?;
        pos += 1;
        let be = |p: usize, n: usize| -> Option<usize> {
            let bytes = buf.get(p..p + n)?;
            Some(bytes.iter().fold(0usize, |acc, b| (acc << 8) | *b as usize))
        };
        match marker {
            0x00..=0x7f | 0xe0..=0xff | 0xc0 | 0xc2 | 0xc3 => {}
            0x80..=0x8f => pending = pending.checked_add(2 * (marker & 0x0f) as usize)?,
            0x90..=0x9f => pending = pending.checked_add((marker & 0x0f) as usize)?,
            0xa0..=0xbf => pos += (marker & 0x1f) as usize,
            0xc4 | 0xd9 => pos += 1 + be(pos, 1)?,
            0xc5 | 0xda => pos += 2 + be(pos, 2)?,
            0xc6 | 0xdb => pos += 4 + be(pos, 4)?,
            0xc7 => pos += 2 + be(pos, 1)?,
            0xc8 => pos += 3 + be(pos, 2)?,
            0xc9 => pos += 5 + be(pos, 4)?,
            0xca => pos += 4,
            0xcb => pos += 8,
            0xcc | 0xd0 => pos += 1,
            0xcd | 0xd1 => pos += 2,
            0xce | 0xd2 => pos += 4,
            0xcf | 0xd3 => pos += 8,
            0xd4 => pos += 2,
            0xd5 => pos += 3,
            0xd6 => pos += 5,
            0xd7 => pos += 9,
            0xd8 => pos += 17,
            0xdc => {
                pending = pending.checked_add(be(pos, 2)?)?;
                pos += 2;
            }
            0xdd => {
                pending = pending.checked_add(be(pos, 4)?)?;
                pos += 4;
            }
            0xde => {
                pending = pending.checked_add(2usize.checked_mul(be(pos, 2)?)?)?;
                pos += 2;
            }
            0xdf => {
                pending = pending.checked_add(2usize.checked_mul(be(pos, 4)?)?)?;
                pos += 4;
            }
            0xc1 => return None,
        }
    }
    (pos <= buf.len()).then_some(pos)
}

/// Call `f` with each key/value pair of the map `data` is: raw key bytes
/// (UTF-8) and the value's encoded bytes. `None` unless `data` is exactly
/// one well-formed map with string keys.
pub(crate) fn visit_map<'a>(data: &'a [u8], mut f: impl FnMut(&'a [u8], &'a [u8])) -> Option<()> {
    let marker = *data.first()?;
    let (len, mut pos) = match marker {
        0x80..=0x8f => ((marker & 0x0f) as usize, 1),
        0xde => (
            u16::from_be_bytes(data.get(1..3)?.try_into().ok()?) as usize,
            3,
        ),
        0xdf => (
            u32::from_be_bytes(data.get(1..5)?.try_into().ok()?) as usize,
            5,
        ),
        _ => return None,
    };
    for _ in 0..len {
        let marker = *data.get(pos)?;
        let (klen, hdr) = match marker {
            0xa0..=0xbf => ((marker & 0x1f) as usize, 1),
            0xd9 => (*data.get(pos + 1)? as usize, 2),
            0xda => (
                u16::from_be_bytes(data.get(pos + 1..pos + 3)?.try_into().ok()?) as usize,
                3,
            ),
            0xdb => (
                u32::from_be_bytes(data.get(pos + 1..pos + 5)?.try_into().ok()?) as usize,
                5,
            ),
            _ => return None,
        };
        let key = data.get(pos + hdr..pos + hdr + klen)?;
        let vstart = pos + hdr + klen;
        let vend = skip_value(data, vstart)?;
        f(key, &data[vstart..vend]);
        pos = vend;
    }
    (pos == data.len()).then_some(())
}

/// A value to emit: stored bytes, a string, or nil.
enum Out<'a> {
    Raw(&'a [u8]),
    Str(String),
    Nil,
}

fn write_out(out: &mut Vec<u8>, value: &Out) {
    match value {
        Out::Raw(bytes) => out.extend_from_slice(bytes),
        Out::Str(s) => {
            let _ = rmp::encode::write_str(out, s);
        }
        Out::Nil => out.push(0xc0),
    }
}

/// A system field's value, from the stored header.
fn system_value(doc: &StoredDocRef, name: &str) -> Option<Out<'static>> {
    let text = |b: &[u8]| std::str::from_utf8(b).ok().map(str::to_owned);
    let stamp = |b: &[u8]| std::str::from_utf8(b).ok().map(stored_timestamp_to_rfc3339);
    Some(Out::Str(match name {
        KEY_FIELD => text(doc.key)?,
        ID_FIELD => text(doc.id)?,
        REV_FIELD => text(doc.rev)?,
        CREATED_AT_FIELD => stamp(doc.created_at)?,
        UPDATED_AT_FIELD => stamp(doc.updated_at)?,
        _ => return None,
    }))
}

/// [`SYSTEM_FIELDS`] in byte order, as a map's keys are written.
const SYSTEM_FIELDS_SORTED: [&str; 5] = [
    CREATED_AT_FIELD,
    ID_FIELD,
    KEY_FIELD,
    REV_FIELD,
    UPDATED_AT_FIELD,
];

/// The system field `name` and its value from the stored header, as
/// [`system_value`] gives it, written as a map entry without building the
/// string first.
fn write_system(out: &mut Vec<u8>, doc: &StoredDocRef, name: &str) -> Option<()> {
    let raw = match name {
        KEY_FIELD => doc.key,
        ID_FIELD => doc.id,
        REV_FIELD => doc.rev,
        CREATED_AT_FIELD => doc.created_at,
        UPDATED_AT_FIELD => doc.updated_at,
        _ => return None,
    };
    let text = std::str::from_utf8(raw).ok()?;
    rmp::encode::write_str(out, name).ok()?;
    if name == CREATED_AT_FIELD || name == UPDATED_AT_FIELD {
        // `stored_timestamp_to_rfc3339`'s common case, written in place.
        if let Some(base) = text.strip_suffix('Z') {
            rmp::encode::write_str_len(out, (base.len() + 6) as u32).ok()?;
            out.extend_from_slice(base.as_bytes());
            out.extend_from_slice(b"+00:00");
            return Some(());
        }
        rmp::encode::write_str(out, &stored_timestamp_to_rfc3339(text)).ok()?;
        return Some(());
    }
    rmp::encode::write_str(out, text).ok()?;
    Some(())
}

/// A field's value: from the header for a system field (which overrides a
/// same-named data field, as in `Document::to_value`), else the stored
/// bytes, or nil when absent.
fn value_of<'v>(doc: &StoredDocRef, field: &str, found: Option<&'v [u8]>) -> Option<Out<'v>> {
    if SYSTEM_FIELDS.contains(&field) {
        return system_value(doc, field);
    }
    Some(found.map_or(Out::Nil, Out::Raw))
}

/// Append one row, for the stored document `bytes`, to `out`. `None` (with
/// `out` possibly holding a partial row) when the document cannot be read
/// this way; the caller then discards the whole buffer.
pub fn write_row(bytes: &[u8], shape: &RawShape, out: &mut Vec<u8>) -> Option<()> {
    let doc = StoredDocRef::parse(bytes)?;
    match shape {
        RawShape::Single(field) => {
            let mut found = None;
            visit_map(doc.data, |k, v| {
                if k == field.as_bytes() {
                    found = Some(v);
                }
            })?;
            write_out(out, &value_of(&doc, field, found)?);
        }
        RawShape::Fields(pairs) => {
            // One pass over the document, no allocation for up to 16 fields.
            let mut stack = [None; 16];
            let mut heap;
            let found: &mut [Option<&[u8]>] = if pairs.len() <= stack.len() {
                &mut stack[..pairs.len()]
            } else {
                heap = vec![None; pairs.len()];
                &mut heap
            };
            visit_map(doc.data, |k, v| {
                for (slot, (_, field)) in found.iter_mut().zip(pairs) {
                    if k == field.as_bytes() {
                        *slot = Some(v);
                    }
                }
            })?;
            rmp::encode::write_map_len(out, pairs.len() as u32).ok()?;
            for ((key, field), found) in pairs.iter().zip(found.iter()) {
                let value = value_of(&doc, field, *found)?;
                rmp::encode::write_str(out, key).ok()?;
                write_out(out, &value);
            }
        }
        RawShape::Whole => {
            // The data fields, less any named like a system field (the
            // header's value wins). They are stored in key order, being
            // serialized from a `BTreeMap`, so the system fields are merged
            // into them as they are written: no sort, and no string built.
            let mut entries: Vec<(&[u8], &[u8])> = Vec::with_capacity(24);
            let mut sorted = true;
            visit_map(doc.data, |k, v| {
                if entries.last().is_some_and(|(last, _)| *last >= k) {
                    sorted = false;
                }
                if !SYSTEM_FIELDS.iter().any(|f| f.as_bytes() == k) {
                    entries.push((k, v));
                }
            })?;
            if !sorted {
                entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
            }
            rmp::encode::write_map_len(out, (entries.len() + SYSTEM_FIELDS.len()) as u32).ok()?;
            let mut system = SYSTEM_FIELDS_SORTED.iter().peekable();
            for (k, v) in &entries {
                while let Some(name) = system.next_if(|name| name.as_bytes() < *k) {
                    write_system(out, &doc, name)?;
                }
                rmp::encode::write_str(out, std::str::from_utf8(k).ok()?).ok()?;
                out.extend_from_slice(v);
            }
            for name in system {
                write_system(out, &doc, name)?;
            }
        }
    }
    Some(())
}

/// The `fields` of a stored document, as
/// [`super::serializer::deserialize_doc_projected`] returns them: one pass over
/// the stored map that decodes the values asked for and steps over the others
/// by their encoded length, never visiting them. A scan that reads two fields
/// of a 500-byte document with nested arrays and objects spends its time on
/// those two. `None` for anything this cannot read (another storage format, a
/// document that is not one map, a key or system string that is not UTF-8, a
/// value that does not decode); the caller then takes the serde path, which
/// decides what those mean.
pub(crate) fn project(bytes: &[u8], fields: &[String]) -> Option<serde_json::Value> {
    let doc = StoredDocRef::parse(bytes)?;
    let mut map = serde_json::Map::new();
    let mut readable = true;
    visit_map(doc.data, |k, v| {
        if !readable {
            return;
        }
        let Ok(key) = std::str::from_utf8(k) else {
            readable = false;
            return;
        };
        if fields.iter().any(|f| f == key) {
            match rmp_serde::from_slice::<serde_json::Value>(v) {
                // A repeated key keeps its last value, as a map decode does.
                Ok(value) => {
                    map.insert(key.to_owned(), value);
                }
                Err(_) => readable = false,
            }
        }
    })?;
    if !readable {
        return None;
    }
    // System fields last, overriding a same-named data field, as the full
    // decode does.
    for field in fields {
        if SYSTEM_FIELDS.contains(&field.as_str()) {
            let Out::Str(text) = system_value(&doc, field)? else {
                return None;
            };
            map.insert(field.clone(), serde_json::Value::String(text));
        }
    }
    Some(serde_json::Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::document::Document;
    use crate::storage::serializer::{deserialize_doc_as_value, serialize_doc};
    use serde_json::{json, Value};

    fn stored(data: Value) -> Vec<u8> {
        serialize_doc(&Document::with_key("c", "k1".to_string(), data)).unwrap()
    }

    /// What the ordinary path sends for one row.
    fn reference(bytes: &[u8], shape: &RawShape) -> Vec<u8> {
        let doc = deserialize_doc_as_value(bytes).unwrap();
        let value = match shape {
            RawShape::Whole => doc,
            RawShape::Single(f) => doc.get(f).cloned().unwrap_or(Value::Null),
            RawShape::Fields(pairs) => Value::Object(
                pairs
                    .iter()
                    .map(|(k, f)| (k.clone(), doc.get(f).cloned().unwrap_or(Value::Null)))
                    .collect(),
            ),
        };
        rmp_serde::to_vec_named(&value).unwrap()
    }

    fn docs() -> Vec<Value> {
        vec![
            json!({"id": 1, "title": "Post title 1", "views": 7}),
            json!({"n": -3, "big": 18446744073709551615u64, "neg": -9223372036854775808i64,
                   "f": 1.5, "t": true, "z": null, "s": "é".repeat(40),
                   "arr": [1, "two", [3, {"four": 4}], {}], "obj": {"b": 2, "a": {"x": [null]}}}),
            json!({"_key": "shadowed", "zzz": "last", "_id": 5, "long": "x".repeat(70_000)}),
            json!({}),
        ]
    }

    #[test]
    fn same_bytes_as_the_decode_and_encode_path() {
        let shapes = [
            RawShape::Whole,
            RawShape::Single("title".into()),
            RawShape::Single("_key".into()),
            RawShape::Single("missing".into()),
            RawShape::Fields(vec![
                ("id".into(), "id".into()),
                ("title".into(), "title".into()),
                ("views".into(), "views".into()),
            ]),
            RawShape::Fields(vec![
                ("a".into(), "obj".into()),
                ("k".into(), "_key".into()),
                ("m".into(), "missing".into()),
                ("when".into(), "_created_at".into()),
            ]),
        ];
        for data in docs() {
            let bytes = stored(data.clone());
            for shape in &shapes {
                let mut out = Vec::new();
                write_row(&bytes, shape, &mut out).unwrap();
                assert_eq!(out, reference(&bytes, shape), "{data} {shape:?}");
            }
        }
    }

    /// The single-pass projection keeps exactly the fields asked for, with
    /// the values (system fields included, shadowing a data field of the
    /// same name) the full decode gives them.
    #[test]
    fn projection_matches_the_full_decode() {
        let field_sets: [&[&str]; 5] = [
            &["title", "views"],
            &["obj", "arr", "missing"],
            &["_key", "_id", "zzz", "_created_at", "_updated_at", "_rev"],
            &["long", "s", "big", "neg", "f", "t", "z", "n"],
            &[],
        ];
        for data in docs() {
            let bytes = stored(data.clone());
            let full = deserialize_doc_as_value(&bytes).unwrap();
            for set in field_sets {
                let fields: Vec<String> = set.iter().map(|f| f.to_string()).collect();
                let expected: serde_json::Map<String, Value> = full
                    .as_object()
                    .unwrap()
                    .iter()
                    .filter(|(k, _)| fields.contains(k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                assert_eq!(
                    project(&bytes, &fields),
                    Some(Value::Object(expected)),
                    "{data} {set:?}"
                );
            }
        }
        // Left to the serde path: not a map, not this format.
        assert_eq!(project(&stored(json!([1, 2])), &["a".into()]), None);
        assert_eq!(project(b"{\"a\":1}", &["a".into()]), None);
    }

    #[test]
    fn system_fields_sorted_is_sorted() {
        let mut sorted = SYSTEM_FIELDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, SYSTEM_FIELDS_SORTED);
    }

    /// A stored map whose keys are out of order (no current writer makes
    /// one) is still written in key order, as decoding it into a map does.
    #[test]
    fn unsorted_stored_fields_are_written_sorted() {
        let mut bytes = stored(json!({"a": 1, "b": 2}));
        // Swap the two entries of the stored data map in place.
        let data = rmp_serde::to_vec_named(&json!({"a": 1, "b": 2})).unwrap();
        let swapped = [&[0x82u8][..], &[0xa1, b'b', 0x02], &[0xa1, b'a', 0x01]].concat();
        assert_eq!(data.len(), swapped.len());
        let at = bytes
            .windows(data.len())
            .position(|w| w == data.as_slice())
            .unwrap();
        bytes[at..at + data.len()].copy_from_slice(&swapped);
        let mut out = Vec::new();
        write_row(&bytes, &RawShape::Whole, &mut out).unwrap();
        assert_eq!(out, reference(&bytes, &RawShape::Whole));
    }

    #[test]
    fn skip_value_handles_every_width() {
        for data in docs() {
            let bytes = rmp_serde::to_vec_named(&data).unwrap();
            assert_eq!(skip_value(&bytes, 0), Some(bytes.len()), "{data}");
        }
        assert_eq!(skip_value(&[0xc1], 0), None);
        assert_eq!(skip_value(&[0x92, 0x01], 0), None, "truncated array");
        assert_eq!(
            skip_value(&[0xdb, 0xff, 0xff, 0xff, 0xff], 0),
            None,
            "truncated string"
        );
    }

    #[test]
    fn unreadable_documents_are_refused() {
        let mut out = Vec::new();
        assert!(write_row(b"", &RawShape::Whole, &mut out).is_none());
        assert!(write_row(b"{\"_key\":\"x\"}", &RawShape::Whole, &mut out).is_none());
        let not_a_map = stored(json!([1, 2]));
        assert!(write_row(&not_a_map, &RawShape::Whole, &mut out).is_none());
    }
}
