use super::command::Command;
use super::error::DriverError;
use super::response::Response;
use serde::{Deserialize, Serialize};

pub const DRIVER_MAGIC: &[u8] = b"solidb-drv-v1\0";
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

pub fn encode_command(cmd: &Command) -> Result<Vec<u8>, DriverError> {
    let payload = rmp_serde::to_vec_named(cmd)
        .map_err(|e| DriverError::ProtocolError(format!("Serialization failed: {}", e)))?;

    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(DriverError::MessageTooLarge);
    }

    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

pub fn encode_response(resp: &Response) -> Result<Vec<u8>, DriverError> {
    // Serialized in place after a length placeholder, rather than into its
    // own buffer and then copied behind the length: one allocation and no
    // copy of what can be a large result set.
    let mut buf = Vec::with_capacity(256);
    buf.extend_from_slice(&[0u8; 4]);
    if let Response::RawRows { data } = resp {
        // `{"status": "ok", "data": <rows>}`, written around the rows'
        // bytes: what `Ok { data }` serializes to (tested in `response`).
        buf.reserve(data.0.len() + 16);
        let header = rmp::encode::write_map_len(&mut buf, 2)
            .and_then(|_| rmp::encode::write_str(&mut buf, "status"))
            .and_then(|_| rmp::encode::write_str(&mut buf, "ok"))
            .and_then(|_| rmp::encode::write_str(&mut buf, "data"));
        header.map_err(|e| DriverError::ProtocolError(format!("Serialization failed: {}", e)))?;
        buf.extend_from_slice(&data.0);
    } else {
        rmp_serde::encode::write_named(&mut buf, resp)
            .map_err(|e| DriverError::ProtocolError(format!("Serialization failed: {}", e)))?;
    }

    let payload_len = buf.len() - 4;
    if payload_len > MAX_MESSAGE_SIZE {
        return Err(DriverError::MessageTooLarge);
    }
    buf[..4].copy_from_slice(&(payload_len as u32).to_be_bytes());
    Ok(buf)
}

pub fn encode_message<T: Serialize>(msg: &T) -> Result<Vec<u8>, DriverError> {
    let payload = rmp_serde::to_vec_named(msg)
        .map_err(|e| DriverError::ProtocolError(format!("Serialization failed: {}", e)))?;

    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(DriverError::MessageTooLarge);
    }

    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

pub fn decode_message<T: for<'de> Deserialize<'de>>(data: &[u8]) -> Result<T, DriverError> {
    rmp_serde::from_slice(data)
        .map_err(|e| DriverError::ProtocolError(format!("Deserialization failed: {}", e)))
}

/// Decode a command frame, as [`decode_message`] does.
///
/// `Command` is internally tagged (`{"cmd": "insert", …}`), and serde decodes
/// such an enum by first buffering the whole frame into its generic `Content`
/// tree, then building the variant from that: every document of a write was
/// decoded twice, a second copy of each string and number allocated and
/// freed. The document writes are decoded straight into their fields when the
/// frame opens with its tag, which is what serde writes and so what the
/// clients send. Any other frame, and any frame the direct decode rejects,
/// goes through [`decode_message`]: the result, and every error, is the one
/// it gives.
pub fn decode_command(data: &[u8]) -> Result<Command, DriverError> {
    if let Some(command) = direct::decode(data) {
        return Ok(command);
    }
    decode_message(data)
}

mod direct {
    use super::Command;
    use serde::de::IgnoredAny;
    use serde::Deserialize;
    use serde_json::Value;

    // The variants' fields as `Command` declares them. `cmd` is a field so
    // that a frame repeating it fails here as it fails there.

    #[derive(Deserialize)]
    struct Insert {
        #[allow(dead_code)]
        cmd: IgnoredAny,
        database: String,
        collection: String,
        key: Option<String>,
        document: Value,
    }

    #[derive(Deserialize)]
    struct Update {
        #[allow(dead_code)]
        cmd: IgnoredAny,
        database: String,
        collection: String,
        key: String,
        document: Value,
        #[serde(default)]
        merge: bool,
    }

    #[derive(Deserialize)]
    struct Delete {
        #[allow(dead_code)]
        cmd: IgnoredAny,
        database: String,
        collection: String,
        key: String,
    }

    #[derive(Deserialize)]
    struct BulkInsert {
        #[allow(dead_code)]
        cmd: IgnoredAny,
        database: String,
        collection: String,
        documents: Vec<Value>,
    }

