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
fn skip_value(buf: &[u8], mut pos: usize) -> Option<usize> {
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
fn visit_map<'a>(data: &'a [u8], mut f: impl FnMut(&'a [u8], &'a [u8])) -> Option<()> {
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
            let mut entries: Vec<(&[u8], &[u8])> = Vec::with_capacity(16);
            visit_map(doc.data, |k, v| entries.push((k, v)))?;
            let mut all: Vec<(&str, Out)> = Vec::with_capacity(entries.len() + 5);
            for (k, v) in &entries {
                let k = std::str::from_utf8(k).ok()?;
                if !SYSTEM_FIELDS.contains(&k) {
                    all.push((k, Out::Raw(v)));
                }
            }
            for name in SYSTEM_FIELDS {
                all.push((name, system_value(&doc, name)?));
            }
            all.sort_unstable_by(|a, b| a.0.cmp(b.0));
            rmp::encode::write_map_len(out, all.len() as u32).ok()?;
            for (k, v) in &all {
                rmp::encode::write_str(out, k).ok()?;
                write_out(out, v);
            }
        }
    }
    Some(())
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
