//! Responses decoded from MessagePack by hand.
//!
//! A query's rows used to reach the caller through `rmp_serde` and
//! `serde_json::Value`'s `Deserialize`: a visitor call per value, the first
//! key of every map through serde_json's key classifier, every array grown
//! one push at a time. [`decode_response`] reads the same bytes straight
//! into `Value`s. The allocations stay what `Value` needs — a `String` per
//! key and per string, a vector per array, B-tree nodes per map, 58 for an
//! order of about 500 bytes with its system fields — and they are now most
//! of what decoding costs; the rest is about a fifth less work than before.
//!
//! It reads only what it is sure to read as `rmp_serde` does — a `status:
//! "ok"` response whose values are nil, booleans, integers, floats, UTF-8
//! strings, arrays and maps with UTF-8 string keys — and gives up on
//! anything else: another status, a binary or extension value, a key that is
//! not a string, invalid UTF-8, a truncated frame, an unknown or repeated
//! field, nesting deeper than [`MAX_DEPTH`]. A frame it gives up on is
//! decoded by [`decode_message`], so its result, or its error, is exactly
//! the one it always was.

use super::codec::decode_message;
use super::error::DriverError;
use super::response::Response;
use serde_json::{Map, Number, Value};

/// Deeper than this, the frame goes to `rmp_serde` (whose own limit is 1024).
const MAX_DEPTH: usize = 256;

/// The prefix of the map keys serde_json gives a meaning of their own to
/// (`arbitrary_precision` numbers, `raw_value`), when the first key of a
/// map. Left to serde_json, under whatever features it was built with.
const SERDE_JSON_PRIVATE: &str = "$serde_json::private::";

