//! AQL-compatible string functions for SDBQL.
//!
//! Offsets and `LENGTH` are Unicode scalar counts (not bytes). Null
//! arguments propagate as JSON null. Regexes go through `safe_regex` and a
//! sharded, bounded LRU compile cache ([`cached_regex_with`]) that the `LIKE`
//! and `=~` operators share.

use crate::error::{DbError, DbResult};
use crate::sdbql::executor::utils::safe_regex;
use lru::LruCache;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::num::NonZeroUsize;
use std::sync::Arc;

const MAX_REPEAT_BYTES: usize = 1_048_576;
const MAX_TOKEN_LEN: usize = 4096;
/// Inputs longer than this skip the O(n·m) n-gram comparison (same ceiling
/// as `LEVENSHTEIN`).
const MAX_NGRAM_INPUT_CHARS: usize = 4096;

const REGEX_CACHE_SHARDS: usize = 16;
const REGEX_CACHE_SHARD_CAP: usize = 64;

/// What a cached pattern string means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RegexKind {
    /// A regular expression (`REGEX_*` functions, the `=~` operator).
    Regex,
    /// A SQL `LIKE` pattern, translated with [`like_to_regex`] on a miss.
    Like,
}

/// One LRU per (kind, case-insensitive) pair, so a lookup is keyed by the
/// caller's own `&str`: no key allocation and no LIKE translation on a hit.
struct RegexShard {
    maps: [LruCache<String, Arc<Regex>>; 4],
}

// Sharded so concurrent queries do not serialise on one mutex, and LRU so a
// burst of one-off patterns evicts cold entries instead of clearing the hot
// ones. Entries are `Arc<Regex>`: a `Regex::clone` starts with an empty
// matcher cache pool, which is exactly the work caching is meant to avoid.
static REGEX_CACHE: Lazy<Vec<Mutex<RegexShard>>> = Lazy::new(|| {
    let cap = NonZeroUsize::new(REGEX_CACHE_SHARD_CAP).expect("non-zero shard capacity");
    (0..REGEX_CACHE_SHARDS)
        .map(|_| {
            Mutex::new(RegexShard {
                maps: std::array::from_fn(|_| LruCache::new(cap)),
            })
        })
        .collect()
});

/// FNV-1a, 64 bit. Used for cache sharding and by `FNV64()`.
pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A case-sensitive regular expression from the process-wide cache
/// (shorthand for [`cached_regex_with`] with [`RegexKind::Regex`]).
pub(crate) fn cached_regex_arc(pattern: &str) -> DbResult<Arc<Regex>> {
    cached_regex_with(pattern, false, RegexKind::Regex)
}

/// Compile `pattern` (or fetch it from the process-wide cache).
///
/// `kind` says how to read `pattern`: as a regex, or as a `LIKE` pattern
/// that is translated with [`like_to_regex`] only on a miss (so callers
/// should pass the raw LIKE pattern, not its translation).
/// `case_insensitive` adds `(?i)`.
pub(crate) fn cached_regex_with(
    pattern: &str,
    case_insensitive: bool,
    kind: RegexKind,
) -> DbResult<Arc<Regex>> {
    let slot = usize::from(kind == RegexKind::Like) * 2 + usize::from(case_insensitive);
    let shard = &REGEX_CACHE[(fnv1a64(pattern.as_bytes()) as usize) % REGEX_CACHE_SHARDS];
    if let Some(re) = shard.lock().maps[slot].get(pattern) {
        return Ok(Arc::clone(re));
    }
    let source = match kind {
        RegexKind::Like => like_to_regex(pattern, case_insensitive),
        RegexKind::Regex if case_insensitive => format!("(?i){}", pattern),
        RegexKind::Regex => pattern.to_string(),
    };
    // Compile outside the lock so a slow pattern does not stall its shard.
    let re = Arc::new(safe_regex(&source)?);
    shard.lock().maps[slot].put(pattern.to_string(), Arc::clone(&re));
    Ok(re)
}

/// Translate a SQL `LIKE` pattern into an anchored regex.
///
/// `%` is any run of characters and `_` any one character, newlines
/// included (`(?s)`). `\%`, `\_` and `\\` are the literal characters; a
/// backslash before anything else is a literal backslash.
pub(crate) fn like_to_regex(pattern: &str, case_insensitive: bool) -> String {
    fn push_literal(out: &mut String, c: char) {
        if c.is_alphanumeric() || c == ' ' {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            out.push_str(&regex::escape(c.encode_utf8(&mut buf)));
        }
    }
    let mut out = String::with_capacity(pattern.len() + 12);
    out.push_str(if case_insensitive { "(?si)^" } else { "(?s)^" });
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '%' => out.push_str(".*"),
            '_' => out.push('.'),
            '\\' => match chars.peek() {
                Some(&next) if matches!(next, '%' | '_' | '\\') => {
                    chars.next();
                    push_literal(&mut out, next);
                }
                _ => push_literal(&mut out, '\\'),
            },
            _ => push_literal(&mut out, c),
        }
    }
    out.push('$');
    out
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

/// Append `n` as AQL prints it: an integral finite float prints without a
/// fractional part (`2.0` → `"2"`), everything else as serde_json does.
fn write_number(out: &mut String, n: &serde_json::Number) {
    use std::fmt::Write;
    if n.is_f64() {
        if let Some(f) = n.as_f64() {
            // Below 2^53 every integral f64 is exactly representable as i64.
            if f.is_finite() && f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 {
                let _ = write!(out, "{}", f as i64);
                return;
            }
        }
    }
    let _ = write!(out, "{}", n);
}

/// Append the string form of `v`: strings verbatim, null as nothing,
/// numbers per [`write_number`], arrays and objects as JSON.
pub(crate) fn write_value(out: &mut String, v: &Value) {
    match v {
        Value::String(s) => out.push_str(s),
        Value::Number(n) => write_number(out, n),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Null => {}
        other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
    }
}

/// String form of `v` (see [`write_value`]).
pub(crate) fn stringify(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => {
            let mut out = String::new();
            write_value(&mut out, other);
            out
        }
    }
}

