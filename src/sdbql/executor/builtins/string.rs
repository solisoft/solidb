//! AQL-compatible string functions for SDBQL.
//!
//! Offsets and `LENGTH` are Unicode scalar counts (not bytes). Null
//! arguments propagate as JSON null. Regexes go through `safe_regex`
//! and a small process-wide compile cache.

use crate::error::{DbError, DbResult};
use crate::sdbql::executor::utils::safe_regex;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;

const MAX_REPEAT_BYTES: usize = 1_048_576;
const MAX_TOKEN_LEN: usize = 4096;
const REGEX_CACHE_CAP: usize = 128;

static REGEX_CACHE: Lazy<Mutex<HashMap<String, Regex>>> =
    Lazy::new(|| Mutex::new(HashMap::with_capacity(REGEX_CACHE_CAP)));

fn cached_regex(pattern: &str) -> DbResult<Regex> {
    {
        let cache = REGEX_CACHE.lock();
        if let Some(re) = cache.get(pattern) {
            return Ok(re.clone());
        }
    }
    let re = safe_regex(pattern)?;
    let mut cache = REGEX_CACHE.lock();
    if cache.len() >= REGEX_CACHE_CAP {
        cache.clear();
    }
    cache.insert(pattern.to_string(), re.clone());
    Ok(re)
}

fn null_if_any_null(args: &[Value]) -> bool {
    args.iter().any(Value::is_null)
}

fn require_str<'a>(name: &str, args: &'a [Value], i: usize) -> DbResult<&'a str> {
    args.get(i).and_then(Value::as_str).ok_or_else(|| {
        DbError::ExecutionError(format!("{}: argument {} must be a string", name, i + 1))
    })
}

fn as_i64(v: &Value, default: i64) -> i64 {
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .or_else(|| v.as_u64().map(|u| u as i64))
        .unwrap_or(default)
}

fn stringify(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// Convert a UTF-8 byte offset into a Unicode scalar index.
fn char_index_at_byte(s: &str, byte: usize) -> usize {
    s.get(..byte.min(s.len()))
        .map(|prefix| prefix.chars().count())
        .unwrap_or_else(|| s.chars().count())
}

fn nth_char_byte(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len())
}

/// `FIND_FIRST` / `CONTAINS(..., true)`: character index, or -1.
fn find_from(hay: &str, needle: &str, start_chars: i64) -> i64 {
    let n_chars = hay.chars().count() as i64;
    let start = if start_chars < 0 {
        (n_chars + start_chars).max(0) as usize
    } else {
        start_chars as usize
    };
    if needle.is_empty() {
        return start.min(n_chars as usize) as i64;
    }
    let byte_start = nth_char_byte(hay, start);
    match hay.get(byte_start..).and_then(|rest| rest.find(needle)) {
        Some(rel) => char_index_at_byte(hay, byte_start + rel) as i64,
        None => -1,
    }
}

fn rfind_from(hay: &str, needle: &str, start_chars: Option<i64>) -> i64 {
    let n_chars = hay.chars().count() as i64;
    let end_chars = match start_chars {
        Some(s) if s < 0 => (n_chars + s).max(0) as usize,
        Some(s) => (s as usize).min(n_chars as usize),
        None => n_chars as usize,
    };
    if needle.is_empty() {
        return end_chars as i64;
    }
    let byte_end = nth_char_byte(hay, end_chars);
    match hay.get(..byte_end).and_then(|head| head.rfind(needle)) {
        Some(pos) => char_index_at_byte(hay, pos) as i64,
        None => -1,
    }
}

fn substring(s: &str, start: i64, length: Option<i64>) -> String {
    if s.is_ascii() {
        let n = s.len() as i64;
        let mut start = start;
        if start < 0 {
            start += n;
        }
        if start < 0 {
            start = 0;
        }
        let start = start as usize;
        if start >= s.len() {
            return String::new();
        }
        let end = match length {
            Some(len) if len < 0 => start,
            Some(len) => start.saturating_add(len as usize).min(s.len()),
            None => s.len(),
        };
        return s[start..end].to_string();
    }
    let n = s.chars().count() as i64;
    let mut start = start;
    if start < 0 {
        start += n;
    }
    if start < 0 {
        start = 0;
    }
    let start = start as usize;
    if start >= n as usize {
        return String::new();
    }
    let take = match length {
        Some(len) if len < 0 => 0,
        Some(len) => len as usize,
        None => usize::MAX,
    };
    s.chars().skip(start).take(take).collect()
}

