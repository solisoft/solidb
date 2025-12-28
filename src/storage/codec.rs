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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_encode_decode_null() {
        let value = Value::Null;
        let encoded = encode_key(&value);
        let decoded = decode_key(&encoded);
        assert_eq!(decoded, Some(Value::Null));
    }

    #[test]
    fn test_encode_decode_bool() {
        let true_val = Value::Bool(true);
        let false_val = Value::Bool(false);
        
        let encoded_true = encode_key(&true_val);
        let encoded_false = encode_key(&false_val);
        
        assert_eq!(decode_key(&encoded_true), Some(Value::Bool(true)));
        assert_eq!(decode_key(&encoded_false), Some(Value::Bool(false)));
        
        // false should sort before true
        assert!(encoded_false < encoded_true);
    }

    #[test]
    fn test_encode_decode_numbers() {
        let values = vec![
            json!(-100.5),
            json!(-1),
            json!(0),
            json!(1),
            json!(42),
            json!(100.5),
            json!(1000000),
        ];
        
        for v in &values {
            let encoded = encode_key(v);
            let decoded = decode_key(&encoded);
            // Numbers might have precision differences, check approximately
            assert!(decoded.is_some());
            if let (Some(orig), Some(dec)) = 
                   (v.as_f64(), decoded.as_ref().and_then(|d| d.as_f64())) {
                assert!((orig - dec).abs() < 1e-10);
            }
        }
    }

    #[test]
    fn test_encode_decode_strings() {
        let values = vec!["", "hello", "world", "abc123", "🦀 Rust"];
        
        for s in values {
            let value = Value::String(s.to_string());
            let encoded = encode_key(&value);
            let decoded = decode_key(&encoded);
            assert_eq!(decoded, Some(Value::String(s.to_string())));
        }
    }

    #[test]
    fn test_encode_decode_array() {
        let value = json!([1, 2, 3]);
        let encoded = encode_key(&value);
        let decoded = decode_key(&encoded);
        assert!(decoded.is_some());
    }

    #[test]
    fn test_encode_decode_object() {
        let value = json!({"key": "value"});
        let encoded = encode_key(&value);
        let decoded = decode_key(&encoded);
        assert!(decoded.is_some());
    }

    #[test]
    fn test_sort_order_types() {
        // Sort order: Null < Bool < Number < String < Complex
        let null = encode_key(&Value::Null);
        let bool_val = encode_key(&Value::Bool(false));
        let number = encode_key(&json!(0));
        let string = encode_key(&json!("a"));
        let array = encode_key(&json!([]));
        
        assert!(null < bool_val);
        assert!(bool_val < number);
        assert!(number < string);
        assert!(string < array);
    }

    #[test]
    fn test_sort_order_numbers() {
        let neg_big = encode_key(&json!(-1000));
        let neg_small = encode_key(&json!(-1));
        let zero = encode_key(&json!(0));
        let pos_small = encode_key(&json!(1));
        let pos_big = encode_key(&json!(1000));
        
        assert!(neg_big < neg_small);
        assert!(neg_small < zero);
        assert!(zero < pos_small);
        assert!(pos_small < pos_big);
    }

    #[test]
    fn test_sort_order_strings() {
        let a = encode_key(&json!("a"));
        let b = encode_key(&json!("b"));
        let aa = encode_key(&json!("aa"));
        let ab = encode_key(&json!("ab"));
        
        assert!(a < aa);
        assert!(a < b);
        assert!(aa < ab);
        assert!(ab < b);
    }

    #[test]
    fn test_decode_empty() {
        assert_eq!(decode_key(&[]), None);
    }

    #[test]
    fn test_decode_invalid_type() {
        assert_eq!(decode_key(&[0xFF]), None);
    }

    #[test]
    fn test_decode_truncated_bool() {
        // Only type byte, no value byte
        assert_eq!(decode_key(&[0x02]), None);
    }

    #[test]
    fn test_decode_truncated_number() {
        // Type byte but not enough bytes for f64
        assert_eq!(decode_key(&[0x03, 0x00, 0x00]), None);
    }

    #[test]
    fn test_f64_roundtrip() {
        let values = vec![-1.0, 0.0, 1.0, std::f64::consts::PI, -std::f64::consts::E];
        for v in values {
            let encoded = encode_f64(v);
            let decoded = decode_f64(encoded);
            assert!((v - decoded).abs() < 1e-15);
        }
    }
}

