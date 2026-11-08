//! Document writes built from the client's MessagePack, without decoding it.
//!
//! The write-side twin of [`super::raw_rows`]. A document arrives from the
//! native driver as MessagePack and is stored as MessagePack: going through a
//! `serde_json::Value` tree on the way meant allocating every string and
//! node, encoding them all again, then freeing them — about a third of a bulk
//! load. Here the incoming bytes are transcoded straight into the stored form
//! ([`canonical_document`]), and only the fields an index reads are decoded
//! ([`indexed_view`]).
//!
//! The stored bytes must be exactly what the decoded path writes:
//! `write_msgpack(v)` for the `Value` `v` the driver's direct decoder
//! (`driver::protocol::codec`, `frame::value`) builds. So the transcoder
//! takes what that decoder takes and refuses what it refuses — the same
//! markers, valid UTF-8, string map keys, the same nesting limit — and
//! writes what the round trip writes: map keys sorted with a repeated key
//! keeping its last value, integers in their smallest encoding (a
//! non-negative one as unsigned), floats as 64-bit and non-finite ones as
//! nil, lengths in their smallest header. Anything it refuses goes the
//! decoded way, which decides what it means. The tests hold the two paths to
//! the same bytes.

use crate::storage::document::SYSTEM_FIELDS;
use chrono::{DateTime, Utc};
use serde_json::Value;

/// How deep the driver's direct decoder reads (`frame::MAX_DEPTH`).
pub(crate) const MAX_DEPTH: u32 = 256;

fn take<'a>(rd: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    if rd.len() < n {
        return None;
    }
    let (head, rest) = rd.split_at(n);
    *rd = rest;
    Some(head)
}

fn byte(rd: &mut &[u8]) -> Option<u8> {
    Some(take(rd, 1)?[0])
}

fn be<const N: usize>(rd: &mut &[u8]) -> Option<[u8; N]> {
    take(rd, N)?.try_into().ok()
}

fn is_system_field(key: &[u8]) -> bool {
    SYSTEM_FIELDS.iter().any(|f| f.as_bytes() == key)
}

/// A string's bytes after its marker, checked to be UTF-8.
fn utf8<'a>(rd: &mut &'a [u8], len: usize) -> Option<&'a str> {
    std::str::from_utf8(take(rd, len)?).ok()
}

/// A map key: any of the four string markers, UTF-8.
fn read_key<'a>(rd: &mut &'a [u8]) -> Option<&'a str> {
    let len = match byte(rd)? {
        m @ 0xa0..=0xbf => (m & 0x1f) as usize,
        0xd9 => byte(rd)? as usize,
        0xda => u16::from_be_bytes(be(rd)?) as usize,
        0xdb => u32::from_be_bytes(be(rd)?) as usize,
        _ => return None,
    };
    utf8(rd, len)
}

fn write_str(out: &mut Vec<u8>, s: &str) {
    let _ = rmp::encode::write_str(out, s);
}

fn write_int(out: &mut Vec<u8>, n: i64) {
    // `serde_json` keeps a non-negative integer as unsigned.
    if n >= 0 {
        let _ = rmp::encode::write_uint(out, n as u64);
    } else {
        let _ = rmp::encode::write_sint(out, n);
    }
}

fn write_float(out: &mut Vec<u8>, f: f64) {
    // `Number::from_f64` refuses what is not finite: the value is `null`.
    if f.is_finite() {
        let _ = rmp::encode::write_f64(out, f);
    } else {
        out.push(0xc0);
    }
}