    /// The tag, when `data` is a MessagePack map whose first key is `cmd`
    /// with a string value.
    fn leading_tag(data: &[u8]) -> Option<&[u8]> {
        let mut rd = data;
        rmp::decode::read_map_len(&mut rd).ok()?;
        let key = read_str(&mut rd)?;
        if key != b"cmd" {
            return None;
        }
        read_str(&mut rd)
    }

    fn read_str<'a>(rd: &mut &'a [u8]) -> Option<&'a [u8]> {
        let len = rmp::decode::read_str_len(rd).ok()? as usize;
        if rd.len() < len {
            return None;
        }
        let (s, rest) = rd.split_at(len);
        *rd = rest;
        Some(s)
    }

    fn from<'a, T: Deserialize<'a>>(data: &'a [u8]) -> Option<T> {
        rmp_serde::from_slice(data).ok()
    }

    pub(super) fn decode(data: &[u8]) -> Option<Command> {
        let tag = leading_tag(data)?;
        if let Some(command) = frame::parse(data, tag) {
            return Some(command);
        }
        Some(match tag {
            b"insert" => {
                let c: Insert = from(data)?;
                Command::Insert {
                    database: c.database,
                    collection: c.collection,
                    key: c.key,
                    document: c.document,
                }
            }
            b"update" => {
                let c: Update = from(data)?;
                Command::Update {
                    database: c.database,
                    collection: c.collection,
                    key: c.key,
                    document: c.document,
                    merge: c.merge,
                }
            }
            b"delete" => {
                let c: Delete = from(data)?;
                Command::Delete {
                    database: c.database,
                    collection: c.collection,
                    key: c.key,
                }
            }
            b"bulk_insert" => {
                let c: BulkInsert = from(data)?;
                Command::BulkInsert {
                    database: c.database,
                    collection: c.collection,
                    documents: c.documents,
                }
            }
            _ => return None,
        })
    }

    /// The write commands read straight off the MessagePack, documents
    /// included, without serde. Only the plainest frames are taken: the
    /// command's own fields, each once, with the types serde would accept,
    /// and documents made of nil, booleans, numbers, valid UTF-8 strings,
    /// arrays and string-keyed maps, not deeper than [`MAX_DEPTH`]. Anything
    /// else — an unknown field, a binary or extension value, a truncated
    /// frame — is `None`, and the caller decodes the frame with serde.
    ///
    /// Each document comes out as `serde_json`'s visitor builds it from
    /// `rmp_serde`: a positive integer as `u64`, a negative one as `i64`, a
    /// float through `Number::from_f64` (so not-a-number is `null`), a
    /// repeated map key keeping its last value.
    pub(super) mod frame {
        use super::Command;
        use serde_json::{Map, Number, Value};

        /// `rmp_serde` allows 1,024 levels; deeper documents go to it.
        const MAX_DEPTH: u32 = 256;

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

        fn utf8(rd: &mut &[u8], len: usize) -> Option<String> {
            std::str::from_utf8(take(rd, len)?).ok().map(str::to_owned)
        }

        /// A string, as any of the four string markers.
        fn string(rd: &mut &[u8]) -> Option<String> {
            let len = match byte(rd)? {
                m @ 0xa0..=0xbf => (m & 0x1f) as usize,
                0xd9 => byte(rd)? as usize,
                0xda => u16::from_be_bytes(be(rd)?) as usize,
                0xdb => u32::from_be_bytes(be(rd)?) as usize,
                _ => return None,
            };
            utf8(rd, len)
        }

        fn float(f: f64) -> Value {
            Number::from_f64(f).map_or(Value::Null, Value::Number)
        }

        fn array(rd: &mut &[u8], len: usize, depth: u32) -> Option<Value> {
            // Every element takes a byte at least: a length beyond what is
            // left is a truncated frame, and must not size an allocation.
            if len > rd.len() || depth >= MAX_DEPTH {
                return None;
            }
            let mut items = Vec::with_capacity(len);
            for _ in 0..len {
                items.push(value(rd, depth + 1)?);
            }
            Some(Value::Array(items))
        }

        fn map(rd: &mut &[u8], len: usize, depth: u32) -> Option<Value> {
            if len > rd.len() || depth >= MAX_DEPTH {
                return None;
            }
            let mut fields = Map::new();
            for _ in 0..len {
                let key = string(rd)?;
                let item = value(rd, depth + 1)?;
                fields.insert(key, item);
            }
            Some(Value::Object(fields))
        }

        pub(super) fn value(rd: &mut &[u8], depth: u32) -> Option<Value> {
            let marker = byte(rd)?;
            Some(match marker {
                0x00..=0x7f => Value::Number(u64::from(marker).into()),
                0x80..=0x8f => return map(rd, (marker & 0x0f) as usize, depth),
                0x90..=0x9f => return array(rd, (marker & 0x0f) as usize, depth),
                0xa0..=0xbf => Value::String(utf8(rd, (marker & 0x1f) as usize)?),
                0xc0 => Value::Null,
                0xc2 => Value::Bool(false),
                0xc3 => Value::Bool(true),
                0xca => float(f64::from(f32::from_be_bytes(be(rd)?))),
                0xcb => float(f64::from_be_bytes(be(rd)?)),
                0xcc => Value::Number(u64::from(byte(rd)?).into()),
                0xcd => Value::Number(u64::from(u16::from_be_bytes(be(rd)?)).into()),
                0xce => Value::Number(u64::from(u32::from_be_bytes(be(rd)?)).into()),
                0xcf => Value::Number(u64::from_be_bytes(be(rd)?).into()),
                0xd0 => Value::Number(i64::from(i8::from_be_bytes(be(rd)?)).into()),
                0xd1 => Value::Number(i64::from(i16::from_be_bytes(be(rd)?)).into()),
                0xd2 => Value::Number(i64::from(i32::from_be_bytes(be(rd)?)).into()),
                0xd3 => Value::Number(i64::from_be_bytes(be(rd)?).into()),
                0xd9 => {
                    let len = byte(rd)? as usize;
                    Value::String(utf8(rd, len)?)
                }
                0xda => {
                    let len = u16::from_be_bytes(be(rd)?) as usize;
                    Value::String(utf8(rd, len)?)
                }
                0xdb => {
                    let len = u32::from_be_bytes(be(rd)?) as usize;
                    Value::String(utf8(rd, len)?)
                }
                0xdc => {
                    let len = u16::from_be_bytes(be(rd)?) as usize;
                    return array(rd, len, depth);
                }
                0xdd => {
                    let len = u32::from_be_bytes(be(rd)?) as usize;
                    return array(rd, len, depth);
                }
                0xde => {
                    let len = u16::from_be_bytes(be(rd)?) as usize;
                    return map(rd, len, depth);
                }
                0xdf => {
                    let len = u32::from_be_bytes(be(rd)?) as usize;
                    return map(rd, len, depth);
                }
                0xe0..=0xff => Value::Number(i64::from(marker as i8).into()),
                // 0xc1 (never used), binary and extension types.
                _ => return None,
            })
        }

        fn map_len(rd: &mut &[u8]) -> Option<usize> {
            Some(match byte(rd)? {
                m @ 0x80..=0x8f => (m & 0x0f) as usize,
                0xde => u16::from_be_bytes(be(rd)?) as usize,
                0xdf => u32::from_be_bytes(be(rd)?) as usize,
                _ => return None,
            })
        }

        fn once<T>(slot: &mut Option<T>, value: Option<T>) -> Option<()> {
            if slot.is_some() {
                return None; // a repeated field: serde's error
            }
            *slot = Some(value?);
            Some(())
        }

        pub(crate) fn parse(data: &[u8], tag: &[u8]) -> Option<Command> {
            let mut rd = data;
            let fields = map_len(&mut rd)?;
            let (mut cmd, mut database, mut collection) = (None, None, None);
            let (mut key, mut document, mut documents, mut merge) = (None, None, None, None);
            for _ in 0..fields {
                let name = string(&mut rd)?;
                match (name.as_str(), tag) {
                    ("cmd", _) => once(&mut cmd, string(&mut rd))?,
                    ("database", _) => once(&mut database, string(&mut rd))?,
                    ("collection", _) => once(&mut collection, string(&mut rd))?,
                    ("key", b"insert") => {
                        let k = if rd.first() == Some(&0xc0) {
                            rd = &rd[1..];
                            Some(None)
                        } else {
                            string(&mut rd).map(Some)
                        };
                        once(&mut key, k)?
                    }
                    ("key", _) => once(&mut key, string(&mut rd).map(Some))?,
                    ("document", b"insert" | b"update") => once(&mut document, value(&mut rd, 0))?,
                    ("documents", b"bulk_insert") => {
                        let docs = match value(&mut rd, 0)? {
                            Value::Array(docs) => Some(docs),
                            _ => None,
                        };
                        once(&mut documents, docs)?
                    }
                    ("merge", b"update") => {
                        let m = match byte(&mut rd)? {
                            0xc2 => Some(false),
                            0xc3 => Some(true),
                            _ => None,
                        };
                        once(&mut merge, m)?
                    }
                    _ => return None,
                }
            }
            cmd?;
            let (database, collection) = (database?, collection?);
            Some(match tag {
                b"insert" => Command::Insert {
                    database,
                    collection,
                    key: key.flatten(),
                    document: document?,
                },
                b"update" => Command::Update {
                    database,
                    collection,
                    key: key??,
                    document: document?,
                    merge: merge.unwrap_or(false),
                },
                b"delete" => Command::Delete {
                    database,
                    collection,
                    key: key??,
                },
                b"bulk_insert" => Command::BulkInsert {
                    database,
                    collection,
                    documents: documents?,
                },
                _ => return None,
            })
        }
    }
}

