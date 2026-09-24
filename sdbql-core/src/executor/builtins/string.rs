//! String builtin functions.
//!
//! Semantics follow AQL (and the server): a null first argument yields null,
//! positions are character offsets (never byte offsets), and every function
//! that can grow its input is capped at [`MAX_REPEAT_BYTES`].

use serde_json::{json, Value};

use super::common::{check_arity, err, int_arg, length_of, stringify, MAX_REPEAT_BYTES};
use crate::error::SdbqlResult;
use crate::executor::helpers::{compile_regex, levenshtein_distance, like_to_regex};

/// Inputs longer than this are not compared by LEVENSHTEIN (same as the
/// server); the result is the longer input's length.
const MAX_LEVENSHTEIN_CHARS: usize = 4096;

fn text_arg(args: &[Value], i: usize) -> String {
    args.get(i).map(stringify).unwrap_or_default()
}

fn too_long(name: &str, bytes: usize) -> crate::error::SdbqlError {
    err(format!(
        "{}: result would be {} bytes (max {})",
        name, bytes, MAX_REPEAT_BYTES
    ))
}

/// Byte offset of the `n`-th character (or the string's length).
fn nth_char_byte(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len())
}

/// Character index of a byte offset that lies on a char boundary.
fn char_index_at_byte(s: &str, byte: usize) -> usize {
    s.get(..byte.min(s.len()))
        .map(|p| p.chars().count())
        .unwrap_or_else(|| s.chars().count())
}

/// Resolve a possibly negative character position against `n` characters.
fn resolve_pos(pos: i64, n: usize) -> usize {
    let n_i = n as i64;
    if pos < 0 {
        n_i.saturating_add(pos).max(0) as usize
    } else {
        (pos as usize).min(n)
    }
}

/// AQL SUBSTRING: negative `start` counts from the end; a negative
/// `length` yields "".
pub(super) fn substring(s: &str, start: i64, length: Option<i64>) -> String {
    let n = s.chars().count();
    let start = resolve_pos(start, n);
    if start >= n {
        return String::new();
    }
    let take = match length {
        Some(len) if len < 0 => 0,
        Some(len) => len as usize,
        None => usize::MAX,
    };
    s.chars().skip(start).take(take).collect()
}

/// Character window `[start, end]` (inclusive, AQL) as byte offsets.
fn window(s: &str, start: Option<i64>, end: Option<i64>) -> (usize, usize) {
    let n = s.chars().count();
    let from = start.map(|p| resolve_pos(p, n)).unwrap_or(0);
    let to = match end {
        // `end` is an inclusive character position.
        Some(e) => {
            let e = if e < 0 {
                (n as i64).saturating_add(e)
            } else {
                e
            };
            if e < 0 {
                0
            } else {
                (e as usize).saturating_add(1).min(n)
            }
        }
        None => n,
    };
    (nth_char_byte(s, from), nth_char_byte(s, to.max(from)))
}

/// FIND_FIRST: character index of the first match inside the window, or -1.
fn find_first(hay: &str, needle: &str, start: Option<i64>, end: Option<i64>) -> i64 {
    let (b0, b1) = window(hay, start, end);
    let slice = &hay[b0..b1];
    match slice.find(needle) {
        Some(rel) => char_index_at_byte(hay, b0 + rel) as i64,
        None => -1,
    }
}

/// FIND_LAST: character index of the last match inside the window, or -1.
fn find_last(hay: &str, needle: &str, start: Option<i64>, end: Option<i64>) -> i64 {
    let (b0, b1) = window(hay, start, end);
    let slice = &hay[b0..b1];
    match slice.rfind(needle) {
        Some(rel) => char_index_at_byte(hay, b0 + rel) as i64,
        None => -1,
    }
}