/// Transcode one value from `rd` into `out`, at nesting `depth`.
fn canonical_value(rd: &mut &[u8], out: &mut Vec<u8>, depth: u32) -> Option<()> {
    let marker = byte(rd)?;
    match marker {
        0x00..=0x7f | 0xe0..=0xff | 0xc0 | 0xc2 | 0xc3 => out.push(marker),
        0x80..=0x8f => canonical_map(rd, (marker & 0x0f) as usize, out, depth, None)?,
        0x90..=0x9f => canonical_array(rd, (marker & 0x0f) as usize, out, depth)?,
        0xa0..=0xbf => write_str(out, utf8(rd, (marker & 0x1f) as usize)?),
        0xca => write_float(out, f64::from(f32::from_be_bytes(be(rd)?))),
        0xcb => write_float(out, f64::from_be_bytes(be(rd)?)),
        0xcc => write_int(out, i64::from(byte(rd)?)),
        0xcd => write_int(out, i64::from(u16::from_be_bytes(be(rd)?))),
        0xce => write_int(out, i64::from(u32::from_be_bytes(be(rd)?))),
        0xcf => {
            let _ = rmp::encode::write_uint(out, u64::from_be_bytes(be(rd)?));
        }
        0xd0 => write_int(out, i64::from(i8::from_be_bytes(be(rd)?))),
        0xd1 => write_int(out, i64::from(i16::from_be_bytes(be(rd)?))),
        0xd2 => write_int(out, i64::from(i32::from_be_bytes(be(rd)?))),
        0xd3 => write_int(out, i64::from_be_bytes(be(rd)?)),
        0xd9 => {
            let len = byte(rd)? as usize;
            write_str(out, utf8(rd, len)?)
        }
        0xda => {
            let len = u16::from_be_bytes(be(rd)?) as usize;
            write_str(out, utf8(rd, len)?)
        }
        0xdb => {
            let len = u32::from_be_bytes(be(rd)?) as usize;
            write_str(out, utf8(rd, len)?)
        }
        0xdc => {
            let len = u16::from_be_bytes(be(rd)?) as usize;
            canonical_array(rd, len, out, depth)?
        }
        0xdd => {
            let len = u32::from_be_bytes(be(rd)?) as usize;
            canonical_array(rd, len, out, depth)?
        }
        0xde => {
            let len = u16::from_be_bytes(be(rd)?) as usize;
            canonical_map(rd, len, out, depth, None)?
        }
        0xdf => {
            let len = u32::from_be_bytes(be(rd)?) as usize;
            canonical_map(rd, len, out, depth, None)?
        }
        // 0xc1 (never used), binary and extension types.
        _ => return None,
    }
    Some(())
}

fn canonical_array(rd: &mut &[u8], len: usize, out: &mut Vec<u8>, depth: u32) -> Option<()> {
    // As the decoder: a length beyond what is left is a truncated frame.
    if len > rd.len() || depth >= MAX_DEPTH {
        return None;
    }
    let _ = rmp::encode::write_array_len(out, len as u32);
    for _ in 0..len {
        canonical_value(rd, out, depth + 1)?;
    }
    Some(())
}

/// What a document's top-level `_key` holds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum KeyField {
    #[default]
    Absent,
    Str(String),
    /// Present but not a string: the decoded path reports it.
    Other,
}

fn key_field(canonical: &[u8]) -> KeyField {
    let mut rd = canonical;
    let len = match byte(&mut rd) {
        Some(m @ 0xa0..=0xbf) => (m & 0x1f) as usize,
        Some(0xd9) => byte(&mut rd).map_or(usize::MAX, usize::from),
        Some(0xda) => be(&mut rd).map_or(usize::MAX, |b| u16::from_be_bytes(b) as usize),
        Some(0xdb) => be(&mut rd).map_or(usize::MAX, |b| u32::from_be_bytes(b) as usize),
        _ => return KeyField::Other,
    };
    match utf8(&mut rd, len) {
        Some(s) => KeyField::Str(s.to_owned()),
        None => KeyField::Other,
    }
}

/// Transcode a map of `len` entries. With `top`, it is a document's
/// top-level map: its system fields are not fields (`Document::with_key`
/// strips them) and are left out, `_key`'s value going to `top`.
fn canonical_map(
    rd: &mut &[u8],
    len: usize,
    out: &mut Vec<u8>,
    depth: u32,
    mut top: Option<&mut KeyField>,
) -> Option<()> {
    if len > rd.len() || depth >= MAX_DEPTH {
        return None;
    }
    let entries = *rd;
    let start = out.len();
    // The common case: keys already in order, as a sorted map writes them.
    // A top-level map's header is written once its fields are counted.
    if top.is_some() {
        out.extend_from_slice(&[0; 5]);
    } else {
        let _ = rmp::encode::write_map_len(out, len as u32);
    }
    let mut previous: Option<&str> = None;
    let mut written = 0usize;
    let mut in_order = true;
    for _ in 0..len {
        let key = read_key(rd)?;
        if previous.is_some_and(|p| p >= key) {
            in_order = false;
            break;
        }
        previous = Some(key);
        if let Some(top) = top.as_deref_mut() {
            if is_system_field(key.as_bytes()) {
                let mut value = Vec::new();
                canonical_value(rd, &mut value, depth + 1)?;
                if key == "_key" {
                    *top = key_field(&value);
                }
                continue;
            }
        }
        write_str(out, key);
        canonical_value(rd, out, depth + 1)?;
        written += 1;
    }
    if !in_order {
        out.truncate(start);
        *rd = entries;
        return canonical_map_sorting(rd, len, out, depth, top);
    }
    if top.is_some() {
        let mut header = Vec::with_capacity(5);
        let _ = rmp::encode::write_map_len(&mut header, written as u32);
        out[start..start + header.len()].copy_from_slice(&header);
        out.copy_within(start + 5.., start + header.len());
        out.truncate(out.len() - (5 - header.len()));
    }
    Some(())
}