/// Decode a response frame's payload, as `decode_message::<Response>` does.
pub fn decode_response(data: &[u8]) -> Result<Response, DriverError> {
    match Reader::new(data).ok_response() {
        Some(response) => Ok(response),
        None => decode_message(data),
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    /// `{"status": "ok", "data"?, "count"?, "tx_id"?}` in any order, as the
    /// flat `WireResponse` reads it. Trailing bytes are ignored, as
    /// `rmp_serde::from_slice` ignores them.
    fn ok_response(&mut self) -> Option<Response> {
        let fields = match self.byte()? {
            m @ 0x80..=0x8f => usize::from(m & 0x0f),
            0xde => usize::from(self.u16()?),
            0xdf => self.u32()? as usize,
            _ => return None,
        };
        let mut status = false;
        let mut data: Option<Option<Value>> = None;
        let mut count: Option<Option<usize>> = None;
        let mut tx_id: Option<Option<String>> = None;
        for _ in 0..fields {
            match self.str()? {
                "status" if !status => {
                    if self.str()? != "ok" {
                        return None;
                    }
                    status = true;
                }
                "data" if data.is_none() => {
                    data = Some(if self.nil() {
                        None
                    } else {
                        Some(self.value(1)?)
                    });
                }
                "count" if count.is_none() => {
                    count = Some(if self.nil() {
                        None
                    } else {
                        Some(usize::try_from(self.uint()?).ok()?)
                    });
                }
                "tx_id" if tx_id.is_none() => {
                    tx_id = Some(if self.nil() {
                        None
                    } else {
                        Some(self.str()?.to_owned())
                    });
                }
                _ => return None,
            }
        }
        if !status {
            return None;
        }
        Some(Response::Ok {
            data: data.flatten(),
            count: count.flatten(),
            tx_id: tx_id.flatten(),
        })
    }

    #[inline]
    fn byte(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    #[inline]
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let bytes = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(bytes)
    }

    #[inline]
    fn array_of<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }

    #[inline]
    fn u16(&mut self) -> Option<u16> {
        self.array_of().map(u16::from_be_bytes)
    }

    #[inline]
    fn u32(&mut self) -> Option<u32> {
        self.array_of().map(u32::from_be_bytes)
    }

    /// Consume a nil, if that is what comes next.
    #[inline]
    fn nil(&mut self) -> bool {
        let is_nil = self.data.get(self.pos) == Some(&0xc0);
        if is_nil {
            self.pos += 1;
        }
        is_nil
    }

    /// An unsigned integer marker's value; anything else gives up.
    fn uint(&mut self) -> Option<u64> {
        Some(match self.byte()? {
            m @ 0x00..=0x7f => u64::from(m),
            0xcc => u64::from(self.byte()?),
            0xcd => u64::from(self.u16()?),
            0xce => u64::from(self.u32()?),
            0xcf => u64::from_be_bytes(self.array_of()?),
            _ => return None,
        })
    }

    #[inline]
    fn str_of(&mut self, len: usize) -> Option<&'a str> {
        std::str::from_utf8(self.take(len)?).ok()
    }

    /// A string marker and its UTF-8 bytes; anything else gives up.
    #[inline]
    fn str(&mut self) -> Option<&'a str> {
        let len = match self.byte()? {
            m @ 0xa0..=0xbf => usize::from(m & 0x1f),
            0xd9 => usize::from(self.byte()?),
            0xda => usize::from(self.u16()?),
            0xdb => self.u32()? as usize,
            _ => return None,
        };
        self.str_of(len)
    }

    /// One value, as `serde_json::Value::deserialize` builds it from
    /// `rmp_serde`'s visitor calls:
    /// - unsigned integers through `visit_u64`, signed ones through
    ///   `visit_i64` (so a non-negative `int 8` is a `PosInt`, as there);
    /// - a float that is not finite becomes `Null`, `f32` widened first;
    /// - map entries inserted in wire order, a repeated key's last value
    ///   kept.
    fn value(&mut self, depth: usize) -> Option<Value> {
        let marker = self.byte()?;
        Some(match marker {
            0x00..=0x7f => Value::Number(u64::from(marker).into()),
            0x80..=0x8f => return self.map(usize::from(marker & 0x0f), depth),
            0x90..=0x9f => return self.array(usize::from(marker & 0x0f), depth),
            0xa0..=0xbf => return self.string(usize::from(marker & 0x1f)),
            0xc0 => Value::Null,
            0xc2 => Value::Bool(false),
            0xc3 => Value::Bool(true),
            0xca => float(f64::from(f32::from_be_bytes(self.array_of()?))),
            0xcb => float(f64::from_be_bytes(self.array_of()?)),
            0xcc => Value::Number(u64::from(self.byte()?).into()),
            0xcd => Value::Number(u64::from(self.u16()?).into()),
            0xce => Value::Number(u64::from(self.u32()?).into()),
            0xcf => Value::Number(u64::from_be_bytes(self.array_of()?).into()),
            0xd0 => Value::Number(i64::from(self.byte()? as i8).into()),
            0xd1 => Value::Number(i64::from(i16::from_be_bytes(self.array_of()?)).into()),
            0xd2 => Value::Number(i64::from(i32::from_be_bytes(self.array_of()?)).into()),
            0xd3 => Value::Number(i64::from_be_bytes(self.array_of()?).into()),
            0xd9 => {
                let len = usize::from(self.byte()?);
                return self.string(len);
            }
            0xda => {
                let len = usize::from(self.u16()?);
                return self.string(len);
            }
            0xdb => {
                let len = self.u32()? as usize;
                return self.string(len);
            }
            0xdc => {
                let len = usize::from(self.u16()?);
                return self.array(len, depth);
            }
            0xdd => {
                let len = self.u32()? as usize;
                return self.array(len, depth);
            }
            0xde => {
                let len = usize::from(self.u16()?);
                return self.map(len, depth);
            }
            0xdf => {
                let len = self.u32()? as usize;
                return self.map(len, depth);
            }
            0xe0..=0xff => Value::Number(i64::from(marker as i8).into()),
            // 0xc1 (never used), bin 8/16/32, ext and fixext.
            _ => return None,
        })
    }

    #[inline]
    fn string(&mut self, len: usize) -> Option<Value> {
        self.str_of(len).map(|s| Value::String(s.to_owned()))
    }

    fn array(&mut self, len: usize, depth: usize) -> Option<Value> {
        if depth >= MAX_DEPTH {
            return None;
        }
        // Every element takes at least a byte: a length the frame cannot
        // hold must not size the allocation.
        let mut items = Vec::with_capacity(len.min(self.data.len() - self.pos));
        for _ in 0..len {
            items.push(self.value(depth + 1)?);
        }
        Some(Value::Array(items))
    }

    fn map(&mut self, len: usize, depth: usize) -> Option<Value> {
        if depth >= MAX_DEPTH {
            return None;
        }
        // Sized up front, as `array` is: with serde_json's `preserve_order`
        // a map grown one insert at a time reallocates and rehashes its
        // index on the way. An entry takes at least two bytes.
        let mut map = Map::with_capacity(len.min((self.data.len() - self.pos) / 2));
        for n in 0..len {
            let key = self.str()?;
            if n == 0 && key.starts_with(SERDE_JSON_PRIVATE) {
                return None;
            }
            let value = self.value(depth + 1)?;
            map.insert(key.to_owned(), value);
        }
        Some(Value::Object(map))
    }
}