/// PAD_LEFT / PAD_RIGHT: pad by characters (a multibyte pad is fine), an
/// empty pad leaves the string alone, the result is capped.
fn pad_to(name: &str, s: &str, target: usize, pad: &str, left: bool) -> SdbqlResult<String> {
    let current = s.chars().count();
    let pad_chars: Vec<char> = pad.chars().collect();
    if current >= target || pad_chars.is_empty() {
        return Ok(s.to_string());
    }
    let need = target - current;
    // Every pad char is at least one byte: reject before allocating.
    if need.saturating_add(s.len()) > MAX_REPEAT_BYTES {
        return Err(too_long(name, need.saturating_add(s.len())));
    }
    let extra: String = pad_chars.iter().cycle().take(need).collect();
    if extra.len() + s.len() > MAX_REPEAT_BYTES {
        return Err(too_long(name, extra.len() + s.len()));
    }
    Ok(if left {
        format!("{}{}", extra, s)
    } else {
        format!("{}{}", s, extra)
    })
}

fn regex_with_flags(pattern: &str, case_insensitive: bool) -> SdbqlResult<regex::Regex> {
    if case_insensitive {
        compile_regex(&format!("(?i){}", pattern))
    } else {
        compile_regex(pattern)
    }
}

/// `replace_all` that stops as soon as the output passes the cap, so an
/// empty-matching pattern cannot build a huge string.
fn regex_replace_capped(re: &regex::Regex, s: &str, rep: &str) -> SdbqlResult<String> {
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for caps in re.captures_iter(s) {
        let Some(m) = caps.get(0) else { continue };
        out.push_str(&s[last..m.start()]);
        caps.expand(rep, &mut out);
        last = m.end();
        if out.len() > MAX_REPEAT_BYTES {
            return Err(too_long("REGEX_REPLACE", out.len()));
        }
    }
    out.push_str(&s[last..]);
    if out.len() > MAX_REPEAT_BYTES {
        return Err(too_long("REGEX_REPLACE", out.len()));
    }
    Ok(out)
}

fn trim_chars(s: &str, chars: &str, left: bool, right: bool) -> String {
    let pred = |c: char| chars.contains(c);
    match (left, right) {
        (true, true) => s.trim_matches(pred).to_string(),
        (true, false) => s.trim_start_matches(pred).to_string(),
        (false, true) => s.trim_end_matches(pred).to_string(),
        (false, false) => s.to_string(),
    }
}

fn trim_ws(s: &str, left: bool, right: bool) -> String {
    match (left, right) {
        (true, true) => s.trim().to_string(),
        (true, false) => s.trim_start().to_string(),
        (false, true) => s.trim_end().to_string(),
        (false, false) => s.to_string(),
    }
}

/// Items for CONCAT / CONCAT_SEPARATOR: array arguments are flattened one
/// level, nulls are skipped at both levels.
fn concat_items(items: &[Value]) -> Vec<String> {
    let mut parts = Vec::new();
    for v in items {
        match v {
            Value::Null => {}
            Value::Array(arr) => parts.extend(arr.iter().filter(|x| !x.is_null()).map(stringify)),
            other => parts.push(stringify(other)),
        }
    }
    parts
}