/// [`canonical_map`] for keys out of order or repeated: every entry is
/// transcoded aside, then written sorted, a repeated key keeping its last
/// value as a map insert does.
fn canonical_map_sorting(
    rd: &mut &[u8],
    len: usize,
    out: &mut Vec<u8>,
    depth: u32,
    top: Option<&mut KeyField>,
) -> Option<()> {
    let mut values = Vec::new();
    let mut entries: Vec<(&str, usize, usize)> = Vec::with_capacity(len);
    for _ in 0..len {
        let key = read_key(rd)?;
        let start = values.len();
        canonical_value(rd, &mut values, depth + 1)?;
        entries.push((key, start, values.len()));
    }
    // Stable: equal keys stay in arrival order, and the last one is kept.
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut kept: Vec<(&str, usize, usize)> = Vec::with_capacity(entries.len());
    for entry in entries {
        match kept.last_mut() {
            Some(last) if last.0 == entry.0 => *last = entry,
            _ => kept.push(entry),
        }
    }
    if let Some(top) = top {
        *top = kept
            .iter()
            .find(|e| e.0 == "_key")
            .map_or(KeyField::Absent, |e| key_field(&values[e.1..e.2]));
        kept.retain(|e| !is_system_field(e.0.as_bytes()));
    }
    let _ = rmp::encode::write_map_len(out, kept.len() as u32);
    for (key, start, end) in kept {
        write_str(out, key);
        out.extend_from_slice(&values[start..end]);
    }
    Some(())
}

/// A document as it will be stored: its fields' MessagePack, and its `_key`.
#[derive(Debug)]
pub(crate) struct CanonicalDoc {
    pub data: Vec<u8>,
    pub key: KeyField,
}

/// The stored fields of the document `bytes` (one MessagePack map, read at
/// nesting `depth` of its frame), or `None` when it is not a map or holds
/// anything the driver's decoder would not take.
pub(crate) fn canonical_document(bytes: &[u8], depth: u32) -> Option<CanonicalDoc> {
    let mut rd = bytes;
    let len = match byte(&mut rd)? {
        m @ 0x80..=0x8f => (m & 0x0f) as usize,
        0xde => u16::from_be_bytes(be(&mut rd)?) as usize,
        0xdf => u32::from_be_bytes(be(&mut rd)?) as usize,
        _ => return None,
    };
    let mut key = KeyField::Absent;
    let mut data = Vec::with_capacity(bytes.len());
    canonical_map(&mut rd, len, &mut data, depth, Some(&mut key))?;
    rd.is_empty().then_some(CanonicalDoc { data, key })
}

/// Stored fields `data` made canonical, every key kept; `None` if a key is
/// named like a system field (those are only ever in a document's header,
/// and one in the fields would change what the decoded path builds) or the
/// bytes do not transcode.
fn canonical_fields(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    let mut rd = data;
    canonical_value(&mut rd, &mut out, 0)?;
    if !rd.is_empty() || !matches!(out.first().copied(), Some(0x80..=0x8f | 0xde | 0xdf)) {
        return None;
    }
    let mut system = false;
    super::raw_rows::visit_map(&out, |k, _| system |= is_system_field(k))?;
    (!system).then_some(out)
}