fn pad_to(s: &str, target: usize, pad: &str, left: bool) -> DbResult<String> {
    let current = s.chars().count();
    if current >= target {
        return Ok(s.to_string());
    }
    if pad.is_empty() {
        return Ok(s.to_string());
    }
    let need = target - current;
    let pad_chars: Vec<char> = pad.chars().collect();
    if pad_chars.is_empty() {
        return Ok(s.to_string());
    }
    let mut extra = String::new();
    extra.reserve(need);
    for i in 0..need {
        extra.push(pad_chars[i % pad_chars.len()]);
    }
    if extra.len() + s.len() > MAX_REPEAT_BYTES {
        return Err(DbError::ExecutionError(
            "PAD: result would exceed 1 MiB".to_string(),
        ));
    }
    Ok(if left {
        format!("{}{}", extra, s)
    } else {
        format!("{}{}", s, extra)
    })
}

fn like_to_regex(pattern: &str) -> String {
    let mut regex_pattern = String::from("^");
    for c in pattern.chars() {
        match c {
            '%' => regex_pattern.push_str(".*"),
            '_' => regex_pattern.push('.'),
            '^' | '$' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                regex_pattern.push('\\');
                regex_pattern.push(c);
            }
            _ => regex_pattern.push(c),
        }
    }
    regex_pattern.push('$');
    regex_pattern
}

fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{:02X}", b);
            }
        }
    }
    out
}

