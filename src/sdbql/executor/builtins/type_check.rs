//! Type checking functions for SDBQL.
//!
//! IS_ARRAY, IS_BOOL, IS_NUMBER, IS_STRING, IS_NULL, IS_OBJECT, etc.

use crate::error::{DbError, DbResult};
use serde_json::Value;

/// Evaluate type checking functions
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "IS_ARRAY" | "IS_LIST" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Array(_)))))
        }
        "IS_BOOL" | "IS_BOOLEAN" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Bool(_)))))
        }
        "IS_NUMBER" | "IS_NUMERIC" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Number(_)))))
        }
        "IS_INTEGER" | "IS_INT" => {
            check_args(name, args, 1)?;
            let is_int = match &args[0] {
                Value::Number(n) => {
                    if n.as_i64().is_some() {
                        true
                    } else if let Some(f) = n.as_f64() {
                        f.fract() == 0.0 && f.is_finite()
                    } else {
                        false
                    }
                }
                _ => false,
            };
            Ok(Some(Value::Bool(is_int)))
        }
        "IS_STRING" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::String(_)))))
        }
        "IS_NULL" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Null))))
        }
        "IS_OBJECT" | "IS_DOCUMENT" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(matches!(args[0], Value::Object(_)))))
        }
        "IS_EMPTY" => {
            check_args(name, args, 1)?;
            let is_empty = match &args[0] {
                Value::Null => true,
                Value::String(s) => s.is_empty(),
                Value::Array(arr) => arr.is_empty(),
                Value::Object(obj) => obj.is_empty(),
                _ => false,
            };
            Ok(Some(Value::Bool(is_empty)))
        }
        // Strings the date functions can parse (RFC 3339, `YYYY-MM-DD`,
        // `YYYY-MM-DD HH:MM:SS`). Numbers are not dates here even though the
        // date functions accept epoch timestamps: every number would pass.
        "IS_DATE" | "IS_DATETIME" | "IS_DATESTRING" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(
                args[0].is_string()
                    && crate::sdbql::executor::utils::parse_datetime(&args[0]).is_ok(),
            )))
        }
        "IS_IPV4" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(
                args[0]
                    .as_str()
                    .and_then(super::string::parse_ipv4)
                    .is_some(),
            )))
        }
        "IS_KEY" => {
            check_args(name, args, 1)?;
            let ok = args[0].as_str().is_some_and(|s| {
                !s.is_empty()
                    && s.len() <= 254
                    && !s.contains('/')
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))
            });
            Ok(Some(Value::Bool(ok)))
        }
        "IS_SAME_COLLECTION" => {
            check_args(name, args, 2)?;
            let extract_collection = |val: &Value| -> Option<String> {
                match val {
                    Value::String(s) => s.split('/').next().map(|c| c.to_string()),
                    Value::Object(obj) => obj
                        .get("_id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.split('/').next().map(|c| c.to_string())),
                    _ => None,
                }
            };
            let col1 = extract_collection(&args[0]);
            let col2 = extract_collection(&args[1]);
            match (col1, col2) {
                (Some(c1), Some(c2)) => Ok(Some(Value::Bool(c1 == c2))),
                _ => Ok(Some(Value::Bool(false))),
            }
        }
        // Checksum validators. Spaces (and, for IBAN, nothing else) are
        // ignored so values typed by people validate; anything that is not a
        // string or a number is simply not valid.
        "IS_IBAN" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(
                compact(&args[0]).is_some_and(|s| is_iban(&s)),
            )))
        }
        "LUHN" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(compact(&args[0]).is_some_and(|s| {
                !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && luhn(&s)
            }))))
        }
        "IS_SIREN" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(
                compact(&args[0]).is_some_and(|s| is_siren(&s)),
            )))
        }
        "IS_SIRET" => {
            check_args(name, args, 1)?;
            Ok(Some(Value::Bool(
                compact(&args[0]).is_some_and(|s| is_siret(&s)),
            )))
        }
        _ => Ok(None),
    }
}