/// The stored fields `old` with `changes` (a canonical map of fields, as
/// [`canonical_document`] writes them) set over them: what
/// `Document::update` does to a stored object, in MessagePack. `None` when
/// `old` cannot be read this way.
pub(crate) fn merge_fields(old: &[u8], changes: &[u8]) -> Option<Vec<u8>> {
    let old = canonical_fields(old)?;
    let mut old_entries: Vec<(&[u8], &[u8])> = Vec::with_capacity(24);
    super::raw_rows::visit_map(&old, |k, v| old_entries.push((k, v)))?;
    let mut new_entries: Vec<(&[u8], &[u8])> = Vec::with_capacity(8);
    super::raw_rows::visit_map(changes, |k, v| new_entries.push((k, v)))?;

    let mut merged: Vec<(&[u8], &[u8])> = Vec::with_capacity(old_entries.len() + new_entries.len());
    let (mut a, mut b) = (
        old_entries.into_iter().peekable(),
        new_entries.into_iter().peekable(),
    );
    loop {
        let next = match (a.peek(), b.peek()) {
            (Some(x), Some(y)) if x.0 < y.0 => a.next(),
            (Some(x), Some(y)) if x.0 == y.0 => {
                a.next();
                b.next()
            }
            (Some(_), Some(_)) => b.next(),
            (Some(_), None) => a.next(),
            (None, Some(_)) => b.next(),
            (None, None) => break,
        };
        merged.extend(next);
    }
    let mut out = Vec::with_capacity(old.len() + changes.len());
    let _ = rmp::encode::write_map_len(&mut out, merged.len() as u32);
    for (k, v) in merged {
        write_str(&mut out, std::str::from_utf8(k).ok()?);
        out.extend_from_slice(v);
    }
    Some(out)
}

/// A document's system fields, as `Document::to_value` shows them.
pub(crate) struct Header<'a> {
    pub key: &'a str,
    pub id: &'a str,
    pub rev: &'a str,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `Collection::indexed_view` of the document with this header and these
/// stored fields: the system fields and data fields in `read_fields`, the
/// data fields decoded from their canonical bytes (which decode to the very
/// values they were written from).
pub(crate) fn indexed_view(header: &Header, data: &[u8], read_fields: &[String]) -> Option<Value> {
    let mut view = serde_json::Map::new();
    let mut wanted = 0usize;
    for field in read_fields {
        let value = match field.as_str() {
            "_key" => Value::String(header.key.to_owned()),
            "_id" => Value::String(header.id.to_owned()),
            "_rev" => Value::String(header.rev.to_owned()),
            "_created_at" => Value::String(header.created_at.to_rfc3339()),
            "_updated_at" => Value::String(header.updated_at.to_rfc3339()),
            _ => {
                wanted += 1;
                continue;
            }
        };
        view.insert(field.clone(), value);
    }
    if wanted > 0 {
        let mut ok = true;
        super::raw_rows::visit_map(data, |k, v| {
            let Some(field) = read_fields.iter().find(|f| f.as_bytes() == k) else {
                return;
            };
            if is_system_field(k) {
                return;
            }
            match rmp_serde::from_slice::<Value>(v) {
                Ok(value) => {
                    view.insert(field.clone(), value);
                }
                Err(_) => ok = false,
            }
        })?;
        if !ok {
            return None;
        }
    }
    Some(Value::Object(view))
}

/// The document's API value (`Document::to_value`) in MessagePack, from its
/// system fields and stored fields: what `raw_rows::write_row(stored,
/// Whole)` writes for its stored bytes, without parsing them again or
/// checking strings this module wrote. `data` is canonical (no system-named
/// key, keys in order); the timestamps are in stored form.
pub(crate) fn write_api_value(
    out: &mut Vec<u8>,
    key: &str,
    id: &str,
    rev: &str,
    created_at: &str,
    updated_at: &str,
    data: &[u8],
) -> Option<()> {
    let mut entries: Vec<(&[u8], &[u8])> = Vec::with_capacity(24);
    super::raw_rows::visit_map(data, |k, v| entries.push((k, v)))?;
    // In byte order, as a sorted map writes them.
    let system: [(&str, &str, bool); 5] = [
        ("_created_at", created_at, true),
        ("_id", id, false),
        ("_key", key, false),
        ("_rev", rev, false),
        ("_updated_at", updated_at, true),
    ];
    let write_system = |out: &mut Vec<u8>, (name, value, stamp): (&str, &str, bool)| {
        write_str(out, name);
        match value.strip_suffix('Z') {
            // `DateTime::to_rfc3339`: the stored form with `+00:00` for `Z`.
            Some(base) if stamp => {
                let _ = rmp::encode::write_str_len(out, (base.len() + 6) as u32);
                out.extend_from_slice(base.as_bytes());
                out.extend_from_slice(b"+00:00");
            }
            _ if stamp => write_str(out, &super::serializer::stored_timestamp_to_rfc3339(value)),
            _ => write_str(out, value),
        }
    };
    let _ = rmp::encode::write_map_len(out, (entries.len() + system.len()) as u32);
    let mut pending = system.into_iter().peekable();
    for (k, v) in entries {
        while let Some(field) = pending.next_if(|(name, _, _)| name.as_bytes() < k) {
            write_system(out, field);
        }
        let _ = rmp::encode::write_str_len(out, k.len() as u32);
        out.extend_from_slice(k);
        out.extend_from_slice(v);
    }
    for field in pending {
        write_system(out, field);
    }
    Some(())
}