#[cfg(test)]
mod decode_command_tests {
    use super::*;
    use serde_json::json;

    fn payload(cmd: &Command) -> Vec<u8> {
        encode_command(cmd).unwrap()[4..].to_vec()
    }

    fn debug(r: &Result<Command, DriverError>) -> String {
        format!("{r:?}")
    }

    /// The direct decoder answers what the derived one does.
    #[test]
    fn write_commands_decode_as_the_derived_decoder_does() {
        let commands = vec![
            Command::Insert {
                database: "db".into(),
                collection: "c".into(),
                key: Some("k".into()),
                document: json!({"a": 1, "b": [1.5, -2, "x", null, true], "n": {"m": {}}}),
            },
            Command::Insert {
                database: "db".into(),
                collection: "c".into(),
                key: None,
                document: json!({"big": u64::MAX, "neg": i64::MIN}),
            },
            Command::Update {
                database: "db".into(),
                collection: "c".into(),
                key: "k".into(),
                document: json!({"status": "shipped"}),
                merge: true,
            },
            Command::Delete {
                database: "db".into(),
                collection: "c".into(),
                key: "k".into(),
            },
            Command::BulkInsert {
                database: "db".into(),
                collection: "c".into(),
                documents: vec![json!({"_key": "a"}), json!({"x": [1, 2]}), json!(7)],
            },
        ];
        for cmd in commands {
            let bytes = payload(&cmd);
            assert!(
                direct::decode(&bytes).is_some(),
                "{cmd:?} takes the direct path"
            );
            assert_eq!(
                debug(&decode_command(&bytes)),
                debug(&decode_message::<Command>(&bytes))
            );
            assert_eq!(debug(&decode_command(&bytes)), debug(&Ok(cmd)));
        }
    }

