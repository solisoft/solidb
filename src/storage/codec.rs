use serde_json::Value;

/// Encode a value into a binary-comparable key
/// Preserves sort order: Null < Bool < Number < String < Other
pub fn encode_key(value: &Value) -> Vec<u8> {
    let mut key = Vec::with_capacity(9);
    match value {
        Value::Null => {
            key.push(0x01);
        }
        Value::Bool(false) => {
            key.push(0x02);
            key.push(0x00);
        }
        Value::Bool(true) => {
            key.push(0x02);
            key.push(0x01);
        }
        Value::Number(n) => {
            key.push(0x03);
            let f = n.as_f64().unwrap_or(0.0);
            key.extend_from_slice(&encode_f64(f));
        }
        Value::String(s) => {
            key.push(0x04);
            key.extend_from_slice(s.as_bytes());
            key.push(0x00); // Null terminator
        }
        Value::Array(_) | Value::Object(_) => {
            // Complex types: Fallback to lexical JSON sort (safe default)
            key.push(0x05);
            let s = value.to_string();
            key.extend_from_slice(s.as_bytes());
            key.push(0x00);
        }
    }
    key
}

/// Decode a key back into a Value
pub fn decode_key(bytes: &[u8]) -> Option<Value> {
    if bytes.is_empty() {
        return None;
    }
    match bytes[0] {
        0x01 => Some(Value::Null),
        0x02 => {
             if bytes.len() < 2 { return None; }
             Some(Value::Bool(bytes[1] == 0x01))
        }
        0x03 => {
            if bytes.len() < 9 { return None; }
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&bytes[1..9]);
            let f = decode_f64(arr);
            Some(serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null))
        }
        0x04 => {
             let content = if let Some(last) = bytes.last() {
                 if *last == 0x00 && bytes.len() > 1 {
                     &bytes[1..bytes.len()-1]
                 } else {
                     &bytes[1..]
                 }
             } else {
                 &bytes[1..]
             };
             String::from_utf8(content.to_vec()).ok().map(Value::String)
        }
        0x05 => {
             let content = if let Some(last) = bytes.last() {
                 if *last == 0x00 && bytes.len() > 1 {
                     &bytes[1..bytes.len()-1]
                 } else {
                     &bytes[1..]
                 }
             } else {
                 &bytes[1..]
             };
            serde_json::from_slice(content).ok()
        }
        _ => None,
    }
}

/// Encode f64 to binary-comparable bytes
fn encode_f64(val: f64) -> [u8; 8] {
    let mut bits = val.to_bits();
    if bits & 0x8000_0000_0000_0000 != 0 {
        bits = !bits;
    } else {
        bits ^= 0x8000_0000_0000_0000;
    }
    bits.to_be_bytes()
}

/// Decode binary-comparable bytes to f64
fn decode_f64(arr: [u8; 8]) -> f64 {
    let mut bits = u64::from_be_bytes(arr);
    if bits & 0x8000_0000_0000_0000 != 0 {
        bits ^= 0x8000_0000_0000_0000;
    } else {
        bits = !bits;
    }
    f64::from_bits(bits)
}