/// A stored timestamp as the decoded path reads it (chrono's `Deserialize`:
/// parsed with its offset, then taken to UTC).
pub(crate) fn parse_stored_timestamp(stored: &[u8]) -> Option<DateTime<Utc>> {
    std::str::from_utf8(stored)
        .ok()?
        .parse::<DateTime<chrono::FixedOffset>>()
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::document::Document;
    use crate::storage::serializer::write_msgpack;
    use rmp::encode as e;
    use serde_json::json;

    /// What the decoded path stores for a document arriving as `bytes`:
    /// `rmp_serde` (what the frame decoder's values equal, tested in
    /// `codec`), then `Document::with_key`, then `write_msgpack`.
    fn decoded_path(bytes: &[u8]) -> Option<(Vec<u8>, KeyField)> {
        let mut value: Value = rmp_serde::from_slice(bytes).ok()?;
        let key = match value.as_object_mut()?.remove("_key") {
            None => KeyField::Absent,
            Some(Value::String(s)) => KeyField::Str(s),
            Some(_) => KeyField::Other,
        };
        let doc = Document::with_key("c", "k".to_string(), value);
        let mut out = Vec::new();
        write_msgpack(&mut out, &doc.data);
        Some((out, key))
    }

    fn check(bytes: &[u8]) {
        let raw = canonical_document(bytes, 0).map(|d| (d.data, d.key));
        let decoded = decoded_path(bytes);
        // Refusing is always allowed; the decoded path then decides.
        if let Some(raw) = raw {
            assert_eq!(Some(raw), decoded, "{bytes:02x?}");
        }
    }

    type Writer<'a> = &'a dyn Fn(&mut Vec<u8>);

    fn map(entries: &[(&str, Writer)]) -> Vec<u8> {
        let mut f = Vec::new();
        e::write_map_len(&mut f, entries.len() as u32).unwrap();
        for (k, w) in entries {
            e::write_str(&mut f, k).unwrap();
            w(&mut f);
        }
        f
    }

    #[test]
    fn canonical_documents_are_what_the_decoded_path_stores() {
        // Shapes rmp_serde writes, from serde_json values.
        let values = vec![
            json!({}),
            json!({"a": 1}),
            json!({"_key": "k1", "status": "pending", "n": -3, "f": 1.5}),
            json!({"_key": 5, "x": 1}),
            json!({"_id": "x/y", "_rev": "r", "_created_at": "t", "_updated_at": "u", "z": [1, 2]}),
            json!({"nested": {"b": {"d": [1, {"z": 0, "a": null}]}, "a": true}, "e": ""}),
            json!({"big": u64::MAX, "small": i64::MIN, "zero": 0, "float": -0.0, "int_float": 3.0}),
            json!({"s": "x".repeat(31), "t": "y".repeat(32), "u": "é".repeat(200), "v": "w".repeat(70_000)}),
            json!({"odd": "\u{0}\u{1f}\"\\\u{2028}✓𝄞", "ünï": "ç", "": "empty key"}),
            json!({"arr16": (0..20).collect::<Vec<i32>>(), "map16": (0..20).map(|i| (format!("k{i:02}"), json!(i))).collect::<serde_json::Map<_, _>>()}),
        ];
        for v in values {
            let bytes = rmp_serde::to_vec(&v).unwrap();
            assert!(canonical_document(&bytes, 0).is_some(), "{v}");
            check(&bytes);
        }

        // Encodings rmp_serde never writes, which clients may.
        let unusual: Vec<Vec<u8>> = vec![
            // Keys out of order, and repeated: sorted, the last value kept.
            map(&[
                ("b", &|f: &mut Vec<u8>| e::write_u8(f, 1).unwrap()),
                ("a", &|f| e::write_u8(f, 2).unwrap()),
                ("b", &|f| e::write_u8(f, 3).unwrap()),
                ("_key", &|f| e::write_str(f, "first").unwrap()),
                ("_key", &|f| e::write_str(f, "second").unwrap()),
            ]),
            // Wide markers for small numbers, signed markers for positive
            // ones, f32, non-finite floats.
            map(&[
                ("u32", &|f: &mut Vec<u8>| e::write_u32(f, 7).unwrap()),
                ("u64", &|f| e::write_u64(f, 300).unwrap()),
                ("i8", &|f| e::write_i8(f, 5).unwrap()),
                ("i64", &|f| e::write_i64(f, -1).unwrap()),
                ("i16n", &|f| e::write_i16(f, -200).unwrap()),
                ("f32", &|f| e::write_f32(f, 1.25).unwrap()),
                ("nan", &|f| e::write_f64(f, f64::NAN).unwrap()),
                ("inf", &|f| e::write_f32(f, f32::INFINITY).unwrap()),
            ]),
            // str8/16/32 for short strings, array16/map32 for short ones,
            // nested maps out of order.
            {
                let mut f = vec![0x83];
                f.extend_from_slice(&[0xd9, 1, b'k', 0xda, 0, 2, b'h', b'i']);
                f.extend_from_slice(&[0xa1, b'a', 0xdc, 0, 2, 0x01, 0xdb, 0, 0, 0, 1, b'x']);
                f.extend_from_slice(&[
                    0xa1, b'm', 0xdf, 0, 0, 0, 2, 0xa1, b'z', 0x01, 0xa1, b'y', 0x02,
                ]);
                f
            },
            // A system field in a nested map is an ordinary key there.
            rmp_serde::to_vec(&json!({"inner": {"_key": 1, "_rev": 2}})).unwrap(),
            // Map16 header for the top level.
            {
                let mut f = vec![0xde, 0, 2];
                f.extend_from_slice(&[0xa1, b'b', 0x02, 0xa1, b'a', 0x01]);
                f
            },
        ];
        for bytes in &unusual {
            assert!(canonical_document(bytes, 0).is_some(), "{bytes:02x?}");
            check(bytes);
        }

        // Refused: what the frame decoder does not take.
        let refused: Vec<Vec<u8>> = vec![
            rmp_serde::to_vec(&json!([1, 2])).unwrap(),
            map(&[("b", &|f: &mut Vec<u8>| e::write_bin(f, b"x").unwrap())]),
            map(&[("x", &|f: &mut Vec<u8>| f.extend_from_slice(&[0xd4, 1, 2]))]),
            map(&[("x", &|f: &mut Vec<u8>| {
                f.extend_from_slice(&[0xa2, 0xff, 0xfe])
            })]),
            {
                let mut f = vec![0x81, 0x01, 0xc0]; // an integer key
                f.push(0xc0);
                f
            },
            vec![0x81, 0xa1, b'a'], // truncated
            {
                let mut f = vec![0x81, 0xa1, b'a'];
                f.extend(std::iter::repeat_n(0x91, 300));
                f.push(0xc0);
                f
            },
            {
                let mut f = rmp_serde::to_vec(&json!({"a": 1})).unwrap();
                f.push(0xc0); // trailing bytes
                f
            },
        ];
        for bytes in &refused {
            assert!(canonical_document(bytes, 0).is_none(), "{bytes:02x?}");
        }
    }

    /// The nesting limit is the frame decoder's, counted from the frame.
    #[test]
    fn nesting_limit_matches_the_decoder() {
        let nested = |levels: usize| {
            let mut f = vec![0x81, 0xa1, b'a'];
            f.extend(std::iter::repeat_n(0x91, levels));
            f.push(0xc0);
            f
        };
        // The map is at depth 0, its arrays from 1: 255 arrays reach 255.
        assert!(canonical_document(&nested(255), 0).is_some());
        assert!(canonical_document(&nested(256), 0).is_none());
        // A bulk insert's documents sit one level deeper.
        assert!(canonical_document(&nested(254), 1).is_some());
        assert!(canonical_document(&nested(255), 1).is_none());
    }

    /// Merging changes into stored fields is `Document::update` on them.
    #[test]
    fn merged_fields_are_what_update_stores() {
        let old = json!({"a": 1, "c": {"x": [1, 2]}, "e": "keep", "z": null});
        let changes = json!({"c": 5, "b": [true], "_rev": "forged", "_key": "x", "zz": {"n": -1}});
        let mut doc = Document::with_key("c", "k".to_string(), old.clone());
        let mut stored = Vec::new();
        write_msgpack(&mut stored, &doc.data);
        doc.update(changes.clone());
        let mut expected = Vec::new();
        write_msgpack(&mut expected, &doc.data);

        let canon = canonical_document(&rmp_serde::to_vec(&changes).unwrap(), 0).unwrap();
        assert_eq!(merge_fields(&stored, &canon.data).unwrap(), expected);
        let _ = old;

        // Stored fields that are not canonical (wide markers, out of order)
        // are made so, as a decode and re-encode would make them.
        let wide = vec![0x82, 0xa1, b'b', 0xcd, 0, 1, 0xa1, b'a', 0xd0, 5];
        let mut doc = Document::with_key(
            "c",
            "k".to_string(),
            rmp_serde::from_slice::<Value>(&wide).unwrap(),
        );
        doc.update(changes);
        let mut expected = Vec::new();
        write_msgpack(&mut expected, &doc.data);
        assert_eq!(merge_fields(&wide, &canon.data).unwrap(), expected);

        // A system-named stored field is left to the decoded path.
        let odd = rmp_serde::to_vec(&json!({"_rev": "in data", "a": 1})).unwrap();
        assert!(merge_fields(&odd, &canon.data).is_none());
    }

    /// The API value written from parts is what `raw_rows` writes from the
    /// stored bytes.
    #[test]
    fn api_value_is_what_raw_rows_writes() {
        use crate::storage::serializer::{serialize_doc, stored_timestamp};
        for value in [
            json!({}),
            json!({"a": 1, "zz": [1, {"b": null}], "_x": "under", "Z": "upper", "": 0}),
            json!({"aaaa": "x", "_key0": 1, "_id_": 2, "_created_at_": 3, "status": "ok"}),
        ] {
            for nanos in [0u32, 5_000_000, 123_456_789] {
                let mut doc = Document::with_key("db:c", "k/1".to_string(), value.clone());
                doc.created_at = chrono::DateTime::from_timestamp(1_600_000_000, nanos).unwrap();
                let stored = serialize_doc(&doc).unwrap();
                let mut expected = Vec::new();
                crate::storage::raw_rows::write_row(
                    &stored,
                    &crate::storage::raw_rows::RawShape::Whole,
                    &mut expected,
                )
                .unwrap();
                let mut data = Vec::new();
                write_msgpack(&mut data, &doc.data);
                let mut ours = Vec::new();
                write_api_value(
                    &mut ours,
                    &doc.key,
                    &doc.id,
                    &doc.rev,
                    &stored_timestamp(&doc.created_at),
                    &stored_timestamp(&doc.updated_at),
                    &data,
                )
                .unwrap();
                assert_eq!(ours, expected, "{value}");
            }
        }
    }

    #[test]
    fn indexed_view_matches_the_document_view() {
        let doc = Document::with_key(
            "db:c",
            "k1".to_string(),
            json!({"customer_id": "c1", "address": {"city": "Lyon"}, "n": 2.5, "other": [1]}),
        );
        let mut data = Vec::new();
        write_msgpack(&mut data, &doc.data);
        let fields: Vec<String> = [
            "customer_id",
            "address",
            "_key",
            "_created_at",
            "missing",
            "_rev",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let header = Header {
            key: &doc.key,
            id: &doc.id,
            rev: &doc.rev,
            created_at: doc.created_at,
            updated_at: doc.updated_at,
        };
        let view = indexed_view(&header, &data, &fields).unwrap();
        let full = doc.to_value();
        let mut expected = serde_json::Map::new();
        for f in &fields {
            if let Some(v) = full.get(f) {
                expected.insert(f.clone(), v.clone());
            }
        }
        assert_eq!(view, Value::Object(expected));
    }

    #[test]
    fn stored_timestamps_parse_as_serde_does() {
        let now = Utc::now();
        let stored = serde_json::to_value(now).unwrap();
        let s = stored.as_str().unwrap();
        assert_eq!(parse_stored_timestamp(s.as_bytes()), Some(now));
        let via_serde: DateTime<Utc> =
            serde_json::from_value(json!("2024-05-01T10:00:00+02:00")).unwrap();
        assert_eq!(
            parse_stored_timestamp(b"2024-05-01T10:00:00+02:00"),
            Some(via_serde)
        );
        assert_eq!(parse_stored_timestamp(b"not a date"), None);
    }
}