    /// A frame `{cmd: "insert", database, collection, document: <doc>}` with
    /// `doc` written by `write`.
    fn insert_frame(write: impl Fn(&mut Vec<u8>)) -> Vec<u8> {
        let mut f = Vec::new();
        rmp::encode::write_map_len(&mut f, 4).unwrap();
        for (k, v) in [("cmd", "insert"), ("database", "d"), ("collection", "c")] {
            rmp::encode::write_str(&mut f, k).unwrap();
            rmp::encode::write_str(&mut f, v).unwrap();
        }
        rmp::encode::write_str(&mut f, "document").unwrap();
        write(&mut f);
        f
    }

    /// Documents read off the MessagePack are what serde builds, for every
    /// encoding a client may use — including ones `rmp_serde` itself never
    /// writes (wide markers for small numbers, `f32`, not-a-number, repeated
    /// keys) — and anything the reader does not take is left to serde.
    #[test]
    fn documents_read_as_serde_reads_them() {
        use rmp::encode as e;
        type Writer = Box<dyn Fn(&mut Vec<u8>)>;
        let mut docs: Vec<Writer> = vec![
            Box::new(|f| {
                e::write_map_len(f, 3).unwrap();
                e::write_str(f, "a").unwrap();
                e::write_u32(f, 7).unwrap();
                e::write_str(f, "a").unwrap(); // repeated: the last one wins
                e::write_i8(f, 5).unwrap();
                e::write_str(f, "neg").unwrap();
                e::write_i64(f, -1).unwrap();
            }),
            Box::new(|f| {
                e::write_array_len(f, 6).unwrap();
                e::write_f32(f, 1.5).unwrap();
                e::write_f64(f, f64::NAN).unwrap();
                e::write_f64(f, f64::INFINITY).unwrap();
                e::write_f32(f, f32::NAN).unwrap();
                e::write_u64(f, u64::MAX).unwrap();
                e::write_i64(f, i64::MIN).unwrap();
            }),
            Box::new(|f| {
                // str8 / str16 / str32 for short strings, array16, map32.
                e::write_map_len(f, 1).unwrap();
                f.extend_from_slice(&[0xd9, 1, b'k']);
                f.extend_from_slice(&[0xdc, 0, 3]);
                f.extend_from_slice(&[0xda, 0, 2, b'h', b'i']);
                f.extend_from_slice(&[0xdb, 0, 0, 0, 1, b'x']);
                f.extend_from_slice(&[0xdf, 0, 0, 0, 0]);
            }),
            Box::new(|f| {
                e::write_str(f, &"é".repeat(40_000)).unwrap();
            }),
            Box::new(|f| {
                // Nesting deeper than the reader goes.
                for _ in 0..300 {
                    e::write_array_len(f, 1).unwrap();
                }
                e::write_nil(f).unwrap();
            }),
            // Not taken by the reader; serde accepts or rejects them.
            Box::new(|f| e::write_bin(f, b"bytes").unwrap()),
            Box::new(|f| {
                e::write_map_len(f, 1).unwrap();
                e::write_u8(f, 1).unwrap(); // an integer key
                e::write_nil(f).unwrap();
            }),
            Box::new(|f| {
                e::write_map_len(f, 1).unwrap();
                e::write_bin(f, b"k").unwrap(); // a binary key
                e::write_nil(f).unwrap();
            }),
            Box::new(|f| f.extend_from_slice(&[0xa2, 0xff, 0xfe])), // bad UTF-8
            Box::new(|f| f.extend_from_slice(&[0xc1])),
            Box::new(|f| f.extend_from_slice(&[0xd4, 1, 2])), // fixext1
            Box::new(|f| f.extend_from_slice(&[0xdd, 0xff, 0xff, 0xff, 0xff])), // truncated
        ];
        docs.push(Box::new(|f| {
            rmp_serde::encode::write(f, &json!({"nested": {"x": [1, -2, 3.5, "s", null, false]}}))
                .unwrap()
        }));
        for write in docs {
            let bytes = insert_frame(write);
            assert_eq!(
                debug(&decode_command(&bytes)),
                debug(&decode_message::<Command>(&bytes)),
                "frame {:?}",
                &bytes[..bytes.len().min(64)]
            );
        }
        // The plain ones did take the direct path.
        let plain = insert_frame(|f| {
            rmp_serde::encode::write(f, &json!({"a": [1, {"b": "c"}], "d": -1.25})).unwrap()
        });
        assert!(direct::frame::parse(&plain, b"insert").is_some());
    }