/// Call a string function. Returns None if function not found.
pub fn call(name: &str, args: &[Value]) -> SdbqlResult<Option<Value>> {
    let result = match name {
        "TOKENS" => {
            let text = args.first().and_then(Value::as_str).unwrap_or("");
            let analyzer = args.get(1).and_then(Value::as_str).unwrap_or("text_en");
            let toks = match analyzer {
                "identity" => {
                    if text.is_empty() {
                        vec![]
                    } else {
                        vec![text.to_string()]
                    }
                }
                _ => text
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_lowercase())
                    .filter(|s| {
                        !matches!(
                            s.as_str(),
                            "a" | "an" | "the" | "and" | "or" | "of" | "to" | "in"
                        )
                    })
                    .collect(),
            };
            Some(Value::Array(toks.into_iter().map(Value::String).collect()))
        }
        "BOOST" => {
            let base = match args.first() {
                Some(Value::Bool(true)) => 1.0,
                Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
                _ => 0.0,
            };
            let f = args.get(1).and_then(Value::as_f64).unwrap_or(1.0);
            Some(json!(base * f))
        }
        "PHRASE" => {
            let text = args.first().and_then(Value::as_str).unwrap_or("");
            let hay: Vec<String> = text
                .split(|c: char| !c.is_alphanumeric())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_lowercase())
                .collect();
            // `args[1..]` panicked on a call with no arguments.
            let needle: Vec<String> = args
                .get(1..)
                .unwrap_or(&[])
                .iter()
                .filter_map(Value::as_str)
                .flat_map(|s| {
                    s.split(|c: char| !c.is_alphanumeric())
                        .filter(|t| !t.is_empty())
                        .map(|t| t.to_lowercase())
                })
                .collect();
            Some(Value::Bool(
                !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle),
            ))
        }
        "UPPER" | "TO_UPPER" | "TOUPPER" => {
            check_arity(name, args, 1, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            Some(Value::String(text_arg(args, 0).to_uppercase()))
        }

        "LOWER" | "TO_LOWER" | "TOLOWER" => {
            check_arity(name, args, 1, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            Some(Value::String(text_arg(args, 0).to_lowercase()))
        }

        "TRIM" => {
            check_arity(name, args, 1, 2)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = text_arg(args, 0);
            let out = match args.get(1) {
                None | Some(Value::Null) => trim_ws(&s, true, true),
                Some(Value::String(chars)) => trim_chars(&s, chars, true, true),
                Some(v) => match int_arg(std::slice::from_ref(v), 0, 0) {
                    1 => trim_ws(&s, true, false),
                    2 => trim_ws(&s, false, true),
                    _ => trim_ws(&s, true, true),
                },
            };
            Some(Value::String(out))
        }

        "LTRIM" | "RTRIM" => {
            check_arity(name, args, 1, 2)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = text_arg(args, 0);
            let left = name == "LTRIM";
            let out = match args.get(1).and_then(Value::as_str) {
                Some(chars) => trim_chars(&s, chars, left, !left),
                None => trim_ws(&s, left, !left),
            };
            Some(Value::String(out))
        }

        "LENGTH" => {
            check_arity(name, args, 1, 1)?;
            Some(Value::from(length_of(&args[0])))
        }

        "CHAR_LENGTH" | "CHARACTER_LENGTH" | "CHAR_COUNT" | "BYTE_LENGTH" => {
            check_arity(name, args, 1, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = text_arg(args, 0);
            let n = if name == "BYTE_LENGTH" {
                s.len()
            } else {
                s.chars().count()
            };
            Some(Value::from(n))
        }

        "CONCAT" => Some(Value::String(concat_items(args).concat())),

        "CONCAT_SEPARATOR" | "CONCAT_WS" => {
            if args.is_empty() {
                return Ok(Some(Value::String(String::new())));
            }
            let separator = stringify(&args[0]);
            Some(Value::String(concat_items(&args[1..]).join(&separator)))
        }

        // Arrays are handled by the array module (AQL CONTAINS on arrays).
        "CONTAINS" if args.first().is_some_and(Value::is_array) => None,

        "CONTAINS" => {
            check_arity(name, args, 2, 3)?;
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let hay = text_arg(args, 0);
            let needle = text_arg(args, 1);
            if args.get(2).and_then(Value::as_bool).unwrap_or(false) {
                Some(Value::from(find_first(&hay, &needle, None, None)))
            } else {
                Some(Value::Bool(hay.contains(needle.as_str())))
            }
        }

        "STARTS_WITH" | "ENDS_WITH" => {
            check_arity(name, args, 2, 2)?;
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = text_arg(args, 0);
            let test = |p: &str| {
                if name == "STARTS_WITH" {
                    s.starts_with(p)
                } else {
                    s.ends_with(p)
                }
            };
            let hit = match &args[1] {
                // AQL: an array of prefixes matches if any of them does.
                Value::Array(list) => list.iter().any(|p| test(&stringify(p))),
                other => test(&stringify(other)),
            };
            Some(Value::Bool(hit))
        }

        "SUBSTRING" | "SUBSTR" => {
            check_arity(name, args, 2, 3)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let length = args
                .get(2)
                .filter(|v| !v.is_null())
                .map(|v| int_arg(std::slice::from_ref(v), 0, 0));
            Some(Value::String(substring(
                &text_arg(args, 0),
                int_arg(args, 1, 0),
                length,
            )))
        }

        "LEFT" => {
            check_arity(name, args, 2, 2)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let n = int_arg(args, 1, 0).max(0);
            Some(Value::String(substring(&text_arg(args, 0), 0, Some(n))))
        }

        "RIGHT" => {
            check_arity(name, args, 2, 2)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = text_arg(args, 0);
            let n = int_arg(args, 1, 0).max(0) as usize;
            let total = s.chars().count();
            let skip = total.saturating_sub(n);
            Some(Value::String(s.chars().skip(skip).collect()))
        }

        "SPLIT" => {
            // AQL SPLIT(value, separator, limit): no separator returns the
            // value wrapped in an array, "" splits into characters, an array
            // of separators splits on any of them, and `limit` truncates.
            check_arity(name, args, 1, 3)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = text_arg(args, 0);
            let limit = args
                .get(2)
                .filter(|v| !v.is_null())
                .map(|v| int_arg(std::slice::from_ref(v), 0, 0))
                .filter(|&n| n > 0)
                .map(|n| n as usize)
                .unwrap_or(usize::MAX);
            let parts: Vec<String> = match args.get(1) {
                None | Some(Value::Null) => vec![s],
                Some(Value::Array(seps)) => {
                    let seps: Vec<String> = seps
                        .iter()
                        .map(stringify)
                        .filter(|p| !p.is_empty())
                        .collect();
                    split_multi(&s, &seps)
                }
                Some(sep) => {
                    let sep = stringify(sep);
                    if sep.is_empty() {
                        s.chars().map(|c| c.to_string()).collect()
                    } else {
                        s.split(sep.as_str()).map(str::to_string).collect()
                    }
                }
            };
            Some(Value::Array(
                parts.into_iter().take(limit).map(Value::String).collect(),
            ))
        }

        "JOIN" => {
            check_arity(name, args, 1, 2)?;
            match &args[0] {
                Value::Null => Some(Value::Null),
                Value::Array(arr) => {
                    let separator = args.get(1).map(stringify).unwrap_or_else(|| ",".into());
                    let parts: Vec<String> =
                        arr.iter().filter(|v| !v.is_null()).map(stringify).collect();
                    Some(Value::String(parts.join(&separator)))
                }
                _ => return Err(err("JOIN: first argument must be an array")),
            }
        }

        "REVERSE" => {
            check_arity(name, args, 1, 1)?;
            match &args[0] {
                Value::Null => Some(Value::Null),
                Value::Array(arr) => Some(Value::Array(arr.iter().rev().cloned().collect())),
                other => Some(Value::String(stringify(other).chars().rev().collect())),
            }
        }

        "REPLACE" => {
            check_arity(name, args, 3, 3)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = text_arg(args, 0);
            let search = text_arg(args, 1);
            let replace = text_arg(args, 2);
            // AQL returns the input for an empty search string; replacing
            // between every character is also how the output exploded.
            if search.is_empty() {
                return Ok(Some(Value::String(s)));
            }
            let count = s.matches(search.as_str()).count();
            let size = (s.len() - count * search.len())
                .saturating_add(count.saturating_mul(replace.len()));
            if size > MAX_REPEAT_BYTES {
                return Err(too_long(name, size));
            }
            Some(Value::String(s.replace(search.as_str(), &replace)))
        }

        "REGEX_REPLACE" => {
            check_arity(name, args, 3, 4)?;
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let ci = args.get(3).and_then(Value::as_bool).unwrap_or(false);
            let re = regex_with_flags(&text_arg(args, 1), ci)?;
            Some(Value::String(regex_replace_capped(
                &re,
                &text_arg(args, 0),
                &text_arg(args, 2),
            )?))
        }

        "REGEX_TEST" | "REGEX_MATCH" => {
            check_arity(name, args, 2, 3)?;
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let ci = args.get(2).and_then(Value::as_bool).unwrap_or(false);
            let re = regex_with_flags(&text_arg(args, 1), ci)?;
            Some(Value::Bool(re.is_match(&text_arg(args, 0))))
        }

        "REGEX_MATCHES" => {
            check_arity(name, args, 2, 3)?;
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let ci = args.get(2).and_then(Value::as_bool).unwrap_or(false);
            let re = regex_with_flags(&text_arg(args, 1), ci)?;
            let s = text_arg(args, 0);
            let matches: Vec<Value> = re
                .find_iter(&s)
                .map(|m| Value::String(m.as_str().to_string()))
                .collect();
            Some(Value::Array(matches))
        }

        "REGEX_SPLIT" => {
            check_arity(name, args, 2, 3)?;
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let re = compile_regex(&text_arg(args, 1))?;
            let s = text_arg(args, 0);
            let limit = args
                .get(2)
                .map(|v| int_arg(std::slice::from_ref(v), 0, 0))
                .filter(|&n| n > 0)
                .map(|n| n as usize)
                .unwrap_or(usize::MAX);
            Some(Value::Array(
                re.split(&s)
                    .take(limit)
                    .map(|p| Value::String(p.to_string()))
                    .collect(),
            ))
        }

        "LIKE" => {
            check_arity(name, args, 2, 3)?;
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let ci = args.get(2).and_then(Value::as_bool).unwrap_or(false);
            let re = regex_with_flags(&like_to_regex(&text_arg(args, 1)), ci)?;
            Some(Value::Bool(re.is_match(&text_arg(args, 0))))
        }

        "TO_STRING" | "TO_STR" => {
            check_arity(name, args, 1, 1)?;
            Some(Value::String(stringify(&args[0])))
        }

        "REPEAT" => {
            // AQL REPEAT(value, count, separator). A count <= 0 is "".
            check_arity(name, args, 2, 3)?;
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = text_arg(args, 0);
            let n = int_arg(args, 1, 0);
            if n <= 0 {
                return Ok(Some(Value::String(String::new())));
            }
            let n = n as usize;
            let sep = args.get(2).map(stringify).unwrap_or_default();
            let total = s
                .len()
                .saturating_mul(n)
                .saturating_add(sep.len().saturating_mul(n - 1));
            if total > MAX_REPEAT_BYTES {
                return Err(too_long(name, total));
            }
            let mut out = String::with_capacity(total);
            for i in 0..n {
                if i > 0 {
                    out.push_str(&sep);
                }
                out.push_str(&s);
            }
            Some(Value::String(out))
        }

        "PAD_LEFT" | "LPAD" | "PAD_RIGHT" | "RPAD" => {
            check_arity(name, args, 2, 3)?;
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let pad = match args.get(2) {
                None | Some(Value::Null) => " ".to_string(),
                Some(v) => stringify(v),
            };
            let target = int_arg(args, 1, 0).max(0) as usize;
            let left = matches!(name, "PAD_LEFT" | "LPAD");
            Some(Value::String(pad_to(
                name,
                &text_arg(args, 0),
                target,
                &pad,
                left,
            )?))
        }

        "FIND_FIRST" | "FIND" | "FIND_LAST" | "RFIND" => {
            check_arity(name, args, 2, 4)?;
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let opt = |i: usize| {
                args.get(i)
                    .filter(|v| !v.is_null())
                    .map(|v| int_arg(std::slice::from_ref(v), 0, 0))
            };
            let hay = text_arg(args, 0);
            let needle = text_arg(args, 1);
            let pos = if matches!(name, "FIND_FIRST" | "FIND") {
                find_first(&hay, &needle, opt(2), opt(3))
            } else {
                find_last(&hay, &needle, opt(2), opt(3))
            };
            Some(Value::from(pos))
        }

        "CAPITALIZE" => {
            check_arity(name, args, 1, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = text_arg(args, 0);
            let mut chars = s.chars();
            let out = match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            };
            Some(Value::String(out))
        }

        "TITLE_CASE" | "INITCAP" => {
            check_arity(name, args, 1, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = text_arg(args, 0);
            let mut out = String::with_capacity(s.len());
            let mut at_word_start = true;
            for c in s.chars() {
                if c.is_alphanumeric() {
                    if at_word_start {
                        out.extend(c.to_uppercase());
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    at_word_start = false;
                } else {
                    out.push(c);
                    at_word_start = true;
                }
            }
            Some(Value::String(out))
        }

        "WORD_COUNT" => {
            check_arity(name, args, 1, 1)?;
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            Some(Value::from(text_arg(args, 0).split_whitespace().count()))
        }

        "LEVENSHTEIN" | "LEVENSHTEIN_DISTANCE" => {
            check_arity(name, args, 2, 2)?;
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let a = text_arg(args, 0);
            let b = text_arg(args, 1);
            let (la, lb) = (a.chars().count(), b.chars().count());
            // O(n·m): refuse to compare very long inputs.
            if la > MAX_LEVENSHTEIN_CHARS || lb > MAX_LEVENSHTEIN_CHARS {
                return Ok(Some(Value::from(la.max(lb))));
            }
            Some(Value::from(levenshtein_distance(&a, &b)))
        }

        _ => None,
    };

    Ok(result)
}

/// Split on any of `seps`, leftmost-first, preferring the longest separator
/// at a position.
fn split_multi(s: &str, seps: &[String]) -> Vec<String> {
    if seps.is_empty() {
        return vec![s.to_string()];
    }
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let hit = seps
            .iter()
            .filter(|p| rest.starts_with(p.as_str()))
            .map(|p| p.len())
            .max();
        match hit {
            Some(len) => {
                parts.push(s[start..i].to_string());
                i += len;
                start = i;
            }
            None => {
                // Advance by one character, never into the middle of one.
                i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
            }
        }
    }
    parts.push(s[start..].to_string());
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ok(name: &str, args: &[Value]) -> Value {
        call(name, args).unwrap().unwrap()
    }

    #[test]
    fn test_upper_lower() {
        assert_eq!(
            call("UPPER", &[json!("hello")]).unwrap(),
            Some(json!("HELLO"))
        );
        assert_eq!(
            call("LOWER", &[json!("HELLO")]).unwrap(),
            Some(json!("hello"))
        );
        assert_eq!(ok("UPPER", &[Value::Null]), Value::Null);
    }

    #[test]
    fn test_trim() {
        assert_eq!(
            call("TRIM", &[json!("  hello  ")]).unwrap(),
            Some(json!("hello"))
        );
        assert_eq!(
            call("LTRIM", &[json!("  hello")]).unwrap(),
            Some(json!("hello"))
        );
        assert_eq!(
            call("RTRIM", &[json!("hello  ")]).unwrap(),
            Some(json!("hello"))
        );
        assert_eq!(ok("TRIM", &[json!("--foo--"), json!("-")]), json!("foo"));
        assert_eq!(ok("TRIM", &[json!(" foo "), json!(1)]), json!("foo "));
        assert_eq!(ok("LTRIM", &[json!("xxfoo"), json!("x")]), json!("foo"));
    }

    #[test]
    fn test_concat() {
        assert_eq!(
            call("CONCAT", &[json!("hello"), json!(" "), json!("world")]).unwrap(),
            Some(json!("hello world"))
        );
        assert_eq!(
            ok("CONCAT", &[json!("a"), Value::Null, json!(2.0)]),
            json!("a2")
        );
        assert_eq!(ok("CONCAT", &[json!(["a", null, "b"])]), json!("ab"));
    }

    #[test]
    fn concat_separator_skips_nulls() {
        assert_eq!(
            ok(
                "CONCAT_SEPARATOR",
                &[
                    json!(", "),
                    json!("a"),
                    Value::Null,
                    json!(["b", null, "c"])
                ]
            ),
            json!("a, b, c")
        );
    }

    #[test]
    fn test_split_join() {
        assert_eq!(
            call("SPLIT", &[json!("a,b,c"), json!(",")]).unwrap(),
            Some(json!(["a", "b", "c"]))
        );
        assert_eq!(
            call("JOIN", &[json!(["a", "b", "c"]), json!("-")]).unwrap(),
            Some(json!("a-b-c"))
        );
    }

    #[test]
    fn split_follows_aql() {
        // No separator: the value wrapped in an array (was: split on ",").
        assert_eq!(ok("SPLIT", &[json!("a,b")]), json!(["a,b"]));
        assert_eq!(
            ok("SPLIT", &[json!("abc"), json!("")]),
            json!(["a", "b", "c"])
        );
        // The limit truncates, it does not keep the remainder.
        assert_eq!(
            ok("SPLIT", &[json!("a-b-c"), json!("-"), json!(2)]),
            json!(["a", "b"])
        );
        assert_eq!(
            ok("SPLIT", &[json!("a-b_c"), json!(["-", "_"])]),
            json!(["a", "b", "c"])
        );
        assert_eq!(ok("SPLIT", &[Value::Null, json!(",")]), Value::Null);
    }

    #[test]
    fn test_substring() {
        assert_eq!(
            call("SUBSTRING", &[json!("hello world"), json!(0), json!(5)]).unwrap(),
            Some(json!("hello"))
        );
        assert_eq!(
            call("SUBSTRING", &[json!("hello"), json!(2)]).unwrap(),
            Some(json!("llo"))
        );
    }

    #[test]
    fn substring_negative_start_counts_from_end() {
        assert_eq!(ok("SUBSTRING", &[json!("hello"), json!(-3)]), json!("llo"));
        assert_eq!(
            ok("SUBSTRING", &[json!("hello"), json!(-3), json!(2)]),
            json!("ll")
        );
        assert_eq!(
            ok("SUBSTRING", &[json!("hello"), json!(-99)]),
            json!("hello")
        );
        assert_eq!(
            ok("SUBSTRING", &[json!("hello"), json!(1), json!(-1)]),
            json!("")
        );
        assert_eq!(
            ok("SUBSTRING", &[json!("héllo"), json!(1), json!(2)]),
            json!("él")
        );
        assert_eq!(
            ok(
                "SUBSTRING",
                &[json!("abc"), json!(i64::MIN), json!(i64::MAX)]
            ),
            json!("abc")
        );
    }

    #[test]
    fn test_contains() {
        assert_eq!(
            call("CONTAINS", &[json!("hello world"), json!("world")]).unwrap(),
            Some(json!(true))
        );
        assert_eq!(
            call("CONTAINS", &[json!("hello"), json!("xyz")]).unwrap(),
            Some(json!(false))
        );
        assert_eq!(
            ok("CONTAINS", &[json!("héllo"), json!("llo"), json!(true)]),
            json!(2)
        );
    }

    #[test]
    fn test_replace() {
        assert_eq!(
            call(
                "REPLACE",
                &[json!("hello world"), json!("world"), json!("there")]
            )
            .unwrap(),
            Some(json!("hello there"))
        );
    }

    #[test]
    fn replace_is_capped_and_empty_search_is_identity() {
        assert_eq!(
            ok("REPLACE", &[json!("abc"), json!(""), json!("x")]),
            json!("abc")
        );
        let big = "a".repeat(2000);
        assert!(call(
            "REPLACE",
            &[json!(big), json!("a"), json!("x".repeat(1000))]
        )
        .is_err());
    }

    #[test]
    fn regex_replace_is_capped() {
        let big = "a".repeat(2000);
        assert!(call(
            "REGEX_REPLACE",
            &[json!(big), json!(""), json!("x".repeat(1000))]
        )
        .is_err());
        assert_eq!(
            ok(
                "REGEX_REPLACE",
                &[
                    json!("the quick fox"),
                    json!("the (.*) fox"),
                    json!("a $1 dog")
                ]
            ),
            json!("a quick dog")
        );
    }

    #[test]
    fn invalid_regex_is_an_error() {
        assert!(call("REGEX_TEST", &[json!("abc"), json!("(")]).is_err());
        assert!(call("REGEX_MATCHES", &[json!("abc"), json!("[")]).is_err());
        assert!(call("REGEX_REPLACE", &[json!("abc"), json!("("), json!("")]).is_err());
        assert!(call("REGEX_TEST", &[json!("abc"), json!("a".repeat(2000))]).is_err());
        // A pattern whose compiled program exceeds the size limit.
        assert!(call("REGEX_TEST", &[json!("abc"), json!("(\\w{1000}){1000}")]).is_err());
    }

    #[test]
    fn repeat_is_capped_and_negative_is_empty() {
        assert_eq!(ok("REPEAT", &[json!("ab"), json!(3)]), json!("ababab"));
        assert_eq!(ok("REPEAT", &[json!("ab"), json!(-1)]), json!(""));
        assert_eq!(ok("REPEAT", &[json!("ab"), json!(i64::MIN)]), json!(""));
        assert_eq!(
            ok("REPEAT", &[json!("a"), json!(3), json!(",")]),
            json!("a,a,a")
        );
        assert!(call("REPEAT", &[json!("ab"), json!(i64::MAX)]).is_err());
        assert!(call("REPEAT", &[json!("x"), json!(2_000_000)]).is_err());
    }

    #[test]
    fn pad_never_panics() {
        assert_eq!(
            ok("PAD_LEFT", &[json!("42"), json!(5), json!("0")]),
            json!("00042")
        );
        assert_eq!(ok("PAD_RIGHT", &[json!("hi"), json!(5)]), json!("hi   "));
        // Empty pad used to divide by zero.
        assert_eq!(
            ok("PAD_LEFT", &[json!("hi"), json!(5), json!("")]),
            json!("hi")
        );
        // Multibyte pad used to slice inside a character.
        assert_eq!(
            ok("PAD_LEFT", &[json!("x"), json!(4), json!("é")]),
            json!("éééx")
        );
        assert_eq!(
            ok("PAD_RIGHT", &[json!("x"), json!(4), json!("ab")]),
            json!("xaba")
        );
        assert!(call("PAD_LEFT", &[json!("x"), json!(i64::MAX)]).is_err());
        assert_eq!(ok("PAD_LEFT", &[json!("abc"), json!(-5)]), json!("abc"));
    }

    #[test]
    fn find_uses_character_offsets() {
        assert_eq!(ok("FIND_FIRST", &[json!("hello"), json!("l")]), json!(2));
        assert_eq!(ok("FIND_FIRST", &[json!("héllo"), json!("l")]), json!(2));
        assert_eq!(ok("FIND_LAST", &[json!("héllo"), json!("l")]), json!(3));
        assert_eq!(
            ok("FIND_FIRST", &[json!("foobarbaz"), json!("ba"), json!(4)]),
            json!(6)
        );
        assert_eq!(
            ok(
                "FIND_FIRST",
                &[json!("foobarbaz"), json!("ba"), json!(0), json!(3)]
            ),
            json!(-1)
        );
        assert_eq!(
            ok(
                "FIND_LAST",
                &[json!("foobarbaz"), json!("ba"), json!(0), json!(5)]
            ),
            json!(3)
        );
        assert_eq!(ok("FIND_FIRST", &[json!("abc"), json!("z")]), json!(-1));
        assert_eq!(
            ok("FIND_FIRST", &[json!("abc"), json!("a"), json!(i64::MIN)]),
            json!(0)
        );
    }

    #[test]
    fn to_string_follows_aql() {
        assert_eq!(ok("TO_STRING", &[Value::Null]), json!(""));
        assert_eq!(ok("TO_STRING", &[json!(2.0)]), json!("2"));
        assert_eq!(ok("TO_STRING", &[json!(true)]), json!("true"));
    }

    #[test]
    fn length_follows_aql() {
        assert_eq!(ok("LENGTH", &[Value::Null]), json!(0));
        assert_eq!(ok("LENGTH", &[json!(true)]), json!(1));
        assert_eq!(ok("LENGTH", &[json!(123)]), json!(3));
        assert_eq!(ok("LENGTH", &[json!("héllo")]), json!(5));
        assert_eq!(ok("LENGTH", &[json!({"a": 1})]), json!(1));
    }

    #[test]
    fn phrase_without_args_does_not_panic() {
        assert_eq!(ok("PHRASE", &[]), json!(false));
    }

    #[test]
    fn left_right_and_misc() {
        assert_eq!(ok("LEFT", &[json!("héllo"), json!(2)]), json!("hé"));
        assert_eq!(ok("RIGHT", &[json!("héllo"), json!(2)]), json!("lo"));
        assert_eq!(ok("RIGHT", &[json!("abc"), json!(-1)]), json!(""));
        assert_eq!(ok("CAPITALIZE", &[json!("hello")]), json!("Hello"));
        assert_eq!(
            ok("TITLE_CASE", &[json!("hello world")]),
            json!("Hello World")
        );
        assert_eq!(
            ok("LIKE", &[json!("Hello"), json!("h%"), json!(true)]),
            json!(true)
        );
        assert_eq!(ok("LEVENSHTEIN", &[json!("foo"), json!("bar")]), json!(3));
        assert_eq!(
            ok("STARTS_WITH", &[json!("hello"), json!(["x", "he"])]),
            json!(true)
        );
        assert_eq!(ok("REVERSE", &[json!("abc")]), json!("cba"));
    }
}