/// Like [`stringify`], but borrows when `v` already is a string.
pub(crate) fn text_of(v: &Value) -> Cow<'_, str> {
    match v {
        Value::String(s) => Cow::Borrowed(s.as_str()),
        other => Cow::Owned(stringify(other)),
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

/// `FIND_FIRST` / `FIND_LAST` / `CONTAINS(..., true)`: character index of the
/// first (or last) occurrence of `needle` lying wholly inside the character
/// window `[start, end]` (both inclusive, AQL), or -1. Negative bounds count
/// from the end of the string (a SoliDB extension).
fn find_in(hay: &str, needle: &str, start: Option<i64>, end: Option<i64>, last: bool) -> i64 {
    let n = hay.chars().count() as i64;
    let resolve = |v: i64| if v < 0 { (n + v).max(0) } else { v };
    let s = start.map(resolve).unwrap_or(0);
    let e = end.map(resolve).unwrap_or(n - 1).min(n - 1);
    if s > n {
        return -1;
    }
    if needle.is_empty() {
        return if last { (e + 1).clamp(s, n) } else { s };
    }
    if e < s {
        return -1;
    }
    let b0 = nth_char_byte(hay, s as usize);
    let b1 = nth_char_byte(hay, (e + 1) as usize);
    let window = &hay[b0..b1];
    let found = if last {
        window.rfind(needle)
    } else {
        window.find(needle)
    };
    match found {
        Some(rel) => char_index_at_byte(hay, b0 + rel) as i64,
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

/// `SUBSTRING_BYTES`: byte offsets; `None` when the range would split a
/// UTF-8 sequence (AQL returns null there).
fn substring_bytes(s: &str, offset: i64, length: Option<i64>) -> Option<String> {
    let n = s.len() as i64;
    let start = if offset < 0 { n + offset } else { offset }.clamp(0, n);
    let end = match length {
        Some(len) if len < 0 => start,
        Some(len) => start.saturating_add(len).min(n),
        None => n,
    };
    s.get(start as usize..end as usize).map(str::to_string)
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
    // Every padding char is at least one byte, so this rejects before the
    // `reserve` below rather than after it has allocated `target` bytes.
    if need.saturating_add(s.len()) > MAX_REPEAT_BYTES {
        return Err(DbError::ExecutionError(
            "PAD: result would exceed 1 MiB".to_string(),
        ));
    }
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

/// Replacement functions may grow their input by at most this much. An
/// empty-matching pattern inserts the replacement between every character,
/// so without a ceiling 1 MB × 1 MB is a terabyte.
fn output_cap(input_len: usize) -> usize {
    input_len.saturating_add(MAX_REPEAT_BYTES)
}

fn too_large(name: &str) -> DbError {
    DbError::ExecutionError(format!(
        "{}: result would grow the input by more than {} bytes",
        name, MAX_REPEAT_BYTES
    ))
}

/// Leftmost-longest, non-overlapping search for several needles in one pass.
///
/// Each needle remembers its next occurrence and is searched again only once
/// the cursor has moved past it, so the text is scanned once per needle
/// rather than once per match. Needles must be non-empty.
struct MultiFind<'a> {
    hay: &'a str,
    needles: &'a [&'a str],
    /// Byte offset of each needle's next occurrence; `usize::MAX` = none left.
    next: Vec<usize>,
}

impl<'a> MultiFind<'a> {
    fn new(hay: &'a str, needles: &'a [&'a str]) -> Self {
        let next = needles
            .iter()
            .map(|n| hay.find(n).unwrap_or(usize::MAX))
            .collect();
        Self { hay, needles, next }
    }

    /// Next match at or after byte `from` (a char boundary), as
    /// `(byte offset, needle index)`.
    fn next_from(&mut self, from: usize) -> Option<(usize, usize)> {
        let hay = self.hay;
        let needles = self.needles;
        let mut best: Option<(usize, usize)> = None;
        for (i, needle) in needles.iter().enumerate() {
            let mut pos = self.next[i];
            if pos != usize::MAX && pos < from {
                pos = hay[from..].find(needle).map_or(usize::MAX, |r| from + r);
                self.next[i] = pos;
            }
            if pos == usize::MAX {
                continue;
            }
            best = match best {
                Some((bp, bi)) if bp < pos || (bp == pos && needles[bi].len() >= needle.len()) => {
                    Some((bp, bi))
                }
                _ => Some((pos, i)),
            };
        }
        best
    }
}

/// Single-pass replacement of `needles[i]` by `replacements[i]`, at most
/// `limit` replacements in total. Replaced text is never searched again.
fn replace_multi(
    text: &str,
    needles: &[&str],
    replacements: &[&str],
    limit: Option<usize>,
    name: &str,
) -> DbResult<String> {
    let cap = output_cap(text.len());
    let mut finder = MultiFind::new(text, needles);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut count = 0usize;
    while limit.is_none_or(|l| count < l) {
        let Some((pos, i)) = finder.next_from(last) else {
            break;
        };
        out.push_str(&text[last..pos]);
        out.push_str(replacements[i]);
        last = pos + needles[i].len();
        count += 1;
        if out.len() > cap {
            return Err(too_large(name));
        }
    }
    out.push_str(&text[last..]);
    if out.len() > cap {
        return Err(too_large(name));
    }
    Ok(out)
}

/// `REGEX_REPLACE` with the same growth ceiling as [`replace_multi`].
fn regex_replace_capped(re: &Regex, text: &str, replacement: &str) -> DbResult<String> {
    let cap = output_cap(text.len());
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    if replacement.contains('$') {
        for caps in re.captures_iter(text) {
            let m = caps.get(0).expect("group 0 always participates in a match");
            out.push_str(&text[last..m.start()]);
            caps.expand(replacement, &mut out);
            last = m.end();
            if out.len() > cap {
                return Err(too_large("REGEX_REPLACE"));
            }
        }
    } else {
        for m in re.find_iter(text) {
            out.push_str(&text[last..m.start()]);
            out.push_str(replacement);
            last = m.end();
            if out.len() > cap {
                return Err(too_large("REGEX_REPLACE"));
            }
        }
    }
    out.push_str(&text[last..]);
    if out.len() > cap {
        return Err(too_large("REGEX_REPLACE"));
    }
    Ok(out)
}

fn trim_with(value: &str, chars: Option<&str>, left: bool, right: bool) -> String {
    let out = match chars {
        Some(set) => {
            let pred = |ch: char| set.contains(ch);
            match (left, right) {
                (true, true) => value.trim_matches(pred),
                (true, false) => value.trim_start_matches(pred),
                _ => value.trim_end_matches(pred),
            }
        }
        None => match (left, right) {
            (true, true) => value.trim(),
            (true, false) => value.trim_start(),
            _ => value.trim_end(),
        },
    };
    out.to_string()
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

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Percent-decode on bytes (never slicing the `str`, which panicked when a
/// multibyte character followed `%`). `+` becomes a space only for
/// `URL_DECODE` (form encoding); `decodeURIComponent` keeps it.
fn decode_uri(s: &str, plus_as_space: bool) -> String {
    let b = s.as_bytes();
    let mut bytes = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match (hex_digit(b[i + 1]), hex_digit(b[i + 2])) {
                (Some(hi), Some(lo)) => {
                    bytes.push((hi << 4) | lo);
                    i += 3;
                }
                _ => {
                    bytes.push(b'%');
                    i += 1;
                }
            },
            b'+' if plus_as_space => {
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

/// Strict dotted-quad IPv4 (`a.b.c.d`, each 0–255, no leading zeros).
pub(crate) fn parse_ipv4(s: &str) -> Option<u32> {
    let mut parts = s.split('.');
    let mut out: u32 = 0;
    for _ in 0..4 {
        let p = parts.next()?;
        if p.is_empty()
            || p.len() > 3
            || !p.bytes().all(|b| b.is_ascii_digit())
            || (p.len() > 1 && p.starts_with('0'))
        {
            return None;
        }
        let v: u32 = p.parse().ok()?;
        if v > 255 {
            return None;
        }
        out = (out << 8) | v;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

fn ipv4_from_number(v: &Value) -> Option<String> {
    let n = match v {
        Value::Number(n) => n.as_u64().or_else(|| {
            n.as_f64()
                .filter(|f| f.fract() == 0.0 && *f >= 0.0 && *f <= f64::from(u32::MAX))
                .map(|f| f as u64)
        })?,
        _ => return None,
    };
    let n = u32::try_from(n).ok()?;
    let [a, b, c, d] = n.to_be_bytes();
    Some(format!("{}.{}.{}.{}", a, b, c, d))
}

/// AQL `NGRAM_SIMILARITY` / `NGRAM_POSITIONAL_SIMILARITY`.
///
/// Longest common subsequence over the two strings' n-grams. The plain form
/// counts whole n-gram matches and divides by the number of `target`
/// n-grams; the positional form credits partially matching n-grams by the
/// share of equal positions and divides by the larger n-gram count. A string
/// shorter than `n` is a single n-gram.
pub(crate) fn ngram_similarity(input: &str, target: &str, n: usize, positional: bool) -> f64 {
    fn grams(s: &[char], n: usize) -> Vec<&[char]> {
        if s.is_empty() {
            Vec::new()
        } else if s.len() < n {
            vec![s]
        } else {
            s.windows(n).collect()
        }
    }
    let a: Vec<char> = input.chars().collect();
    let b: Vec<char> = target.chars().collect();
    if a.len() > MAX_NGRAM_INPUT_CHARS || b.len() > MAX_NGRAM_INPUT_CHARS {
        return if a == b { 1.0 } else { 0.0 };
    }
    let ga = grams(&a, n);
    let gb = grams(&b, n);
    if ga.is_empty() || gb.is_empty() {
        return if ga.is_empty() && gb.is_empty() {
            1.0
        } else {
            0.0
        };
    }
    let mut prev = vec![0.0f64; gb.len() + 1];
    let mut cur = vec![0.0f64; gb.len() + 1];
    for x in &ga {
        cur[0] = 0.0;
        for (j, y) in gb.iter().enumerate() {
            let score = if positional {
                let same = x.iter().zip(y.iter()).filter(|(p, q)| p == q).count();
                same as f64 / x.len().max(y.len()) as f64
            } else if x == y {
                1.0
            } else {
                0.0
            };
            cur[j + 1] = prev[j + 1].max(cur[j]).max(prev[j] + score);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let denom = if positional {
        ga.len().max(gb.len())
    } else {
        gb.len()
    };
    prev[gb.len()] / denom as f64
}

/// Append `v` to a separator-joined string, skipping nulls (AQL
/// `CONCAT_SEPARATOR` ignores them rather than emitting empty fields).
fn push_joined(out: &mut String, first: &mut bool, sep: &str, v: &Value) {
    if v.is_null() {
        return;
    }
    if !*first {
        out.push_str(sep);
    }
    *first = false;
    write_value(out, v);
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
                        1 => trim_with(value, None, true, false),
                        2 => trim_with(value, None, false, true),
                        _ => trim_with(value, None, true, true),
                    }
                } else if let Some(chars) = args[1].as_str() {
                    trim_with(value, Some(chars), true, true)
                } else {
                    trim_with(value, None, true, true)
                }
            } else {
                trim_with(value, None, true, true)
            };
            Ok(Some(Value::String(result)))
        }
        "LTRIM" | "RTRIM" => {
            if args.is_empty() || args.len() > 2 {
                return Err(err_arity(name, "1-2: string, [chars]"));
            }
            if null_if_any_null(args) {
                return Ok(Some(Value::Null));
            }
            let value = require_str(name, args, 0)?;
            let chars = args.get(1).map(text_of);
            let left = name == "LTRIM";
            Ok(Some(Value::String(trim_with(
                value,
                chars.as_deref(),
                left,
                !left,
            ))))
        }
        "JOIN" => {
            if args.is_empty() {
                return Ok(Some(Value::String(String::new())));
            }
            let sep = args.get(1).and_then(Value::as_str).unwrap_or(",");
            match &args[0] {
                Value::Null => Ok(Some(Value::Null)),
                Value::Array(arr) => {
                    let mut out = String::new();
                    for (i, item) in arr.iter().enumerate() {
                        if i > 0 {
                            out.push_str(sep);
                        }
                        write_value(&mut out, item);
                    }
                    Ok(Some(Value::String(out)))
                }
                _ => Err(DbError::ExecutionError(
                    "JOIN requires an array as first argument".to_string(),
                )),
            }
        }
        "CONCAT" => {
            // One buffer; arrays contribute their members, nulls nothing.
            let mut out = String::new();
            for v in args {
                match v {
                    Value::Array(arr) => {
                        for item in arr {
                            write_value(&mut out, item);
                        }
                    }
                    other => write_value(&mut out, other),
                }
            }
            Ok(Some(Value::String(out)))
        }
        "CONCAT_WS" | "CONCAT_SEPARATOR" => {
            if args.is_empty() {
                return Ok(Some(Value::String(String::new())));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let sep = text_of(&args[0]);
            let mut out = String::new();
            let mut first = true;
            for v in &args[1..] {
                match v {
                    Value::Array(arr) => {
                        for item in arr {
                            push_joined(&mut out, &mut first, &sep, item);
                        }
                    }
                    other => push_joined(&mut out, &mut first, &sep, other),
                }
            }
            Ok(Some(Value::String(out)))
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
                Ok(Some(Value::Number(serde_json::Number::from(find_in(
                    haystack, needle, None, None, false,
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
        "SPLIT" => split(args),
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
        "SUBSTRING_BYTES" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, offset, [length]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = require_str(name, args, 0)?;
            let offset = as_i64(&args[1], 0);
            let length = args.get(2).filter(|v| !v.is_null()).map(|v| as_i64(v, 0));
            Ok(Some(
                substring_bytes(s, offset, length).map_or(Value::Null, Value::String),
            ))
        }
        "REPLACE" => {
            if args.len() != 3 {
                return Err(err_arity(name, "3"));
            }
            if null_if_any_null(args) {
                return Ok(Some(Value::Null));
            }
            let text = require_str(name, args, 0)?;
            let search = require_str(name, args, 1)?;
            // An empty search would insert the replacement between every
            // character; AQL leaves the text alone.
            if search.is_empty() {
                return Ok(Some(Value::String(text.to_string())));
            }
            let replacement = require_str(name, args, 2)?;
            Ok(Some(Value::String(replace_multi(
                text,
                &[search],
                &[replacement],
                None,
                name,
            )?)))
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
        "FIND_FIRST" | "FIND" | "FIND_LAST" | "RFIND" => {
            if args.len() < 2 || args.len() > 4 {
                return Err(err_arity(name, "2-4: string, search, [start], [end]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let bound = |i: usize| args.get(i).filter(|v| !v.is_null()).map(|v| as_i64(v, 0));
            let last = matches!(name, "FIND_LAST" | "RFIND");
            Ok(Some(Value::Number(serde_json::Number::from(find_in(
                require_str(name, args, 0)?,
                require_str(name, args, 1)?,
                bound(2),
                bound(3),
                last,
            )))))
        }
        "LIKE" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: text, pattern, [caseInsensitive]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let text = text_of(&args[0]);
            let pattern = require_str(name, args, 1)?;
            let case_insensitive = args.get(2).and_then(Value::as_bool).unwrap_or(false);
            let re = cached_regex_with(pattern, case_insensitive, RegexKind::Like)?;
            Ok(Some(Value::Bool(re.is_match(&text))))
        }
        "REGEX_TEST" | "REGEX_MATCH" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, pattern, [caseInsensitive]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let ci = args.get(2).and_then(Value::as_bool).unwrap_or(false);
            let re = cached_regex_with(require_str(name, args, 1)?, ci, RegexKind::Regex)?;
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
            let text = require_str(name, args, 0)?;
            let pattern = require_str(name, args, 1)?;
            if pattern.is_empty() {
                return Ok(Some(Value::String(text.to_string())));
            }
            let ci = args.get(3).and_then(Value::as_bool).unwrap_or(false);
            let re = cached_regex_with(pattern, ci, RegexKind::Regex)?;
            Ok(Some(Value::String(regex_replace_capped(
                &re,
                text,
                require_str(name, args, 2)?,
            )?)))
        }
        // SoliDB semantics, documented in docs/SDBQL_REFERENCE.md: every
        // non-overlapping match, not AQL's [match, groups...] of the first.
        "REGEX_MATCHES" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, pattern, [caseInsensitive]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let ci = args.get(2).and_then(Value::as_bool).unwrap_or(false);
            let re = cached_regex_with(require_str(name, args, 1)?, ci, RegexKind::Regex)?;
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
            let re = cached_regex_with(require_str(name, args, 1)?, false, RegexKind::Regex)?;
            let text = require_str(name, args, 0)?;
            let parts: Vec<Value> = match args.get(2).map(|v| as_i64(v, 0)) {
                Some(limit) if limit > 0 => re
                    .splitn(text, limit as usize)
                    .map(|s| Value::String(s.to_string()))
                    .collect(),
                _ => re
                    .split(text)
                    .map(|s| Value::String(s.to_string()))
                    .collect(),
            };
            Ok(Some(Value::Array(parts)))
        }
        "REPEAT" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, count, [separator]"));
            }
            if args[0].is_null() || args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = require_str(name, args, 0)?;
            let n = as_i64(&args[1], 0).max(0) as usize;
            let sep_arg = args.get(2).filter(|v| !v.is_null()).map(text_of);
            let sep = sep_arg.as_deref().unwrap_or("");
            let total = s
                .len()
                .saturating_mul(n)
                .saturating_add(sep.len().saturating_mul(n.saturating_sub(1)));
            if total > MAX_REPEAT_BYTES {
                return Err(DbError::ExecutionError(format!(
                    "REPEAT: result would be {} bytes (max {})",
                    total, MAX_REPEAT_BYTES
                )));
            }
            if sep.is_empty() {
                return Ok(Some(Value::String(s.repeat(n))));
            }
            let mut out = String::with_capacity(total);
            for i in 0..n {
                if i > 0 {
                    out.push_str(sep);
                }
                out.push_str(s);
            }
            Ok(Some(Value::String(out)))
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
                    // Whitespace is copied as-is; only letter case changes.
                    let mut out = String::with_capacity(s.len());
                    let mut word_start = true;
                    for c in s.chars() {
                        if c.is_whitespace() {
                            out.push(c);
                            word_start = true;
                        } else if word_start {
                            out.extend(c.to_uppercase());
                            word_start = false;
                        } else {
                            out.extend(c.to_lowercase());
                        }
                    }
                    Ok(Some(Value::String(out)))
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
        "TRUNCATE_TEXT" | "ELLIPSIS" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(err_arity(name, "2-3: string, length, [suffix]"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = require_str(name, args, 0)?;
            let len = as_i64(&args[1], 0).max(0) as usize;
            let suffix_arg = args.get(2).filter(|v| !v.is_null()).map(text_of);
            let suffix = suffix_arg.as_deref().unwrap_or("...");
            if s.chars().count() <= len {
                return Ok(Some(Value::String(s.to_string())));
            }
            // The suffix counts towards `len`: the result is at most `len`
            // characters long.
            let suffix_len = suffix.chars().count();
            if suffix_len >= len {
                return Ok(Some(Value::String(suffix.chars().take(len).collect())));
            }
            let cut = &s[..nth_char_byte(s, len - suffix_len)];
            Ok(Some(Value::String(format!("{}{}", cut.trim_end(), suffix))))
        }
        "MASK" => {
            if args.is_empty() || args.len() > 4 {
                return Err(err_arity(name, "1-4: string, [start], [end], [char]"));
            }
            if args[0].is_null() {
                return Ok(Some(Value::Null));
            }
            let s = require_str(name, args, 0)?;
            let chars: Vec<char> = s.chars().collect();
            let n = chars.len() as i64;
            let resolve = |v: &Value| {
                let i = as_i64(v, 0);
                if i < 0 {
                    (n + i).max(0)
                } else {
                    i
                }
            };
            let arg = |i: usize| args.get(i).filter(|v| !v.is_null());
            let start = arg(1).map(resolve).unwrap_or(0) as usize;
            // Default end keeps the last character visible (unchanged behaviour).
            let end = arg(2).map(resolve).unwrap_or((n - 1).max(0)) as usize;
            let mask_char = arg(3)
                .and_then(|v| text_of(v).chars().next())
                .unwrap_or('*');
            let out: String = chars
                .iter()
                .enumerate()
                .map(|(i, &c)| if i >= start && i < end { mask_char } else { c })
                .collect();
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
                Value::String(s) => Ok(Some(Value::String(decode_uri(s, name == "URL_DECODE")))),
                _ => Err(DbError::ExecutionError(
                    "DECODE_URI requires a string argument".to_string(),
                )),
            }
        }
        "IPV4_TO_NUMBER" => {
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            Ok(Some(
                args[0]
                    .as_str()
                    .and_then(parse_ipv4)
                    .map_or(Value::Null, |n| json!(n)),
            ))
        }
        "IPV4_FROM_NUMBER" => {
            if args.len() != 1 {
                return Err(err_arity(name, "1"));
            }
            Ok(Some(
                ipv4_from_number(&args[0]).map_or(Value::Null, Value::String),
            ))
        }
        "NGRAM_SIMILARITY" | "NGRAM_POSITIONAL_SIMILARITY" => {
            if args.len() != 3 {
                return Err(err_arity(name, "3: input, target, ngramSize"));
            }
            if null_if_any_null(args) {
                return Ok(Some(Value::Null));
            }
            let n = as_i64(&args[2], 0);
            if n < 1 {
                return Err(DbError::ExecutionError(format!(
                    "{}: ngramSize must be a positive integer",
                    name
                )));
            }
            Ok(Some(json!(ngram_similarity(
                require_str(name, args, 0)?,
                require_str(name, args, 1)?,
                n as usize,
                name == "NGRAM_POSITIONAL_SIMILARITY",
            ))))
        }
        // Hash functions live in crypto.rs; routed here until the prefix
        // dispatcher in builtins/mod.rs lists them.
        "SHA1" | "CRC32" | "FNV64" => super::crypto::evaluate(name, args),
        "NUMBER_FORMAT" => number_format(args).map(Some),
        _ => Ok(None),
    }
}

/// Most decimals `NUMBER_FORMAT` will print; an f64 has no more to give.
const MAX_FORMAT_DECIMALS: i64 = 20;

/// Decimal and thousands separators for the locales `NUMBER_FORMAT` knows,
/// from CLDR. French groups with a narrow no-break space (U+202F), as
/// `Intl.NumberFormat("fr")` does; pass `{thousands: " "}` for a plain one.
fn locale_separators(tag: &str) -> Option<(&'static str, &'static str)> {
    let tag = tag.to_ascii_lowercase().replace('_', "-");
    let seps = |t: &str| match t {
        "en" => Some((".", ",")),
        "fr" => Some((",", "\u{202F}")),
        "de" | "es" | "it" | "nl" | "pt" => Some((",", ".")),
        "de-ch" => Some((".", "\u{2019}")),
        _ => None,
    };
    seps(&tag).or_else(|| seps(tag.split('-').next().unwrap_or("")))
}

/// `NUMBER_FORMAT(number, [decimals], [locale | {decimal, thousands}])`:
/// a number as display text, `1234567.891` → `"1,234,567.89"` with 2
/// decimals. Rounds half away from zero on the number's shortest decimal
/// form, so `2.675` gives `"2.68"` as a person expects rather than the
/// `"2.67"` its binary value would.
fn number_format(args: &[Value]) -> DbResult<Value> {
    const NAME: &str = "NUMBER_FORMAT";
    if args.is_empty() || args.len() > 3 {
        return Err(err_arity(NAME, "1-3: number, [decimals], [locale]"));
    }
    let n = match &args[0] {
        Value::Null => return Ok(Value::Null),
        Value::Number(n) => n,
        _ => {
            return Err(DbError::ExecutionError(format!(
                "{NAME}: first argument must be a number"
            )))
        }
    };
    let decimals = match args.get(1) {
        None | Some(Value::Null) => 0,
        Some(v) => super::array::as_int(v)
            .filter(|d| (0..=MAX_FORMAT_DECIMALS).contains(d))
            .ok_or_else(|| {
                DbError::ExecutionError(format!(
                    "{NAME}: decimals must be an integer from 0 to {MAX_FORMAT_DECIMALS}"
                ))
            })? as usize,
    };
    let (decimal_sep, thousands_sep): (Cow<'_, str>, Cow<'_, str>) = match args.get(2) {
        None | Some(Value::Null) => (".".into(), ",".into()),
        Some(Value::String(tag)) => {
            let (d, t) = locale_separators(tag).ok_or_else(|| {
                DbError::ExecutionError(format!(
                    "{NAME}: unknown locale '{tag}' (known: en, fr, de, es, it, nl, pt, de-CH; \
                     or pass {{decimal, thousands}})"
                ))
            })?;
            (d.into(), t.into())
        }
        Some(Value::Object(o)) => {
            let sep = |key: &str, default: &'static str| -> DbResult<Cow<'_, str>> {
                match o.get(key) {
                    None | Some(Value::Null) => Ok(default.into()),
                    Some(Value::String(s)) => Ok(s.as_str().into()),
                    Some(_) => Err(DbError::ExecutionError(format!(
                        "{NAME}: '{key}' must be a string"
                    ))),
                }
            };
            (sep("decimal", ".")?, sep("thousands", ",")?)
        }
        Some(_) => {
            return Err(DbError::ExecutionError(format!(
                "{NAME}: third argument must be a locale string or an object"
            )))
        }
    };

    // Shortest decimal form: integers exactly, floats as Rust's Display
    // prints them (round-trip digits, never an exponent).
    let text = match (n.as_i64(), n.as_u64(), n.as_f64()) {
        (Some(i), _, _) => i.to_string(),
        (_, Some(u), _) => u.to_string(),
        (_, _, Some(f)) => f.to_string(),
        _ => n.to_string(),
    };
    let negative = text.starts_with('-');
    let text = text.trim_start_matches('-');
    let (int_part, frac_part) = text.split_once('.').unwrap_or((text, ""));

    // Digits of int_part followed by exactly `decimals` fraction digits,
    // rounded half away from zero on the first dropped digit.
    let mut digits: Vec<u8> = int_part.bytes().map(|b| b - b'0').collect();
    let frac: Vec<u8> = frac_part.bytes().map(|b| b - b'0').collect();
    digits.extend((0..decimals).map(|i| frac.get(i).copied().unwrap_or(0)));
    if frac.get(decimals).is_some_and(|&d| d >= 5) {
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, 1);
                break;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    let split = digits.len() - decimals;
    let (int_digits, frac_digits) = digits.split_at(split);

    let mut out = String::with_capacity(digits.len() * 2 + 2);
    // `-0.001` to two decimals is "0.00", not "-0.00".
    if negative && digits.iter().any(|&d| d != 0) {
        out.push('-');
    }
    for (i, d) in int_digits.iter().enumerate() {
        if i > 0 && (int_digits.len() - i) % 3 == 0 {
            out.push_str(&thousands_sep);
        }
        out.push((b'0' + d) as char);
    }
    if decimals > 0 {
        out.push_str(&decimal_sep);
        out.extend(frac_digits.iter().map(|d| (b'0' + d) as char));
    }
    Ok(Value::String(out))
}

fn err_arity(name: &str, expected: &str) -> DbError {
    DbError::ExecutionError(format!("{} requires {} argument(s)", name, expected))
}

/// AQL `SPLIT(value, separator, limit?)`. `separator` may be an array (split
/// at any of them, leftmost-longest); an empty separator splits into
/// characters. `limit` keeps at most that many parts and drops the rest.
fn split(args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() < 2 || args.len() > 3 {
        return Err(err_arity("SPLIT", "2-3: string, separator, [limit]"));
    }
    if args[0].is_null() || args[1].is_null() {
        return Ok(Some(Value::Null));
    }
    let value_cow = text_of(&args[0]);
    let value: &str = &value_cow;
    let seps: Vec<String> = match &args[1] {
        Value::Array(a) => a.iter().map(stringify).filter(|s| !s.is_empty()).collect(),
        other => {
            let s = stringify(other);
            if s.is_empty() {
                Vec::new()
            } else {
                vec![s]
            }
        }
    };
    let take = match args.get(2).filter(|v| !v.is_null()).map(|v| as_i64(v, 0)) {
        Some(0) => return Ok(Some(Value::Array(Vec::new()))),
        Some(n) if n > 0 => n as usize,
        _ => usize::MAX,
    };
    let parts: Vec<Value> = if seps.is_empty() {
        value
            .chars()
            .take(take)
            .map(|c| Value::String(c.to_string()))
            .collect()
    } else if seps.len() == 1 {
        value
            .split(seps[0].as_str())
            .take(take)
            .map(|s| Value::String(s.to_string()))
            .collect()
    } else {
        let needles: Vec<&str> = seps.iter().map(String::as_str).collect();
        let mut finder = MultiFind::new(value, &needles);
        let mut out = Vec::new();
        let mut last = 0;
        while out.len() < take {
            match finder.next_from(last) {
                Some((pos, i)) => {
                    out.push(Value::String(value[last..pos].to_string()));
                    last = pos + needles[i].len();
                }
                None => {
                    out.push(Value::String(value[last..].to_string()));
                    break;
                }
            }
        }
        out
    };
    Ok(Some(Value::Array(parts)))
}

/// AQL `SUBSTITUTE`:
/// - `SUBSTITUTE(value, search, replace?, limit?)` — `search` a string or an
///   array; `replace` a string (used for every search), an array (pairwise,
///   missing entries remove the match) or omitted/null (remove matches);
/// - `SUBSTITUTE(value, {search: replace, ...}, limit?)`.
///
/// One left-to-right pass, leftmost-longest match first, so replaced text is
/// never matched again; `limit` caps the total number of replacements.
fn substitute(args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() < 2 || args.len() > 4 {
        return Err(err_arity("SUBSTITUTE", "2-4"));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let text_cow = text_of(&args[0]);
    let text: &str = &text_cow;
    let (searches, replaces, limit_arg): (Vec<String>, Vec<String>, Option<&Value>) =
        if let Value::Object(map) = &args[1] {
            if args.len() > 3 {
                return Err(err_arity("SUBSTITUTE", "2-3 with a mapping object"));
            }
            (
                map.keys().cloned().collect(),
                map.values().map(stringify).collect(),
                args.get(2),
            )
        } else {
            if args[1].is_null() {
                return Ok(Some(Value::Null));
            }
            let searches: Vec<String> = match &args[1] {
                Value::Array(a) => a.iter().map(stringify).collect(),
                other => vec![stringify(other)],
            };
            let replaces: Vec<String> = match args.get(2) {
                None | Some(Value::Null) => vec![String::new(); searches.len()],
                Some(Value::Array(a)) => (0..searches.len())
                    .map(|i| a.get(i).map(stringify).unwrap_or_default())
                    .collect(),
                Some(other) => vec![stringify(other); searches.len()],
            };
            (searches, replaces, args.get(3))
        };
    let limit = limit_arg
        .filter(|v| !v.is_null())
        .map(|v| as_i64(v, 0).max(0) as usize);

    // Empty search strings match nothing (AQL leaves the text unchanged).
    let (needles, replacements): (Vec<&str>, Vec<&str>) = searches
        .iter()
        .zip(replaces.iter())
        .filter(|(s, _)| !s.is_empty())
        .map(|(s, r)| (s.as_str(), r.as_str()))
        .unzip();
    if needles.is_empty() || limit == Some(0) {
        return Ok(Some(Value::String(text.to_string())));
    }
    Ok(Some(Value::String(replace_multi(
        text,
        &needles,
        &replacements,
        limit,
        "SUBSTITUTE",
    )?)))
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
    fn number_format_rounds_and_groups() {
        let f = |args: &[Value]| call("NUMBER_FORMAT", args);
        assert_eq!(f(&[json!(1234567.891), json!(2)]), json!("1,234,567.89"));
        assert_eq!(f(&[json!(1234567)]), json!("1,234,567"));
        assert_eq!(f(&[json!(999.5)]), json!("1,000"));
        assert_eq!(f(&[json!(2.675), json!(2)]), json!("2.68"));
        assert_eq!(f(&[json!(1.005), json!(2)]), json!("1.01"));
        assert_eq!(f(&[json!(-1234.5), json!(0)]), json!("-1,235"));
        assert_eq!(f(&[json!(-0.001), json!(2)]), json!("0.00"));
        assert_eq!(f(&[json!(0.5), json!(3)]), json!("0.500"));
        assert_eq!(f(&[json!(123)]), json!("123"));
        assert_eq!(f(&[json!(u64::MAX)]), json!("18,446,744,073,709,551,615"));
        assert_eq!(f(&[Value::Null]), Value::Null);
    }

    #[test]
    fn number_format_locales_and_separators() {
        let f = |args: &[Value]| call("NUMBER_FORMAT", args);
        assert_eq!(
            f(&[json!(1234.5), json!(2), json!("fr-FR")]),
            json!("1\u{202F}234,50")
        );
        assert_eq!(
            f(&[json!(1234.5), json!(2), json!("de")]),
            json!("1.234,50")
        );
        assert_eq!(
            f(&[json!(1234.5), json!(2), json!("de_CH")]),
            json!("1\u{2019}234.50")
        );
        assert_eq!(
            f(&[
                json!(1234.5),
                json!(2),
                json!({"decimal": ",", "thousands": " "})
            ]),
            json!("1 234,50")
        );
        assert_eq!(
            f(&[json!(1234.5), json!(1), json!({"thousands": ""})]),
            json!("1234.5")
        );
        assert!(evaluate("NUMBER_FORMAT", &[json!(1), json!(0), json!("xx")]).is_err());
        assert!(evaluate("NUMBER_FORMAT", &[json!(1), json!(21)]).is_err());
        assert!(evaluate("NUMBER_FORMAT", &[json!(1), json!(1.5)]).is_err());
        assert!(evaluate("NUMBER_FORMAT", &[json!("1")]).is_err());
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
    fn find_first_last_start_end_window() {
        // AQL: FIND_FIRST("foobarbaz", "ba", 4) → 6; with end 3 → -1.
        let t = json!("foobarbaz");
        assert_eq!(
            call("FIND_FIRST", &[t.clone(), json!("ba"), json!(4)]),
            json!(6)
        );
        assert_eq!(
            call("FIND_FIRST", &[t.clone(), json!("ba"), json!(0), json!(3)]),
            json!(-1)
        );
        assert_eq!(
            call("FIND_FIRST", &[t.clone(), json!("ba"), json!(0), json!(4)]),
            json!(3)
        );
        assert_eq!(call("FIND_LAST", &[t.clone(), json!("ba")]), json!(6));
        // Third argument of FIND_LAST is `start`, as in AQL.
        assert_eq!(
            call("FIND_LAST", &[t.clone(), json!("ba"), json!(7)]),
            json!(-1)
        );
        assert_eq!(
            call("FIND_LAST", &[t.clone(), json!("ba"), json!(0), json!(5)]),
            json!(3)
        );
        assert_eq!(call("FIND_LAST", &[json!("héllo"), json!("l")]), json!(3));
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
        assert_eq!(
            call("REGEX_MATCH", &[json!("abc"), json!("^A"), json!(true)]),
            json!(true)
        );
    }

    #[test]
    fn like_multiline_escapes_and_null() {
        assert_eq!(call("LIKE", &[json!("a\nb"), json!("a%")]), json!(true));
        assert_eq!(call("LIKE", &[json!("a\nb"), json!("a_b")]), json!(true));
        assert_eq!(call("LIKE", &[json!("50%"), json!(r"50\%")]), json!(true));
        assert_eq!(call("LIKE", &[json!("500"), json!(r"50\%")]), json!(false));
        assert_eq!(call("LIKE", &[json!("a_b"), json!(r"a\_b")]), json!(true));
        assert_eq!(call("LIKE", &[json!("axb"), json!(r"a\_b")]), json!(false));
        assert_eq!(call("LIKE", &[json!(r"a\b"), json!(r"a\\b")]), json!(true));
        assert_eq!(call("LIKE", &[json!("a.c"), json!("a.c")]), json!(true));
        assert_eq!(call("LIKE", &[json!("abc"), json!("a.c")]), json!(false));
        assert_eq!(call("LIKE", &[Value::Null, json!("%")]), Value::Null);
        assert_eq!(like_to_regex("a%", false), "(?s)^a.*$");
        assert_eq!(like_to_regex("a_", true), "(?si)^a.$");
    }

    #[test]
    fn regex_cache_returns_shared_arc() {
        let a = cached_regex_with("^x+$", false, RegexKind::Regex).unwrap();
        let b = cached_regex_with("^x+$", false, RegexKind::Regex).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        // Same text, different kind / flags → different entries.
        let c = cached_regex_with("^x+$", true, RegexKind::Regex).unwrap();
        assert!(!Arc::ptr_eq(&a, &c));
        let like = cached_regex_with("x%", false, RegexKind::Like).unwrap();
        assert!(like.is_match("xyz"));
        assert!(cached_regex_with("(", false, RegexKind::Regex).is_err());
        // The single-argument form is the case-sensitive regex entry.
        assert!(Arc::ptr_eq(&a, &cached_regex_arc("^x+$").unwrap()));
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
        // The suffix now counts towards the length (docs site: "Hello
        // World", 8 → "Hello..."); this used to assert 5 → "Hello...".
        assert_eq!(
            call("TRUNCATE_TEXT", &[json!("Hello World"), json!(8)]),
            json!("Hello...")
        );
    }

    #[test]
    fn repeat_separator() {
        assert_eq!(
            call("REPEAT", &[json!("ab"), json!(3), json!(",")]),
            json!("ab,ab,ab")
        );
        assert_eq!(
            call("REPEAT", &[json!("ab"), json!(0), json!(",")]),
            json!("")
        );
        assert!(evaluate("REPEAT", &[json!("a"), json!(600_000), json!("-")]).is_err());
    }

    #[test]
    fn truncate_suffix_and_ellipsis() {
        assert_eq!(
            call("TRUNCATE_TEXT", &[json!("Hello World"), json!(5)]),
            json!("He...")
        );
        assert_eq!(
            call("ELLIPSIS", &[json!("Hello World"), json!(7), json!("…")]),
            json!("Hello…")
        );
        assert_eq!(
            call("TRUNCATE_TEXT", &[json!("short"), json!(10), json!("…")]),
            json!("short")
        );
        assert_eq!(
            call("TRUNCATE_TEXT", &[json!("Hello World"), json!(2)]),
            json!("..")
        );
    }

    #[test]
    fn mask_char_and_optional_start() {
        assert_eq!(
            call("MASK", &[json!("4111111111111111"), json!(0), json!(-4)]),
            json!("************1111")
        );
        assert_eq!(
            call(
                "MASK",
                &[json!("secret"), Value::Null, json!(-2), json!("#")]
            ),
            json!("####et")
        );
        assert_eq!(call("MASK", &[json!("secret")]), json!("*****t"));
    }

    #[test]
    fn ltrim_rtrim_chars() {
        assert_eq!(call("LTRIM", &[json!("  foo ")]), json!("foo "));
        assert_eq!(call("RTRIM", &[json!(" foo  ")]), json!(" foo"));
        assert_eq!(
            call("LTRIM", &[json!("--foo--"), json!("-")]),
            json!("foo--")
        );
        assert_eq!(
            call("RTRIM", &[json!("--foo-+"), json!("+-")]),
            json!("--foo")
        );
    }

    #[test]
    fn uri_roundtrip_multibyte() {
        let encoded = call("ENCODE_URI", &[json!("é")]);
        assert_eq!(encoded, json!("%C3%A9"));
        assert_eq!(call("DECODE_URI", &[encoded]), json!("é"));
    }

    #[test]
    fn decode_uri_multibyte_after_percent_does_not_panic() {
        assert_eq!(call("DECODE_URI", &[json!("%aé")]), json!("%aé"));
        assert_eq!(call("DECODE_URI", &[json!("%é1")]), json!("%é1"));
        assert_eq!(call("DECODE_URI", &[json!("100%")]), json!("100%"));
        // `+` is literal for the decodeURIComponent family, a space for URL_DECODE.
        assert_eq!(call("DECODE_URI_COMPONENT", &[json!("a+b")]), json!("a+b"));
        assert_eq!(call("DECODE_URI", &[json!("a+b")]), json!("a+b"));
        assert_eq!(call("URL_DECODE", &[json!("a+b%21")]), json!("a b!"));
    }

    #[test]
    fn replace_empty_search_is_identity_and_output_is_capped() {
        assert_eq!(
            call("REPLACE", &[json!("abc"), json!(""), json!("x")]),
            json!("abc")
        );
        assert_eq!(
            call("SUBSTITUTE", &[json!("abc"), json!(""), json!("x")]),
            json!("abc")
        );
        assert_eq!(
            call("REGEX_REPLACE", &[json!("abc"), json!(""), json!("x")]),
            json!("abc")
        );
        let big = "a".repeat(10_000);
        let rep = "b".repeat(1_000);
        assert!(evaluate("REPLACE", &[json!(big), json!("a"), json!(rep)]).is_err());
        assert!(evaluate("REGEX_REPLACE", &[json!(big), json!("x*"), json!(rep)]).is_err());
        assert!(evaluate("SUBSTITUTE", &[json!(big), json!("a"), json!(rep)]).is_err());
        // Empty-matching regexes still work below the ceiling.
        assert_eq!(
            call("REGEX_REPLACE", &[json!("ab"), json!("x*"), json!("-")]),
            json!("-a-b-")
        );
        assert_eq!(
            call(
                "REGEX_REPLACE",
                &[json!("john smith"), json!("(\\w+) (\\w+)"), json!("$2 $1")]
            ),
            json!("smith john")
        );
    }

    #[test]
    fn substitute_single_pass_arrays_and_limit() {
        // Mapping no longer chains: "a"→"b" then "b"→"c" must not give "cc".
        assert_eq!(
            call("SUBSTITUTE", &[json!("ab"), json!({"a": "b", "b": "c"})]),
            json!("bc")
        );
        // Leftmost-longest.
        assert_eq!(
            call(
                "SUBSTITUTE",
                &[
                    json!("the quick fox"),
                    json!(["quick", "qu"]),
                    json!(["slow", "X"])
                ]
            ),
            json!("the slow fox")
        );
        // Array search, string replace.
        assert_eq!(
            call(
                "SUBSTITUTE",
                &[json!("a-b_c"), json!(["-", "_"]), json!(" ")]
            ),
            json!("a b c")
        );
        // Fewer replacements than searches: the rest are removed.
        assert_eq!(
            call(
                "SUBSTITUTE",
                &[json!("abc"), json!(["a", "b"]), json!(["X"])]
            ),
            json!("Xc")
        );
        // Omitted replace removes matches.
        assert_eq!(
            call("SUBSTITUTE", &[json!("banana"), json!("a")]),
            json!("bnn")
        );
        // Global limit across all searches.
        assert_eq!(
            call(
                "SUBSTITUTE",
                &[
                    json!("abab"),
                    json!(["a", "b"]),
                    json!(["1", "2"]),
                    json!(3)
                ]
            ),
            json!("121b")
        );
        assert_eq!(
            call("SUBSTITUTE", &[json!("aaa"), json!({"a": "b"}), json!(2)]),
            json!("bba")
        );
    }

    #[test]
    fn split_limit_truncates_and_array_separators() {
        assert_eq!(
            call("SPLIT", &[json!("foo-bar-baz"), json!("-"), json!(2)]),
            json!(["foo", "bar"])
        );
        assert_eq!(
            call("SPLIT", &[json!("foo-bar-baz"), json!("-"), json!(0)]),
            json!([])
        );
        assert_eq!(
            call("SPLIT", &[json!("a-b_c--d"), json!(["-", "_", "--"])]),
            json!(["a", "b", "c", "d"])
        );
        assert_eq!(
            call("SPLIT", &[json!("a-b_c"), json!(["-", "_"]), json!(2)]),
            json!(["a", "b"])
        );
        assert_eq!(
            call("SPLIT", &[json!("abc"), json!("")]),
            json!(["a", "b", "c"])
        );
        assert_eq!(
            call("SPLIT", &[json!("a,b,"), json!(",")]),
            json!(["a", "b", ""])
        );
    }

    #[test]
    fn concat_family() {
        assert_eq!(
            call(
                "CONCAT",
                &[json!("a"), json!(1), Value::Null, json!([2.0, "b"])]
            ),
            json!("a12b")
        );
        assert_eq!(
            call(
                "CONCAT_SEPARATOR",
                &[json!(","), json!("a"), Value::Null, json!("b")]
            ),
            json!("a,b")
        );
        assert_eq!(
            call("CONCAT_SEPARATOR", &[json!(", "), json!(["a", null, "c"])]),
            json!("a, c")
        );
        assert_eq!(call("CONCAT", &[json!(1.5), json!(true)]), json!("1.5true"));
    }

    #[test]
    fn stringify_integral_floats() {
        assert_eq!(stringify(&json!(2.0)), "2");
        assert_eq!(stringify(&json!(-3.0)), "-3");
        assert_eq!(stringify(&json!(2.5)), "2.5");
        assert_eq!(stringify(&json!(1e300)), "1e+300");
        assert_eq!(stringify(&Value::Null), "");
    }

    #[test]
    fn title_case_keeps_whitespace() {
        assert_eq!(
            call("TITLE_CASE", &[json!("hello  wORLD\tfoo")]),
            json!("Hello  World\tFoo")
        );
    }

    #[test]
    fn substring_bytes() {
        assert_eq!(
            call("SUBSTRING_BYTES", &[json!("hello"), json!(1), json!(3)]),
            json!("ell")
        );
        assert_eq!(
            call("SUBSTRING_BYTES", &[json!("café"), json!(3)]),
            json!("é")
        );
        // Splitting "é" (bytes 3..5) → null.
        assert_eq!(
            call("SUBSTRING_BYTES", &[json!("café"), json!(4)]),
            Value::Null
        );
        assert_eq!(
            call("SUBSTRING_BYTES", &[json!("hello"), json!(-2)]),
            json!("lo")
        );
    }

    #[test]
    fn ipv4_functions() {
        assert_eq!(
            call("IPV4_TO_NUMBER", &[json!("127.0.0.1")]),
            json!(2130706433u32)
        );
        assert_eq!(call("IPV4_TO_NUMBER", &[json!("1.2.3.04")]), Value::Null);
        assert_eq!(call("IPV4_TO_NUMBER", &[json!("256.0.0.1")]), Value::Null);
        assert_eq!(call("IPV4_TO_NUMBER", &[json!(1)]), Value::Null);
        assert_eq!(
            call("IPV4_FROM_NUMBER", &[json!(2130706433u32)]),
            json!("127.0.0.1")
        );
        assert_eq!(call("IPV4_FROM_NUMBER", &[json!(0)]), json!("0.0.0.0"));
        assert_eq!(
            call("IPV4_FROM_NUMBER", &[json!(4294967296u64)]),
            Value::Null
        );
        assert_eq!(call("IPV4_FROM_NUMBER", &[json!(-1)]), Value::Null);
        assert_eq!(parse_ipv4("10.0.0"), None);
        assert_eq!(parse_ipv4("10.0.0.1.2"), None);
    }

    #[test]
    fn ngram_similarity_matches_aql_examples() {
        let s = |f: &str, a: &str, b: &str, n: i64| {
            call(f, &[json!(a), json!(b), json!(n)]).as_f64().unwrap()
        };
        let close = |x: f64, y: f64| (x - y).abs() < 1e-9;
        assert!(close(
            s("NGRAM_SIMILARITY", "quick fox", "quick foxx", 2),
            8.0 / 9.0
        ));
        assert!(close(
            s("NGRAM_SIMILARITY", "quick fox", "quick foxx", 3),
            7.0 / 8.0
        ));
        assert!(close(
            s("NGRAM_SIMILARITY", "quick fox", "quirky fox", 2),
            5.0 / 9.0
        ));
        assert!(close(
            s("NGRAM_POSITIONAL_SIMILARITY", "quick fox", "quick foxx", 2),
            8.0 / 9.0
        ));
        assert!(close(
            s("NGRAM_POSITIONAL_SIMILARITY", "quick fox", "quick foxx", 3),
            7.0 / 8.0
        ));
        assert!(close(s("NGRAM_SIMILARITY", "abc", "abc", 5), 1.0));
        assert!(evaluate("NGRAM_SIMILARITY", &[json!("a"), json!("b"), json!(0)]).is_err());
        assert_eq!(
            call("NGRAM_SIMILARITY", &[Value::Null, json!("b"), json!(2)]),
            Value::Null
        );
    }

    #[test]
    fn null_propagates() {
        assert_eq!(call("UPPER", &[Value::Null]), Value::Null);
        assert_eq!(call("CONTAINS", &[Value::Null, json!("a")]), Value::Null);
    }
}