    /// Frames the direct decoder does not take, or rejects, get the derived
    /// decoder's answer, errors included.
    #[test]
    fn other_frames_fall_back() {
        let mut frames: Vec<Vec<u8>> = vec![
            // Not a write command.
            payload(&Command::Ping),
            // `merge` omitted: defaults to false on both paths.
            rmp_serde::to_vec_named(&json!({
                "cmd": "update", "database": "d", "collection": "c", "key": "k",
                "document": {"a": 1}
            }))
            .unwrap(),
            // The tag after other fields.
            rmp_serde::to_vec_named(&json!({
                "database": "d", "cmd": "insert", "collection": "c", "document": {}
            }))
            .unwrap(),
            // A missing field, a wrong type, an unknown command.
            rmp_serde::to_vec_named(&json!({"cmd": "insert", "database": "d"})).unwrap(),
            rmp_serde::to_vec_named(&json!({
                "cmd": "delete", "database": 5, "collection": "c", "key": "k"
            }))
            .unwrap(),
            rmp_serde::to_vec_named(&json!({"cmd": "no_such_thing"})).unwrap(),
            // Not a map at all, and truncated.
            rmp_serde::to_vec(&json!([1, 2])).unwrap(),
            Vec::new(),
        ];
        // A frame naming `cmd` twice.
        let mut twice = Vec::new();
        rmp::encode::write_map_len(&mut twice, 5).unwrap();
        for (k, v) in [
            ("cmd", "delete"),
            ("database", "d"),
            ("collection", "c"),
            ("key", "k"),
            ("cmd", "delete"),
        ] {
            rmp::encode::write_str(&mut twice, k).unwrap();
            rmp::encode::write_str(&mut twice, v).unwrap();
        }
        frames.push(twice);
        let truncated = payload(&Command::Delete {
            database: "d".into(),
            collection: "c".into(),
            key: "k".into(),
        });
        frames.push(truncated[..truncated.len() - 2].to_vec());

        for bytes in frames {
            assert_eq!(
                debug(&decode_command(&bytes)),
                debug(&decode_message::<Command>(&bytes)),
                "frame {bytes:?}"
            );
        }
    }
}