#[inline]
fn float(f: f64) -> Value {
    Number::from_f64(f).map_or(Value::Null, Value::Number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use serde_json::json;

    /// What the client decoded responses with before.
    fn old(data: &[u8]) -> Result<Response, DriverError> {
        decode_message(data)
    }

    /// Both decoders, compared through `Debug`: stricter than `PartialEq`
    /// on `Value` (`-0.0` against `0.0`, `5` against `5.0`), and it covers
    /// the errors too.
    fn same(data: &[u8]) -> Result<Response, DriverError> {
        let (a, b) = (old(data), decode_response(data));
        assert_eq!(format!("{a:?}"), format!("{b:?}"), "frame {data:02x?}");
        b
    }

    /// `same`, and the frame was read by hand rather than handed back.
    fn by_hand(data: &[u8]) -> Response {
        assert!(
            Reader::new(data).ok_response().is_some(),
            "fell back on {data:02x?}"
        );
        same(data).expect("decodes")
    }

    fn frame<T: Serialize>(t: &T) -> Vec<u8> {
        rmp_serde::to_vec_named(t).unwrap()
    }

    // A MessagePack writer that picks its markers, so that every width is
    // seen, not only the shortest one rmp_serde writes.
    struct W(Vec<u8>, u64);

    impl W {
        fn new(seed: u64) -> W {
            W(Vec::new(), seed | 1)
        }
        fn roll(&mut self) -> u64 {
            self.1 ^= self.1 << 13;
            self.1 ^= self.1 >> 7;
            self.1 ^= self.1 << 17;
            self.1
        }
        fn raw(&mut self, b: &[u8]) -> &mut Self {
            self.0.extend_from_slice(b);
            self
        }
        fn str_len(&mut self, len: usize) {
            match (self.roll() % 4, len) {
                (0, 0..=31) => self.0.push(0xa0 | len as u8),
                (1, 0..=255) => self.0.extend_from_slice(&[0xd9, len as u8]),
                (2, 0..=65535) => {
                    self.0.push(0xda);
                    self.0.extend_from_slice(&(len as u16).to_be_bytes());
                }
                _ => {
                    self.0.push(0xdb);
                    self.0.extend_from_slice(&(len as u32).to_be_bytes());
                }
            }
        }
        fn str(&mut self, s: &str) -> &mut Self {
            self.str_len(s.len());
            self.raw(s.as_bytes())
        }
        fn container(&mut self, len: usize, fix: u8, m16: u8, m32: u8) {
            match (self.roll() % 3, len) {
                (0, 0..=15) => self.0.push(fix | len as u8),
                (1, 0..=65535) => {
                    self.0.push(m16);
                    self.0.extend_from_slice(&(len as u16).to_be_bytes());
                }
                _ => {
                    self.0.push(m32);
                    self.0.extend_from_slice(&(len as u32).to_be_bytes());
                }
            }
        }
        fn map(&mut self, len: usize) -> &mut Self {
            self.container(len, 0x80, 0xde, 0xdf);
            self
        }
        fn array(&mut self, len: usize) -> &mut Self {
            self.container(len, 0x90, 0xdc, 0xdd);
            self
        }
        /// An integer in a marker wide enough for it, chosen at random.
        fn int(&mut self, v: i64) -> &mut Self {
            let mut options: Vec<Vec<u8>> = Vec::new();
            if (0..=127).contains(&v) {
                options.push(vec![v as u8]);
            }
            if (-32..0).contains(&v) {
                options.push(vec![v as i8 as u8]);
            }
            if (0..=255).contains(&v) {
                options.push(vec![0xcc, v as u8]);
            }
            if (0..=65535).contains(&v) {
                let mut b = vec![0xcd];
                b.extend_from_slice(&(v as u16).to_be_bytes());
                options.push(b);
            }
            if (0..=u32::MAX as i64).contains(&v) {
                let mut b = vec![0xce];
                b.extend_from_slice(&(v as u32).to_be_bytes());
                options.push(b);
            }
            if v >= 0 {
                let mut b = vec![0xcf];
                b.extend_from_slice(&(v as u64).to_be_bytes());
                options.push(b);
            }
            if (-128..=127).contains(&v) {
                options.push(vec![0xd0, v as i8 as u8]);
            }
            if (-32768..=32767).contains(&v) {
                let mut b = vec![0xd1];
                b.extend_from_slice(&(v as i16).to_be_bytes());
                options.push(b);
            }
            if (i32::MIN as i64..=i32::MAX as i64).contains(&v) {
                let mut b = vec![0xd2];
                b.extend_from_slice(&(v as i32).to_be_bytes());
                options.push(b);
            }
            let mut b = vec![0xd3];
            b.extend_from_slice(&v.to_be_bytes());
            options.push(b);
            let pick = (self.roll() % options.len() as u64) as usize;
            self.raw(&options[pick].clone())
        }
        /// A random value `depth` levels deep at most.
        fn value(&mut self, depth: u32) {
            const STRINGS: [&str; 8] = [
                "",
                "plain",
                "Jaurès",
                "日本語のテキスト",
                "emoji 🦀 and \u{0}",
                "rue de la République, 69002 Lyon",
                "a string that is longer than thirty-one bytes, so not a fixstr",
                "\"quoted\" \\ back",
            ];
            let kind = self.roll() % if depth == 0 { 6 } else { 8 };
            match kind {
                0 => {
                    self.0.push(0xc0);
                }
                1 => {
                    let b = self.roll().is_multiple_of(2);
                    self.0.push(if b { 0xc3 } else { 0xc2 });
                }
                2 => {
                    let v = match self.roll() % 6 {
                        0 => i64::MIN,
                        1 => i64::MAX,
                        2 => -((self.roll() % 40) as i64),
                        3 => (self.roll() % 300) as i64,
                        _ => self.roll() as i64 >> (self.roll() % 63),
                    };
                    self.int(v);
                }
                3 => {
                    if self.roll().is_multiple_of(3) {
                        // u64 beyond i64.
                        let v = u64::MAX - self.roll() % 1000;
                        self.0.push(0xcf);
                        self.0.extend_from_slice(&v.to_be_bytes());
                    } else {
                        let f = match self.roll() % 7 {
                            0 => f64::NAN,
                            1 => f64::INFINITY,
                            2 => -0.0,
                            3 => 1e300,
                            4 => 12.5,
                            _ => (self.roll() % 100_000) as f64 / 100.0,
                        };
                        if self.roll().is_multiple_of(3) {
                            self.0.push(0xca);
                            self.0.extend_from_slice(&(f as f32).to_be_bytes());
                        } else {
                            self.0.push(0xcb);
                            self.0.extend_from_slice(&f.to_be_bytes());
                        }
                    }
                }
                4 | 5 => {
                    let s = STRINGS[(self.roll() % 8) as usize];
                    self.str(s);
                }
                6 => {
                    let n = (self.roll() % 6) as usize;
                    self.array(n);
                    for _ in 0..n {
                        self.value(depth - 1);
                    }
                }
                _ => {
                    let n = (self.roll() % 7) as usize;
                    self.map(n);
                    for _ in 0..n {
                        // A small key set, so that repeats happen.
                        let k = ["_key", "b", "a", "name", "zé", "", "items"]
                            [(self.roll() % 7) as usize];
                        self.str(k);
                        self.value(depth - 1);
                    }
                }
            }
        }
    }

    /// `{"status": "ok", "data": <rows>}` with `rows` written by `W`.
    fn ok_frame(seed: u64, rows: usize) -> Vec<u8> {
        let mut w = W::new(seed);
        w.map(2).str("status").str("ok").str("data").array(rows);
        for _ in 0..rows {
            w.value(5);
        }
        w.0
    }

    fn order(i: u64) -> Value {
        json!({
            "_key": format!("o{i:07}"),
            "_id": format!("orders/o{i:07}"),
            "_rev": "1762600000000000",
            "customer_id": format!("c{:05}", i * 7919 % 100_000),
            "status": "delivered",
            "amount": 123.45 + i as f64,
            "currency": "EUR",
            "created_at": "2025-03-04T05:06:07Z",
            "country": "FR",
            "channel": "web",
            "tags": ["gift", "express", "eco"],
            "items": [
                {"sku": "SKU-00042", "qty": 2, "price": 19.99},
                {"sku": "SKU-01234", "qty": 1, "price": 5.0},
                {"sku": "SKU-49999", "qty": 4, "price": 0.5},
            ],
            "address": {"street": "12 avenue Jean Jaurès", "city": "Lyon", "zip": "69007"},
            "note": "Call before delivery; the intercom does not work.",
            "big": u64::MAX,
            "small": i64::MIN,
            "nested": {"a": [1, [2, [3, {"b": null}]]], "t": true, "f": false},
        })
    }

    #[test]
    fn every_response_the_server_writes_decodes_the_same() {
        let rows: Vec<Value> = (0..20).map(order).collect();
        let resp = by_hand(&frame(&Response::ok(json!(rows))));
        assert!(matches!(resp, Response::Ok { data: Some(Value::Array(r)), .. } if r.len() == 20));
        by_hand(&frame(&Response::ok(json!([]))));
        by_hand(&frame(&Response::ok(json!({"_key": "o1", "x": 1}))));
        by_hand(&frame(&Response::ok(Value::Null)));
        by_hand(&frame(&Response::ok(json!(-3))));
        by_hand(&frame(&Response::ok_count(7)));
        by_hand(&frame(&Response::ok_empty()));
        by_hand(&frame(&Response::ok_tx("tx-1".into())));
        let resp = by_hand(&frame(&Response::Ok {
            data: Some(json!([1])),
            count: Some(usize::MAX),
            tx_id: Some("t".into()),
        }));
        assert!(matches!(
            resp,
            Response::Ok {
                count: Some(usize::MAX),
                ..
            }
        ));
        // Answered by rmp_serde, as before.
        let error = frame(&Response::error(DriverError::DatabaseError("boom".into())));
        assert!(matches!(same(&error), Ok(Response::Error { .. })));
        let too_large = frame(&Response::error(DriverError::MessageTooLarge));
        assert!(matches!(same(&too_large), Ok(Response::Error { .. })));
        let pong = frame(&Response::Pong { timestamp: 42 });
        assert!(matches!(same(&pong), Ok(Response::Pong { timestamp: 42 })));
        let batch = frame(&Response::Batch {
            responses: vec![Response::ok(json!([order(1)])), Response::ok_count(2)],
        });
        assert!(matches!(same(&batch), Ok(Response::Batch { .. })));
    }

    #[test]
    fn generated_documents_decode_the_same() {
        for seed in 0..3000u64 {
            let data = ok_frame(seed, (seed % 12) as usize);
            by_hand(&data);
        }
    }

    #[test]
    fn values_keep_their_number_kinds() {
        let mut w = W::new(1);
        w.map(2).str("status").str("ok").str("data").array(9);
        // A non-negative int 8 and int 64: `PosInt`, as `visit_i64` makes them.
        w.raw(&[0xd0, 5]).raw(&[0xd3, 0, 0, 0, 0, 0, 0, 0, 9]);
        w.raw(&[0xff]); // -1
        w.raw(&[0xcf]).raw(&u64::MAX.to_be_bytes());
        w.raw(&[0xd3]).raw(&i64::MIN.to_be_bytes());
        w.raw(&[0xca]).raw(&1.5f32.to_be_bytes());
        w.raw(&[0xcb]).raw(&f64::NAN.to_be_bytes());
        w.raw(&[0xcb]).raw(&(-0.0f64).to_be_bytes());
        w.raw(&[0xcb]).raw(&3.0f64.to_be_bytes());
        let Response::Ok {
            data: Some(Value::Array(v)),
            ..
        } = by_hand(&w.0)
        else {
            panic!("rows")
        };
        assert!(v[0].is_u64() && v[1].is_u64() && v[2].is_i64() && v[3].is_u64());
        assert_eq!(v[4], json!(i64::MIN));
        assert_eq!(v[5], json!(1.5));
        assert_eq!(v[6], Value::Null);
        assert!(v[7].as_f64().unwrap().is_sign_negative());
        assert!(v[8].is_f64());
    }

    #[test]
    fn maps_keep_the_last_of_a_repeated_key() {
        let mut w = W::new(2);
        w.map(2).str("data").map(3);
        w.str("b").int(1).str("a").int(2).str("b").int(3);
        w.str("status").str("ok");
        let resp = by_hand(&w.0);
        let Response::Ok { data: Some(d), .. } = resp else {
            panic!()
        };
        assert_eq!(d, json!({"a": 2, "b": 3}));
    }

    #[test]
    fn what_it_does_not_read_is_left_to_rmp_serde() {
        let fell_back = |data: &[u8]| {
            assert!(Reader::new(data).ok_response().is_none(), "{data:02x?}");
            same(data)
        };
        let row = |value: &[u8]| {
            let mut w = W::new(3);
            w.map(2)
                .str("status")
                .str("ok")
                .str("data")
                .array(1)
                .raw(value);
            w.0
        };
        // A binary, an extension: rmp_serde's errors.
        assert!(fell_back(&row(&[0xc4, 2, b'h', b'i'])).is_err());
        assert!(fell_back(&row(&[0xd4, 1, 0])).is_err());
        assert!(fell_back(&row(&[0xc7, 1, 5, 0])).is_err());
        assert!(fell_back(&row(&[0xc1])).is_err());
        // Invalid UTF-8.
        assert!(fell_back(&row(&[0xa2, 0xff, 0xfe])).is_err());
        // Keys that are not strings: an integer, a binary first and second.
        assert!(fell_back(&row(&[0x81, 0x01, 0x02])).is_err());
        assert!(fell_back(&row(&[0x81, 0xc4, 1, b'k', 0x02])).is_err());
        fell_back(&row(&[0x82, 0xa1, b'a', 0x01, 0xc4, 1, b'k', 0x02])).unwrap();
        // serde_json's private keys.
        fell_back(&row(&[
            0x81, 0xb6, b'$', b's', b'e', b'r', b'd', b'e', b'_', b'j', b's', b'o', b'n', b':',
            b':', b'p', b'r', b'i', b'v', b'a', b't', b'e', b':', b':', 0x01,
        ]))
        .unwrap();
        // The response itself: as an array, an unknown or repeated field,
        // no status, another status, a count that is not an unsigned integer.
        fell_back(&[0x92, 0xa2, b'o', b'k', 0xc0]).ok();
        let mut w = W::new(4);
        w.map(2).str("status").str("ok").str("extra").int(1);
        fell_back(&w.0).unwrap();
        let mut w = W::new(5);
        w.map(2).str("status").str("ok").str("status").str("ok");
        assert!(fell_back(&w.0).is_err());
        let mut w = W::new(6);
        w.map(1).str("data").array(0);
        assert!(fell_back(&w.0).is_err());
        let mut w = W::new(7);
        w.map(1).str("status").str("weird");
        assert!(fell_back(&w.0).is_err());
        let mut w = W::new(8);
        w.map(2).str("status").str("ok").str("count").int(-1);
        assert!(fell_back(&w.0).is_err());
        let mut w = W::new(9);
        w.map(2)
            .str("status")
            .str("ok")
            .str("count")
            .raw(&[0xd0, 3]);
        fell_back(&w.0).unwrap();
        // Nesting deeper than the limit: read by rmp_serde, up to its own.
        // rmp_serde (and dropping what it built) recurses once per level; in
        // an unoptimised build 2,000 levels overflow a test thread's 2 MiB
        // stack, as they would have with the old client, so give it room.
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || {
                for depth in [MAX_DEPTH + 10, 2000] {
                    let mut data = vec![0x82, 0xa6];
                    data.extend_from_slice(b"status");
                    data.extend_from_slice(&[0xa2, b'o', b'k', 0xa4]);
                    data.extend_from_slice(b"data");
                    data.extend(std::iter::repeat_n(0x91, depth));
                    data.push(0xc0);
                    assert!(Reader::new(&data).ok_response().is_none());
                    same(&data).ok();
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn truncated_frames_fail_as_they_did() {
        let data = frame(&Response::ok(json!([order(1), order(2)])));
        for end in 0..data.len() {
            assert!(same(&data[..end]).is_err(), "{end} bytes");
        }
        let data = ok_frame(11, 6);
        for end in 0..data.len() {
            same(&data[..end]).ok();
        }
    }

    #[test]
    fn trailing_bytes_are_ignored_as_they_were() {
        let mut data = frame(&Response::ok(json!([1, 2])));
        data.extend_from_slice(&[0xc1, 0xff, 0x00]);
        by_hand(&data);
    }

    #[test]
    fn corrupted_frames_fail_or_decode_as_they_did() {
        let base = frame(&Response::ok(json!([order(3), order(4)])));
        let mut rng = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..5000 {
            let mut data = base.clone();
            for _ in 0..1 + rng % 3 {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                let at = (rng >> 8) as usize % data.len();
                data[at] = (rng >> 40) as u8;
            }
            same(&data).ok();
        }
    }
}