fn decode_uri_component(s: &str) -> String {
    let mut bytes = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = &s[i + 1..i + 3];
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    bytes.push(v);
                    i += 3;
                } else {
                    bytes.push(b[i]);
                    i += 1;
                }
            }
            b'+' => {
                bytes.push(b' ');
                i += 1;
            }
            other => {
                bytes.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Evaluate string functions. `Ok(None)` if `name` is not a string function.
#[allow(clippy::get_first)]
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "TOKENS" => {
            let text = args.first().and_then(Value::as_str).unwrap_or("");
            let analyzer = args.get(1).and_then(Value::as_str).unwrap_or("text_en");
            Ok(Some(Value::Array(
                tokens(text, analyzer)
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            )))
        }
        "PHRASE" => {
            if args.len() < 2 {
                return Err(err_arity("PHRASE", "2+"));
            }
            let text = args[0].as_str().unwrap_or("");
            let hay = tokens(text, "text_en");
            let needle: Vec<String> = args[1..]
                .iter()
                .flat_map(|v| match v {
                    Value::String(s) => tokens(s, "text_en"),
                    Value::Array(a) => a
                        .iter()
                        .filter_map(Value::as_str)
                        .flat_map(|s| tokens(s, "text_en"))
                        .collect(),
                    _ => vec![],
                })
                .collect();
            Ok(Some(Value::Bool(contains_phrase(&hay, &needle))))
        }
        "BOOST" => {
            if args.len() != 2 {
                return Err(err_arity("BOOST", "2"));
            }
            let base = match &args[0] {
                Value::Bool(true) => 1.0,
                Value::Bool(false) => 0.0,
                Value::Number(n) => n.as_f64().unwrap_or(0.0),
                _ => 0.0,
            };
            let f = args[1].as_f64().unwrap_or(1.0);
            Ok(Some(json!(base * f)))
        }
        "UPPER" | "TO_UPPER" | "TOUPPER" => {
            if null_if_any_null(args) {
                return Ok(Some(Value::Null));
            }
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            Ok(Some(Value::String(
                require_str(name, args, 0)?.to_uppercase(),
            )))
        }
        "LOWER" | "TO_LOWER" | "TOLOWER" => {
            if null_if_any_null(args) {
                return Ok(Some(Value::Null));
            }
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            Ok(Some(Value::String(
                require_str(name, args, 0)?.to_lowercase(),
            )))
        }
        "TRIM" => {
            if args.is_empty() || args.len() > 2 {
                return Err(err_arity(name, "1-2"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let value = require_str(name, args, 0)?;
            let result = if args.len() == 2 {
                if args[1].is_null() {
                    return Ok(Some(Value::Null));
                }
                if let Some(t) = args[1]
                    .as_i64()
                    .or_else(|| args[1].as_f64().map(|f| f as i64))
                {
                    match t {
                        1 => value.trim_start().to_string(),
                        2 => value.trim_end().to_string(),
                        _ => value.trim().to_string(),
                    }
                } else if let Some(chars) = args[1].as_str() {
                    value.trim_matches(|ch| chars.contains(ch)).to_string()
                } else {
                    value.trim().to_string()
                }
            } else {
                value.trim().to_string()
            };
            Ok(Some(Value::String(result)))
        }
        "LTRIM" => {
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            Ok(Some(Value::String(
                require_str(name, args, 0)?.trim_start().to_string(),
            )))
        }
        "RTRIM" => {
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            Ok(Some(Value::String(
                require_str(name, args, 0)?.trim_end().to_string(),
            )))
        }
        "CONCAT" | "CONCAT_WS" | "CONCAT_SEPARATOR" | "JOIN" => {
            if args.is_empty() {
                return Ok(Some(Value::String(String::new())));
            }
            if name == "JOIN" {
                let sep = args.get(1).and_then(Value::as_str).unwrap_or(",");
                return match &args[0] {
                    Value::Null => Ok(Some(Value::Null)),
                    Value::Array(arr) => Ok(Some(Value::String(
                        arr.iter().map(stringify).collect::<Vec<_>>().join(sep),
                    ))),
                    _ => Err(DbError::ExecutionError(
                        "JOIN requires an array as first argument".to_string(),
                    )),
                };
            }
            let separator: String;
            let items: &[Value] = match name {
                "CONCAT" => {
                    separator = String::new();
                    args
                }
                "CONCAT_WS" | "CONCAT_SEPARATOR" if !args.is_empty() => {
                    if args[0].is_null() {
                        return Ok(Some(Value::Null));
                    }
                    separator = args[0].as_str().unwrap_or("").to_string();
                    &args[1..]
                }
                _ => {
                    separator = String::new();
                    args
                }
            };
            let mut result_parts: Vec<String> = Vec::new();
            for v in items {
                match v {
                    Value::Array(arr) => {
                        for item in arr {
                            result_parts.push(stringify(item));
                        }
                    }
                    other => result_parts.push(stringify(other)),
                }
            }
            Ok(Some(Value::String(result_parts.join(&separator))))
        }
        "CONTAINS" => {
            // Arrays are handled by the array module (AQL CONTAINS).
            if args.first().is_some_and(Value::is_array) {
                return Ok(None);
            }
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, search, [returnIndex]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let haystack = require_str(name, args, 0)?;
            let needle = require_str(name, args, 1)?;
            let return_index = args.get(2).and_then(Value::as_bool).unwrap_or(false);
            if return_index {
                Ok(Some(Value::Number(serde_json::Number::from(find_from(
                    haystack, needle, 0,
                )))))
            } else {
                Ok(Some(Value::Bool(haystack.contains(needle))))
            }
        }
        "STARTS_WITH" => {
            if args.len() < 2 {
                return Err(err_arity(name, "2"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            Ok(Some(Value::Bool(
                require_str(name, args, 0)?.starts_with(require_str(name, args, 1)?),
            )))
        }
        "ENDS_WITH" => {
            if args.len() < 2 {
                return Err(err_arity(name, "2"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            Ok(Some(Value::Bool(
                require_str(name, args, 0)?.ends_with(require_str(name, args, 1)?),
            )))
        }
        "SPLIT" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, separator, [limit]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let value = require_str(name, args, 0)?;
            let separator = require_str(name, args, 1)?;
            let limit = args.get(2).map(|v| as_i64(v, 0));
            let parts: Vec<Value> = match limit {
                Some(n) if n > 0 => value
                    .splitn(n as usize, separator)
                    .map(|s| Value::String(s.to_string()))
                    .collect(),
                Some(n) if n < 0 => {
                    let mut p: Vec<Value> = value
                        .rsplitn(n.unsigned_abs() as usize, separator)
                        .map(|s| Value::String(s.to_string()))
                        .collect();
                    p.reverse();
                    p
                }
                _ if separator.is_empty() => value
                    .chars()
                    .map(|c| Value::String(c.to_string()))
                    .collect(),
                _ => value
                    .split(separator)
                    .map(|s| Value::String(s.to_string()))
                    .collect(),
            };
            Ok(Some(Value::Array(parts)))
        }
        "SUBSTRING" | "SUBSTR" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, start, [length]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = require_str(name, args, 0)?;
            let start = as_i64(&args[1], 0);
            let length = args.get(2).filter(|v| !v.is_null()).map(|v| as_i64(v, 0));
            Ok(Some(Value::String(substring(s, start, length))))
        }
        "REPLACE" => {
            if args.len() < 3 {
                return Err(err_arity(name, "3"));
            }
            if args[0].is_null() || args[1].is_null() || args[2].is_null() {
                return Ok(Some(Value::Null));
            }
            Ok(Some(Value::String(require_str(name, args, 0)?.replace(
                require_str(name, args, 1)?,
                require_str(name, args, 2)?,
            ))))
        }
        "SUBSTITUTE" => substitute(args),
        "LEFT" => {
            if args.len() < 2 {
                return Err(err_arity(name, "2"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let n = as_i64(&args[1], 0).max(0);
            Ok(Some(Value::String(substring(
                require_str(name, args, 0)?,
                0,
                Some(n),
            ))))
        }
        "RIGHT" => {
            if args.len() < 2 {
                return Err(err_arity(name, "2"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = require_str(name, args, 0)?;
            let n = as_i64(&args[1], 0).max(0);
            Ok(Some(Value::String(substring(s, -(n), None))))
        }
        "CHAR_LENGTH" | "CHARACTER_LENGTH" | "BYTE_LENGTH" => {
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = require_str(name, args, 0)?;
            let n = if name == "BYTE_LENGTH" {
                s.len()
            } else {
                s.chars().count()
            };
            Ok(Some(Value::Number(serde_json::Number::from(n))))
        }
        "REVERSE" if args.first().map(Value::is_string).unwrap_or(false) => {
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            Ok(Some(Value::String(
                require_str(name, args, 0)?.chars().rev().collect(),
            )))
        }
        "FIND_FIRST" | "FIND" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, search, [start]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let start = args.get(2).map(|v| as_i64(v, 0)).unwrap_or(0);
            Ok(Some(Value::Number(serde_json::Number::from(find_from(
                require_str(name, args, 0)?,
                require_str(name, args, 1)?,
                start,
            )))))
        }
        "FIND_LAST" | "RFIND" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, search, [end]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let end = args.get(2).map(|v| as_i64(v, 0));
            Ok(Some(Value::Number(serde_json::Number::from(rfind_from(
                require_str(name, args, 0)?,
                require_str(name, args, 1)?,
                end,
            )))))
        }
        "LIKE" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: text, pattern, [caseInsensitive]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let text = require_str(name, args, 0)?;
            let pattern = require_str(name, args, 1)?;
            let case_insensitive = args.get(2).and_then(Value::as_bool).unwrap_or(false);
            let mut re_pat = like_to_regex(pattern);
            if case_insensitive {
                re_pat = format!("(?i){}", re_pat);
            }
            let re = cached_regex(&re_pat)?;
            Ok(Some(Value::Bool(re.is_match(text))))
        }
        "REGEX_TEST" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, pattern, [caseInsensitive]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let mut pattern = require_str(name, args, 1)?.to_string();
            if args.get(2).and_then(Value::as_bool).unwrap_or(false) {
                pattern = format!("(?i){}", pattern);
            }
            let re = cached_regex(&pattern)?;
            Ok(Some(Value::Bool(re.is_match(require_str(name, args, 0)?))))
        }
        "REGEX_REPLACE" => {
            if args.len() < 3 || args.len() > 4 {
                return Err(err_arity(
                    name,
                    "3-4: text, pattern, replacement, [caseInsensitive]",
                ));
            }
            if args[0].is_null() || args[1].is_null() || args[2].is_null() {
                return Ok(Some(Value::Null));
            }
            let mut pattern = require_str(name, args, 1)?.to_string();
            if args.get(3).and_then(Value::as_bool).unwrap_or(false) {
                pattern = format!("(?i){}", pattern);
            }
            let re = cached_regex(&pattern)?;
            Ok(Some(Value::String(
                re.replace_all(require_str(name, args, 0)?, require_str(name, args, 2)?)
                    .into_owned(),
            )))
        }
        "REGEX_MATCHES" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, pattern, [caseInsensitive]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let mut pattern = require_str(name, args, 1)?.to_string();
            if args.get(2).and_then(Value::as_bool).unwrap_or(false) {
                pattern = format!("(?i){}", pattern);
            }
            let re = cached_regex(&pattern)?;
            let matches: Vec<Value> = re
                .find_iter(require_str(name, args, 0)?)
                .map(|m| Value::String(m.as_str().to_string()))
                .collect();
            Ok(Some(Value::Array(matches)))
        }
        "REGEX_SPLIT" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, pattern, [limit]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let re = cached_regex(require_str(name, args, 1)?)?;
            let text = require_str(name, args, 0)?;
            let parts: Vec<Value> = if let Some(limit) = args.get(2).map(|v| as_i64(v, 0)) {
                if limit > 0 {
                    re.splitn(text, limit as usize)
                        .map(|s| Value::String(s.to_string()))
                        .collect()
                } else {
                    re.split(text)
                        .map(|s| Value::String(s.to_string()))
                        .collect()
                }
            } else {
                re.split(text)
                    .map(|s| Value::String(s.to_string()))
                    .collect()
            };
            Ok(Some(Value::Array(parts)))
        }
        "REPEAT" => {
            if args.len() != 2 {
                return Err(err_arity(name, "2"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = require_str(name, args, 0)?;
            let n = as_i64(&args[1], 0).max(0) as usize;
            let total = s.len().saturating_mul(n);
            if total > MAX_REPEAT_BYTES {
                return Err(DbError::ExecutionError(format!(
                    "REPEAT: result would be {} bytes (max {})",
                    total, MAX_REPEAT_BYTES
                )));
            }
            Ok(Some(Value::String(s.repeat(n))))
        }
        "PAD_LEFT" | "LPAD" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let pad = args.get(2).and_then(Value::as_str).unwrap_or(" ");
            Ok(Some(Value::String(pad_to(
                require_str(name, args, 0)?,
                as_i64(&args[1], 0).max(0) as usize,
                pad,
                true,
            )?)))
        }
        "PAD_RIGHT" | "RPAD" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let pad = args.get(2).and_then(Value::as_str).unwrap_or(" ");
            Ok(Some(Value::String(pad_to(
                require_str(name, args, 0)?,
                as_i64(&args[1], 0).max(0) as usize,
                pad,
                false,
            )?)))
        }
        "CAPITALIZE" => {
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            match &args[0] {
                Value::Null => Ok(Some(Value::Null)),
                Value::String(s) => {
                    let mut chars = s.chars();
                    let result = match chars.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    };
                    Ok(Some(Value::String(result)))
                }
                _ => Err(DbError::ExecutionError(
                    "CAPITALIZE requires a string argument".to_string(),
                )),
            }
        }
        "TITLE_CASE" | "INITCAP" => {
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            match &args[0] {
                Value::Null => Ok(Some(Value::Null)),
                Value::String(s) => {
                    let result = s
                        .split_whitespace()
                        .map(|word| {
                            let mut chars = word.chars();
                            match chars.next() {
                                None => String::new(),
                                Some(first) => {
                                    first.to_uppercase().collect::<String>()
                                        + &chars.as_str().to_lowercase()
                                }
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    Ok(Some(Value::String(result)))
                }
                _ => Err(DbError::ExecutionError(
                    "TITLE_CASE requires a string argument".to_string(),
                )),
            }
        }
        "WORD_COUNT" => {
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let n = require_str(name, args, 0)?.split_whitespace().count();
            Ok(Some(Value::Number(serde_json::Number::from(n))))
        }
        "TRUNCATE_TEXT" => {
            if args.len() != 2 {
                return Err(err_arity(name, "2"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = require_str(name, args, 0)?;
            let len = as_i64(&args[1], 0).max(0) as usize;
            let count = s.chars().count();
            if count <= len {
                Ok(Some(Value::String(s.to_string())))
            } else {
                let cut: String = s.chars().take(len).collect();
                Ok(Some(Value::String(format!("{}...", cut))))
            }
        }
        "MASK" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, start, [end]"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = require_str(name, args, 0)?;
            let chars: Vec<char> = s.chars().collect();
            let n = chars.len() as i64;
            let start = as_i64(&args[1], 0).max(0) as usize;
            let end = args
                .get(2)
                .map(|v| {
                    let e = as_i64(v, -1);
                    if e < 0 {
                        (n + e).max(0) as usize
                    } else {
                        e as usize
                    }
                })
                .unwrap_or(chars.len().saturating_sub(1));
            let mut out = String::new();
            for (i, ch) in chars.iter().enumerate() {
                if i >= start && i < end {
                    out.push('*');
                } else {
                    out.push(*ch);
                }
            }
            Ok(Some(Value::String(out)))
        }
        "RANDOM_TOKEN" => {
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let n = as_i64(&args[0], 0).max(0) as usize;
            if n > MAX_TOKEN_LEN {
                return Err(DbError::ExecutionError(format!(
                    "RANDOM_TOKEN: n must be <= {}",
                    MAX_TOKEN_LEN
                )));
            }
            use rand::Rng;
            const CHARSET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
            let mut rng = rand::thread_rng();
            let s: String = (0..n)
                .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
                .collect();
            Ok(Some(Value::String(s)))
        }
        "ENCODE_URI" | "URL_ENCODE" | "ENCODE_URI_COMPONENT" => {
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            match &args[0] {
                Value::Null => Ok(Some(Value::Null)),
                Value::String(s) => Ok(Some(Value::String(encode_uri_component(s)))),
                _ => Err(DbError::ExecutionError(
                    "ENCODE_URI requires a string argument".to_string(),
                )),
            }
        }
        "DECODE_URI" | "URL_DECODE" | "DECODE_URI_COMPONENT" => {
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            match &args[0] {
                Value::Null => Ok(Some(Value::Null)),
                Value::String(s) => Ok(Some(Value::String(decode_uri_component(s)))),
                _ => Err(DbError::ExecutionError(
                    "DECODE_URI requires a string argument".to_string(),
                )),
            }
        }
        _ => Ok(None),
    }
}

fn err_arity(name: &str, expected: &str) -> DbError {
    DbError::ExecutionError(format!("{} requires {} argument(s)", name, expected))
}

fn substitute(args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() < 2 || args.len() > 4 {
        return Err(err_arity("SUBSTITUTE", "2-4"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let text = require_str("SUBSTITUTE", args, 0)?;
    let (limit, mapping_mode) = if args[1].is_object() {
        let limit = if args.len() == 3 {
            Some(as_i64(&args[2], 0))
        } else {
            None
        };
        (limit, true)
    } else {
        if args.len() < 3 {
            return Err(DbError::ExecutionError(
                "SUBSTITUTE requires search and replace strings".to_string(),
            ));
        }
        if args[1].is_null() || args[2].is_null() {
            return Ok(Some(Value::Null));
        }
        let limit = if args.len() == 4 {
            Some(as_i64(&args[3], 0))
        } else {
            None
        };
        (limit, false)
    };
    let count_limit = match limit {
        Some(n) if n > 0 => Some(n as usize),
        Some(_) => Some(0),
        None => None,
    };

    if mapping_mode {
        let mapping = args[1].as_object().unwrap();
        let mut result = text.to_string();
        for (search, replace_val) in mapping {
            let replace_str = if let Some(s) = replace_val.as_str() {
                s.to_string()
            } else {
                stringify(replace_val)
            };
            result = replace_limited(&result, search, &replace_str, count_limit);
        }
        Ok(Some(Value::String(result)))
    } else {
        Ok(Some(Value::String(replace_limited(
            text,
            require_str("SUBSTITUTE", args, 1)?,
            require_str("SUBSTITUTE", args, 2)?,
            count_limit,
        ))))
    }
}

fn replace_limited(text: &str, search: &str, replace: &str, limit: Option<usize>) -> String {
    match limit {
        None => text.replace(search, replace),
        Some(0) => text.to_string(),
        Some(limit_val) => {
            if search.is_empty() {
                return text.to_string();
            }
            let mut new_text = String::new();
            let mut last_end = 0;
            for (count, (start, part)) in text.match_indices(search).enumerate() {
                if count >= limit_val {
                    break;
                }
                new_text.push_str(&text[last_end..start]);
                new_text.push_str(replace);
                last_end = start + part.len();
            }
            new_text.push_str(&text[last_end..]);
            new_text
        }
    }
}

fn tokens(text: &str, analyzer: &str) -> Vec<String> {
    match analyzer {
        "identity" => {
            if text.is_empty() {
                vec![]
            } else {
                vec![text.to_string()]
            }
        }
        _ => {
            const STOP: &[&str] = &["a", "an", "the", "and", "or", "of", "to", "in"];
            text.split(|c: char| !c.is_alphanumeric())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_lowercase())
                .filter(|s| !STOP.contains(&s.as_str()))
                .collect()
        }
    }
}

fn contains_phrase(hay: &[String], needle: &[String]) -> bool {
    if needle.is_empty() {
        return true;
    }
    hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, args: &[Value]) -> Value {
        evaluate(name, args).unwrap().unwrap()
    }

    #[test]
    fn unicode_length_and_substring() {
        assert_eq!(call("CHAR_LENGTH", &[json!("café")]), json!(4));
        assert_eq!(call("BYTE_LENGTH", &[json!("café")]), json!(5));
        assert_eq!(
            call("SUBSTRING", &[json!("café"), json!(3), json!(1)]),
            json!("é")
        );
        assert_eq!(call("SUBSTRING", &[json!("hello"), json!(-2)]), json!("lo"));
    }

    #[test]
    fn find_uses_char_offsets() {
        // "é" is 2 bytes; character index of "x" is 1
        assert_eq!(call("FIND_FIRST", &[json!("éx"), json!("x")]), json!(1));
        assert_eq!(
            call("CONTAINS", &[json!("éx"), json!("x"), json!(true)]),
            json!(1)
        );
        assert_eq!(
            call("FIND_FIRST", &[json!("ababa"), json!("ba"), json!(2)]),
            json!(3)
        );
    }

    #[test]
    fn like_and_regex() {
        assert_eq!(call("LIKE", &[json!("hello"), json!("h%llo")]), json!(true));
        assert_eq!(
            call("LIKE", &[json!("Hello"), json!("hello"), json!(true)]),
            json!(true)
        );
        assert_eq!(
            call("REGEX_MATCHES", &[json!("a1b2"), json!(r"\d")]),
            json!(["1", "2"])
        );
        assert_eq!(
            call("REGEX_SPLIT", &[json!("a,b,c"), json!(",")]),
            json!(["a", "b", "c"])
        );
    }

    #[test]
    fn pad_repeat_mask() {
        assert_eq!(
            call("PAD_LEFT", &[json!("1"), json!(3), json!("0")]),
            json!("001")
        );
        assert_eq!(call("REPEAT", &[json!("ab"), json!(3)]), json!("ababab"));
        assert_eq!(
            call("MASK", &[json!("12345"), json!(1), json!(-1)]),
            json!("1***5")
        );
        assert_eq!(
            call("TRUNCATE_TEXT", &[json!("Hello World"), json!(5)]),
            json!("Hello...")
        );
    }

    #[test]
    fn uri_roundtrip_multibyte() {
        let encoded = call("ENCODE_URI", &[json!("é")]);
        assert_eq!(encoded, json!("%C3%A9"));
        assert_eq!(call("DECODE_URI", &[encoded]), json!("é"));
    }

    #[test]
    fn null_propagates() {
        assert_eq!(call("UPPER", &[Value::Null]), Value::Null);
        assert_eq!(call("CONTAINS", &[Value::Null, json!("a")]), Value::Null);
    }
}