/// A string with its spaces removed, or a non-negative integer as digits.
fn compact(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.chars().filter(|c| !c.is_whitespace()).collect()),
        Value::Number(n) => n.as_u64().map(|u| u.to_string()),
        _ => None,
    }
}

/// Luhn (mod 10) over ASCII digits, rightmost digit being the check digit.
fn luhn(digits: &str) -> bool {
    let sum: u32 = digits
        .bytes()
        .rev()
        .enumerate()
        .map(|(i, b)| {
            let d = u32::from(b - b'0');
            if i % 2 == 1 {
                let x = d * 2;
                if x > 9 {
                    x - 9
                } else {
                    x
                }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

fn is_siren(s: &str) -> bool {
    s.len() == 9 && s.bytes().all(|b| b.is_ascii_digit()) && luhn(s)
}

/// SIRET: 14 digits, Luhn — except La Poste (SIREN 356000000), whose
/// establishments are numbered past what Luhn allows and are checked by the
/// sum of their digits being a multiple of 5 instead.
fn is_siret(s: &str) -> bool {
    if s.len() != 14 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    if s.starts_with("356000000") && s != "35600000000048" {
        let sum: u32 = s.bytes().map(|b| u32::from(b - b'0')).sum();
        return sum.is_multiple_of(5);
    }
    luhn(s)
}

/// IBAN lengths by country (ISO 13616 registry, SWIFT release 2024).
const IBAN_LENGTHS: &[(&str, usize)] = &[
    ("AD", 24),
    ("AE", 23),
    ("AL", 28),
    ("AT", 20),
    ("AZ", 28),
    ("BA", 20),
    ("BE", 16),
    ("BG", 22),
    ("BH", 22),
    ("BI", 27),
    ("BR", 29),
    ("BY", 28),
    ("CH", 21),
    ("CR", 22),
    ("CY", 28),
    ("CZ", 24),
    ("DE", 22),
    ("DJ", 27),
    ("DK", 18),
    ("DO", 28),
    ("EE", 20),
    ("EG", 29),
    ("ES", 24),
    ("FI", 18),
    ("FK", 18),
    ("FO", 18),
    ("FR", 27),
    ("GB", 22),
    ("GE", 22),
    ("GI", 23),
    ("GL", 18),
    ("GR", 27),
    ("GT", 28),
    ("HN", 28),
    ("HR", 21),
    ("HU", 28),
    ("IE", 22),
    ("IL", 23),
    ("IQ", 23),
    ("IS", 26),
    ("IT", 27),
    ("JO", 30),
    ("KW", 30),
    ("KZ", 20),
    ("LB", 28),
    ("LC", 32),
    ("LI", 21),
    ("LT", 20),
    ("LU", 20),
    ("LV", 21),
    ("LY", 25),
    ("MC", 27),
    ("MD", 24),
    ("ME", 22),
    ("MK", 19),
    ("MN", 20),
    ("MR", 27),
    ("MT", 31),
    ("MU", 30),
    ("NI", 28),
    ("NL", 18),
    ("NO", 15),
    ("OM", 23),
    ("PK", 24),
    ("PL", 28),
    ("PS", 29),
    ("PT", 25),
    ("QA", 29),
    ("RO", 24),
    ("RS", 22),
    ("RU", 33),
    ("SA", 24),
    ("SC", 31),
    ("SD", 18),
    ("SE", 24),
    ("SI", 19),
    ("SK", 24),
    ("SM", 27),
    ("SO", 23),
    ("ST", 25),
    ("SV", 28),
    ("TL", 23),
    ("TN", 24),
    ("TR", 26),
    ("UA", 29),
    ("VA", 22),
    ("VG", 24),
    ("XK", 20),
    ("YE", 30),
];

/// IBAN: known country, that country's length, alphanumeric, and the
/// ISO 7064 mod-97 check (rearranged, letters as 10–35) equal to 1.
fn is_iban(s: &str) -> bool {
    let s = s.to_ascii_uppercase();
    if s.len() < 5 || !s.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    let country = &s[..2];
    let Some(&(_, len)) = IBAN_LENGTHS.iter().find(|(c, _)| *c == country) else {
        return false;
    };
    if s.len() != len || !s[2..4].bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let mut rem: u32 = 0;
    for b in s[4..].bytes().chain(s[..4].bytes()) {
        if b.is_ascii_digit() {
            rem = (rem * 10 + u32::from(b - b'0')) % 97;
        } else {
            rem = (rem * 100 + u32::from(b - b'A') + 10) % 97;
        }
    }
    rem == 1
}

fn check_args(name: &str, args: &[Value], expected: usize) -> DbResult<()> {
    if args.len() != expected {
        return Err(DbError::ExecutionError(format!(
            "{} requires {} argument(s)",
            name, expected
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn check(name: &str, v: Value) -> bool {
        evaluate(name, &[v]).unwrap().unwrap().as_bool().unwrap()
    }

    #[test]
    fn iban_validation() {
        assert!(check("IS_IBAN", json!("FR76 3000 6000 0112 3456 7890 189")));
        assert!(check("IS_IBAN", json!("de89370400440532013000")));
        assert!(check("IS_IBAN", json!("GB82WEST12345698765432")));
        assert!(
            !check("IS_IBAN", json!("FR76 3000 6000 0112 3456 7890 188")),
            "bad check digits"
        );
        assert!(
            !check("IS_IBAN", json!("FR76 3000 6000 0112 3456 7890 18")),
            "wrong length"
        );
        assert!(
            !check("IS_IBAN", json!("ZZ89370400440532013000")),
            "unknown country"
        );
        assert!(!check("IS_IBAN", json!("DE89-3704-0044-0532-0130-00")));
        assert!(!check("IS_IBAN", json!(null)));
        assert!(!check("IS_IBAN", json!(42)));
    }

    #[test]
    fn luhn_siren_siret() {
        assert!(check("LUHN", json!("4539 1488 0343 6467")));
        assert!(check("LUHN", json!(79927398713u64)));
        assert!(!check("LUHN", json!("79927398710")));
        assert!(!check("LUHN", json!("")));
        assert!(!check("LUHN", json!("12a4")));
        assert!(check("IS_SIREN", json!("732 829 320")));
        assert!(!check("IS_SIREN", json!("732829321")));
        assert!(!check("IS_SIREN", json!("73282932")));
        assert!(check("IS_SIRET", json!("732 829 320 00074")));
        assert!(!check("IS_SIRET", json!("73282932000075")));
        // La Poste establishments: digit sum a multiple of 5, not Luhn.
        assert!(check("IS_SIRET", json!("35600000049837")));
        assert!(check("IS_SIRET", json!("35600000000048")));
        assert!(!check("IS_SIRET", json!("35600000049838")));
    }

    fn call(name: &str, v: Value) -> Value {
        evaluate(name, &[v]).unwrap().unwrap()
    }

    #[test]
    fn date_predicates_accept_only_date_strings() {
        for f in ["IS_DATE", "IS_DATETIME", "IS_DATESTRING"] {
            assert_eq!(call(f, json!("2024-01-15T10:30:00Z")), json!(true), "{}", f);
            assert_eq!(call(f, json!("2024-01-15")), json!(true), "{}", f);
            assert_eq!(call(f, json!("nope")), json!(false), "{}", f);
            assert_eq!(call(f, json!(1_700_000_000)), json!(false), "{}", f);
            assert_eq!(call(f, Value::Null), json!(false), "{}", f);
        }
    }

    #[test]
    fn is_ipv4() {
        assert_eq!(call("IS_IPV4", json!("127.0.0.1")), json!(true));
        assert_eq!(call("IS_IPV4", json!("255.255.255.255")), json!(true));
        assert_eq!(call("IS_IPV4", json!("1.2.3.04")), json!(false));
        assert_eq!(call("IS_IPV4", json!("1.2.3")), json!(false));
        assert_eq!(call("IS_IPV4", json!("1.2.3.256")), json!(false));
        assert_eq!(call("IS_IPV4", json!(" 1.2.3.4")), json!(false));
        assert_eq!(call("IS_IPV4", json!(16909060)), json!(false));
    }
}
