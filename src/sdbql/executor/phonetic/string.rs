//! Search/sanitize helpers. AQL string functions live in `builtins/string.rs`.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeMap;

use crate::error::{DbError, DbResult};

static EMAIL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap());
static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^https?://[^\s/$.?#].[^\s]*$").unwrap());
static UUID_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")
        .unwrap()
});

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "HIGHLIGHT" => highlight(args),
        "SNIPPET" => snippet(args),
        "SLUGIFY" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "SLUGIFY requires exactly 1 argument".to_string(),
                ));
            }
            match &args[0] {
                Value::String(s) => Ok(Some(Value::String(slug::slugify(s)))),
                Value::Null => Ok(Some(Value::Null)),
                _ => Err(DbError::ExecutionError(
                    "SLUGIFY requires a string argument".to_string(),
                )),
            }
        }
        "SANITIZE" => sanitize(args),
        "IS_EMAIL" => Ok(Some(Value::Bool(
            args.first()
                .and_then(Value::as_str)
                .is_some_and(|s| EMAIL_RE.is_match(s)),
        ))),
        "IS_URL" => Ok(Some(Value::Bool(
            args.first()
                .and_then(Value::as_str)
                .is_some_and(|s| URL_RE.is_match(s)),
        ))),
        "IS_UUID" => Ok(Some(Value::Bool(
            args.first()
                .and_then(Value::as_str)
                .is_some_and(|s| UUID_RE.is_match(s)),
        ))),
        "IS_BLANK" => Ok(Some(Value::Bool(match args.first() {
            Some(Value::String(s)) => s.trim().is_empty(),
            Some(Value::Null) | None => true,
            _ => false,
        }))),
        _ => Ok(None),
    }
}

/// How far a passage may grow, in characters, to finish the word it cut.
const SNIPPET_WORD_SLACK: usize = 16;

/// `SNIPPET(hit, width)` — `width` characters of `hit.doc[hit.field]`, cut
/// around the tokens that matched. `hit.spans` is `[[start, end], …]` in
/// characters. Without spans, `matched` or `terms` are sought
/// case-insensitively; with neither, the passage is the start of the field.
fn snippet(args: &[Value]) -> DbResult<Option<Value>> {
    if args.len() != 2 {
        return Err(DbError::ExecutionError(
            "SNIPPET requires exactly 2 arguments: hit, width".to_string(),
        ));
    }
    if args[0].is_null() {
        return Ok(Some(Value::Null));
    }
    let Some(hit) = args[0].as_object() else {
        return Err(DbError::ExecutionError(
            "SNIPPET: hit must be an object".to_string(),
        ));
    };
    let width = match args.get(1) {
        Some(Value::Number(n)) => n.as_u64().ok_or_else(|| {
            DbError::ExecutionError("SNIPPET: width must be a non-negative integer".to_string())
        })?,
        _ => {
            return Err(DbError::ExecutionError(
                "SNIPPET: width must be a non-negative integer".to_string(),
            ))
        }
    };
    let width = usize::try_from(width).unwrap_or(usize::MAX);
    let field = hit.get("field").and_then(Value::as_str).unwrap_or("");
    let Some(text) = hit.get("doc").and_then(|doc| field_text(doc, field)) else {
        return Ok(Some(Value::Null));
    };
    if width == 0 {
        return Ok(Some(Value::String(String::new())));
    }
    let chars: Vec<char> = text.chars().collect();
    if width >= chars.len() {
        return Ok(Some(Value::String(text)));
    }

    let mut spans = parse_spans(hit.get("spans"), chars.len());
    if spans.is_empty() {
        spans = spans_for_terms(&chars, &term_list(hit));
    }
    let (start, end) = best_window(chars.len(), width, &spans);
    let start = snap_start(&chars, start);
    let end = snap_end(&chars, end);
    Ok(Some(Value::String(render_window(
        &text, &chars, start, end,
    ))))
}

fn field_text(doc: &Value, path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let mut current = doc;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    current.as_str().map(str::to_owned)
}

fn term_list(hit: &serde_json::Map<String, Value>) -> Vec<String> {
    for key in ["matched", "terms"] {
        match hit.get(key) {
            Some(Value::Array(arr)) => {
                let terms: Vec<String> = arr
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
                if !terms.is_empty() {
                    return terms;
                }
            }
            Some(Value::String(s)) if !s.is_empty() => return vec![s.clone()],
            _ => {}
        }
    }
    Vec::new()
}

fn parse_spans(value: Option<&Value>, len: usize) -> Vec<(usize, usize)> {
    let Some(Value::Array(arr)) = value else {
        return Vec::new();
    };
    let mut spans = Vec::new();
    for item in arr {
        let Some(pair) = item.as_array() else {
            continue;
        };
        if pair.len() != 2 {
            continue;
        }
        let (Some(start), Some(end)) = (pair[0].as_u64(), pair[1].as_u64()) else {
            continue;
        };
        let (start, end) = (start as usize, end as usize);
        if start < end && start < len {
            spans.push((start, end.min(len)));
        }
    }
    spans
}

/// Case-insensitive whole-word hits. One-char lowercase, so a position in
/// the folded text is the same position in the original.
fn spans_for_terms(chars: &[char], terms: &[String]) -> Vec<(usize, usize)> {
    if terms.is_empty() {
        return Vec::new();
    }
    let folded: Vec<char> = chars.iter().copied().map(fold).collect();
    let mut spans = Vec::new();
    for term in terms {
        let needle: Vec<char> = term.chars().map(fold).collect();
        if needle.is_empty() {
            continue;
        }
        let mut i = 0;
        while i + needle.len() <= folded.len() {
            let after = i + needle.len();
            let word = (i == 0 || !folded[i - 1].is_alphanumeric())
                && (after == folded.len() || !folded[after].is_alphanumeric());
            if word && folded[i..after] == needle[..] {
                spans.push((i, after));
                i = after;
            } else {
                i += 1;
            }
        }
    }
    spans.sort_unstable();
    spans.dedup();
    spans
}

/// Window of `width` characters covering the most spans, then the most
/// characters of those spans, then the earliest start.
fn best_window(len: usize, width: usize, spans: &[(usize, usize)]) -> (usize, usize) {
    if spans.is_empty() || width >= len {
        return (0, width.min(len));
    }
    if spans.len() > 256 {
        return best_window_counted(len, width, spans);
    }
    let max_start = len - width;
    let mut candidates = Vec::with_capacity(spans.len() * 4 + 1);
    candidates.push(0);
    for &(start, end) in spans {
        candidates.push(start.min(max_start));
        candidates.push(end.min(max_start));
        candidates.push(start.saturating_sub(width).min(max_start));
        candidates.push(end.saturating_sub(width).min(max_start));
    }
    candidates.sort_unstable();
    candidates.dedup();

    let mut best_start = 0usize;
    let mut best: Option<(usize, usize, usize)> = None;
    for start in candidates {
        let (count, overlap) = window_score(spans, start, start + width);
        // Invert the start so the greatest key is the earliest window.
        let key = (count, overlap, usize::MAX - start);
        if best.is_none_or(|prev| key > prev) {
            best = Some(key);
            best_start = start;
        }
    }
    (best_start, best_start + width)
}

fn window_score(spans: &[(usize, usize)], start: usize, end: usize) -> (usize, usize) {
    let mut count = 0usize;
    let mut overlap = 0usize;
    for &(a, b) in spans {
        let lo = a.max(start);
        let hi = b.min(end);
        if hi > lo {
            count += 1;
            overlap += hi - lo;
        }
    }
    (count, overlap)
}

/// Same choice as [`best_window`] for a field with hundreds of hits, tracking
/// how many spans overlap each candidate instead of scoring every pair.
fn best_window_counted(len: usize, width: usize, spans: &[(usize, usize)]) -> (usize, usize) {
    let mut spans = spans.to_vec();
    spans.sort_unstable();
    let max_start = len - width;
    let mut candidates = Vec::with_capacity(spans.len() * 2 + 1);
    candidates.push(0);
    for &(start, end) in &spans {
        candidates.push(start.min(max_start));
        candidates.push(end.saturating_sub(width).min(max_start));
    }
    candidates.sort_unstable();
    candidates.dedup();

    let mut active: BTreeMap<usize, usize> = BTreeMap::new();
    let mut next = 0usize;
    let mut count = 0usize;
    let mut best_start = 0usize;
    let mut best_count = 0usize;
    let mut seen = false;
    for start in candidates {
        let win_end = start + width;
        while next < spans.len() && spans[next].0 < win_end {
            *active.entry(spans[next].1).or_insert(0) += 1;
            count += 1;
            next += 1;
        }
        while let Some((&end, _)) = active.iter().next() {
            if end > start {
                break;
            }
            count -= active.remove(&end).unwrap_or(0);
        }
        if !seen || count > best_count || (count == best_count && start < best_start) {
            seen = true;
            best_start = start;
            best_count = count;
        }
    }
    (best_start, best_start + width)
}

fn cuts_word(chars: &[char], i: usize) -> bool {
    i > 0 && i < chars.len() && !chars[i - 1].is_whitespace() && !chars[i].is_whitespace()
}

fn snap_start(chars: &[char], start: usize) -> usize {
    if !cuts_word(chars, start) {
        return start;
    }
    let mut i = start;
    while i > 0 && start - i < SNIPPET_WORD_SLACK && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    if (i == 0 || chars[i - 1].is_whitespace()) && start - i <= SNIPPET_WORD_SLACK {
        i
    } else {
        start
    }
}

fn snap_end(chars: &[char], end: usize) -> usize {
    if !cuts_word(chars, end) {
        return end;
    }
    let mut j = end;
    while j < chars.len() && j - end < SNIPPET_WORD_SLACK && !chars[j].is_whitespace() {
        j += 1;
    }
    if (j == chars.len() || chars[j].is_whitespace()) && j - end <= SNIPPET_WORD_SLACK {
        j
    } else {
        end
    }
}

fn render_window(text: &str, chars: &[char], mut start: usize, mut end: usize) -> String {
    if start > end {
        start = end;
    }
    if start == 0 && end == chars.len() {
        return text.to_string();
    }
    let cut_left = start > 0;
    let cut_right = end < chars.len();
    if cut_left {
        while start < end && chars[start].is_whitespace() {
            start += 1;
        }
    }
    if cut_right {
        while end > start && chars[end - 1].is_whitespace() {
            end -= 1;
        }
    }
    if start >= end {
        return String::new();
    }
    let mut out = String::with_capacity((end - start) + 2);
    if cut_left {
        out.push('\u{2026}');
    }
    out.extend(chars[start..end].iter().copied());
    if cut_right {
        out.push('\u{2026}');
    }
    out
}

/// Append `c` HTML-escaped. HIGHLIGHT's output is HTML (`<b>` tags), so the
/// text around and inside the tags must not carry markup of its own.
fn push_html_escaped(out: &mut String, c: char) {
    match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&#x27;"),
        _ => out.push(c),
    }
}

/// Single-char case fold used on both sides, so text and term positions stay
/// aligned even for characters whose full lowercase is several chars.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn highlight(args: &[Value]) -> DbResult<Option<Value>> {
    let Some(Value::String(text)) = args.first() else {
        return Ok(Some(Value::Null));
    };
    let mut terms: Vec<Vec<char>> = match args.get(1) {
        Some(Value::String(s)) => vec![s.chars().map(fold).collect()],
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(Value::as_str)
            .map(|s| s.chars().map(fold).collect())
            .collect(),
        _ => Vec::new(),
    };
    // An empty term matches at every position without advancing: the loop
    // below would never terminate.
    terms.retain(|t| !t.is_empty());
    terms.sort_by_key(|t| std::cmp::Reverse(t.len()));

    let text_chars: Vec<char> = text.chars().collect();
    let folded: Vec<char> = text_chars.iter().map(|&c| fold(c)).collect();
    let mut result = String::with_capacity(text.len() + 16);
    let mut i = 0;
    while i < text_chars.len() {
        let hit = terms
            .iter()
            .find(|t| folded.get(i..i + t.len()) == Some(t.as_slice()))
            .map(Vec::len);
        match hit {
            Some(len) => {
                result.push_str("<b>");
                for &c in &text_chars[i..i + len] {
                    push_html_escaped(&mut result, c);
                }
                result.push_str("</b>");
                i += len;
            }
            None => {
                push_html_escaped(&mut result, text_chars[i]);
                i += 1;
            }
        }
    }
    Ok(Some(Value::String(result)))
}

/// Remove HTML tags, including an unclosed one at the end (`<script` with no
/// `>`), which the old `<[^>]*>` pass left in place. A tag is `<` followed by
/// a letter, `/`, `!` or `?` — what an HTML tokenizer treats as markup — so a
/// plain `a < b` survives. Removing a tag can join `<` with a following
/// letter (`<<b>script>`), so the pass repeats; if markup still remains
/// after a few passes the input is adversarial and every `<` is dropped.
fn strip_html(input: &str) -> String {
    fn is_tag_start(c: Option<char>) -> bool {
        c.is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'))
    }
    fn one_pass(input: &str) -> (String, bool) {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        let mut removed = false;
        while let Some(c) = chars.next() {
            if c == '<' && is_tag_start(chars.peek().copied()) {
                removed = true;
                for inner in chars.by_ref() {
                    if inner == '>' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        (out, removed)
    }
    fn has_tag(s: &str) -> bool {
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '<' && is_tag_start(chars.peek().copied()) {
                return true;
            }
        }
        false
    }
    let mut current = input.to_string();
    for _ in 0..4 {
        let (next, removed) = one_pass(&current);
        current = next;
        if !removed || !has_tag(&current) {
            return current;
        }
    }
    current.replace('<', "")
}

fn sanitize(args: &[Value]) -> DbResult<Option<Value>> {
    if args.is_empty() || args.len() > 2 {
        return Err(DbError::ExecutionError(
            "SANITIZE requires 1 or 2 arguments (text, options?)".to_string(),
        ));
    }
    match &args[0] {
        Value::String(s) => {
            let mut result = s.clone();
            let options: Vec<String> = if args.len() == 2 {
                match &args[1] {
                    Value::String(opt) => vec![opt.to_lowercase()],
                    Value::Array(arr) => arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                        .collect(),
                    _ => vec!["trim".to_string()],
                }
            } else {
                vec!["trim".to_string()]
            };

            for opt in &options {
                match opt.as_str() {
                    "trim" => result = result.trim().to_string(),
                    "lowercase" | "lower" => result = result.to_lowercase(),
                    "uppercase" | "upper" => result = result.to_uppercase(),
                    "alphanumeric" | "alnum" => {
                        result = result
                            .chars()
                            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
                            .collect();
                    }
                    "alpha" => {
                        result = result
                            .chars()
                            .filter(|c| c.is_alphabetic() || c.is_whitespace())
                            .collect();
                    }
                    "numeric" | "digits" => {
                        result = result
                            .chars()
                            .filter(|c| c.is_numeric() || *c == '.' || *c == '-')
                            .collect();
                    }
                    "email" => {
                        result = result.trim().to_lowercase();
                        result = result
                            .chars()
                            .filter(|c| {
                                c.is_alphanumeric()
                                    || *c == '@'
                                    || *c == '.'
                                    || *c == '_'
                                    || *c == '-'
                                    || *c == '+'
                            })
                            .collect();
                    }
                    "url" => {
                        result = result.trim().to_string();
                        result = result
                            .chars()
                            .filter(|c| {
                                c.is_alphanumeric()
                                    || matches!(
                                        *c,
                                        '-' | '_'
                                            | '.'
                                            | '~'
                                            | ':'
                                            | '/'
                                            | '?'
                                            | '#'
                                            | '['
                                            | ']'
                                            | '@'
                                            | '!'
                                            | '$'
                                            | '&'
                                            | '\''
                                            | '('
                                            | ')'
                                            | '*'
                                            | '+'
                                            | ','
                                            | ';'
                                            | '='
                                            | '%'
                                    )
                            })
                            .collect();
                    }
                    "html" => {
                        result = result
                            .replace('&', "&amp;")
                            .replace('<', "&lt;")
                            .replace('>', "&gt;")
                            .replace('"', "&quot;")
                            .replace('\'', "&#x27;");
                    }
                    "strip_html" => {
                        result = strip_html(&result);
                    }
                    "normalize_whitespace" | "normalize" => {
                        let parts: Vec<&str> = result.split_whitespace().collect();
                        result = parts.join(" ");
                    }
                    _ => {}
                }
            }
            Ok(Some(Value::String(result)))
        }
        Value::Null => Ok(Some(Value::Null)),
        _ => Err(DbError::ExecutionError(
            "SANITIZE requires a string argument".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, args: &[Value]) -> Value {
        evaluate(name, args).unwrap().unwrap()
    }

    #[test]
    fn highlight_empty_term_terminates() {
        assert_eq!(call("HIGHLIGHT", &[json!("abc"), json!("")]), json!("abc"));
        assert_eq!(
            call("HIGHLIGHT", &[json!("abc"), json!(["", "b"])]),
            json!("a<b>b</b>c")
        );
    }

    #[test]
    fn highlight_escapes_html() {
        assert_eq!(
            call("HIGHLIGHT", &[json!("<i>fox</i> & co"), json!("fox")]),
            json!("&lt;i&gt;<b>fox</b>&lt;/i&gt; &amp; co")
        );
        assert_eq!(
            call("HIGHLIGHT", &[json!("a<b"), json!("a<")]),
            json!("<b>a&lt;</b>b")
        );
        assert_eq!(
            call(
                "HIGHLIGHT",
                &[json!("The quick brown fox"), json!(["quick", "FOX"])]
            ),
            json!("The <b>quick</b> brown <b>fox</b>")
        );
    }

    #[test]
    fn sanitize_strip_html_removes_unclosed_and_rebuilt_tags() {
        let strip = |s: &str| call("SANITIZE", &[json!(s), json!("strip_html")]);
        assert_eq!(
            strip("<script>alert('xss')</script>"),
            json!("alert('xss')")
        );
        assert_eq!(strip("hello <script"), json!("hello "));
        assert_eq!(strip("<b>bold</b> text"), json!("bold text"));
        assert_eq!(strip("1 < 2 and 3 > 2"), json!("1 < 2 and 3 > 2"));
        assert_eq!(strip("<<b>script>alert(1)"), json!("alert(1)"));
        // "<scr<script>" is one tag to an HTML tokenizer; what is left is inert.
        assert_eq!(strip("<scr<script>ipt>x"), json!("ipt>x"));
    }

    fn span_of(text: &str, word: &str) -> (u64, u64) {
        let chars: Vec<char> = text.chars().collect();
        let needle: Vec<char> = word.chars().collect();
        let start = chars
            .windows(needle.len())
            .position(|w| w == needle.as_slice())
            .unwrap_or_else(|| panic!("{word:?} not in {text:?}"));
        (start as u64, (start + needle.len()) as u64)
    }

    fn hit(text: &str, words: &[&str]) -> Value {
        let spans: Vec<(u64, u64)> = words.iter().map(|w| span_of(text, w)).collect();
        json!({"doc": {"body": text}, "field": "body", "spans": spans})
    }

    #[test]
    fn snippet_cuts_around_the_match() {
        let gap = " filler".repeat(40);
        let text = format!("PREAMBLE{gap} réplication{gap}ENDING");
        let out = call("SNIPPET", &[hit(&text, &["réplication"]), json!(40)])
            .as_str()
            .unwrap()
            .to_string();
        assert!(out.contains("réplication"), "{out}");
        assert!(!out.contains("PREAMBLE"), "{out}");
        assert!(!out.contains("ENDING"), "{out}");
        assert!(out.contains('\u{2026}'), "{out}");
    }

    #[test]
    fn snippet_prefers_two_close_terms_over_an_earlier_one() {
        let gap = " filler".repeat(40);
        let text = format!("alpha{gap} beta gamma{gap}");
        let out = call(
            "SNIPPET",
            &[hit(&text, &["alpha", "beta", "gamma"]), json!(24)],
        )
        .as_str()
        .unwrap()
        .to_string();
        assert!(out.contains("beta") && out.contains("gamma"), "{out}");
        assert!(!out.contains("alpha"), "{out}");
    }

    #[test]
    fn snippet_returns_a_short_field_unchanged_and_width_zero_empty() {
        let whole = call(
            "SNIPPET",
            &[hit("réplication", &["réplication"]), json!(160)],
        );
        assert_eq!(whole, json!("réplication"));
        let empty = call("SNIPPET", &[hit("réplication", &["réplication"]), json!(0)]);
        assert_eq!(empty, json!(""));
    }

    #[test]
    fn snippet_falls_back_to_matched_terms_and_walks_dotted_fields() {
        let gap = " pad".repeat(30);
        let text = format!("PREAMBLE{gap} TARGET{gap}");
        let out = call(
            "SNIPPET",
            &[
                json!({"doc": {"body": text}, "field": "body", "matched": ["target"]}),
                json!(20),
            ],
        )
        .as_str()
        .unwrap()
        .to_string();
        assert!(out.contains("TARGET"), "{out}");
        assert!(!out.contains("PREAMBLE"), "{out}");

        let nested = "xx réplication yy";
        let (start, end) = span_of(nested, "réplication");
        let out = call(
            "SNIPPET",
            &[
                json!({
                    "doc": {"addr": {"note": nested}},
                    "field": "addr.note",
                    "spans": [[start, end]],
                }),
                json!(160),
            ],
        );
        assert_eq!(out, json!(nested));
    }

    #[test]
    fn snippet_leading_window_without_spans_or_terms() {
        let text = format!("LEADING {}", "z".repeat(80));
        let out = call(
            "SNIPPET",
            &[json!({"doc": {"body": text}, "field": "body"}), json!(10)],
        )
        .as_str()
        .unwrap()
        .to_string();
        assert!(out.starts_with("LEADING"), "{out}");
        assert!(out.ends_with('\u{2026}'), "{out}");
        assert!(!out.starts_with('\u{2026}'), "{out}");
    }

    #[test]
    fn snippet_null_hit_and_bad_arguments() {
        assert_eq!(call("SNIPPET", &[Value::Null, json!(10)]), Value::Null);
        let err = |args: &[Value]| evaluate("SNIPPET", args).unwrap_err().to_string();
        assert!(err(&[]).contains("2 arguments"), "{}", err(&[]));
        assert!(err(&[json!({}), json!(1.5)]).contains("integer"));
        assert!(err(&[json!({}), json!(-1)]).contains("integer"));
        assert!(err(&[json!("nope"), json!(8)]).contains("object"));
        assert_eq!(
            call(
                "SNIPPET",
                &[json!({"doc": {}, "field": "missing"}), json!(8)]
            ),
            Value::Null
        );
    }

    #[test]
    fn snippet_many_spans_still_cover_a_hit() {
        // Sparse hits, then a cluster. The counted path (more than 256
        // spans) has to prefer the cluster, which is what puts it past the
        // preamble.
        let text = format!(
            "PREAMBLE {}DENSE",
            "word xxxxxxxxxx ".repeat(260) + "word word word "
        );
        let chars: Vec<char> = text.chars().collect();
        let needle: Vec<char> = "word".chars().collect();
        let mut spans = Vec::new();
        let mut i = 0;
        while i + needle.len() <= chars.len() {
            if chars[i..i + needle.len()] == needle[..] {
                spans.push(json!([i, i + needle.len()]));
                i += needle.len();
            } else {
                i += 1;
            }
        }
        assert!(spans.len() > 256, "{}", spans.len());
        let out = call(
            "SNIPPET",
            &[
                json!({"doc": {"body": text}, "field": "body", "spans": spans}),
                json!(12),
            ],
        )
        .as_str()
        .unwrap()
        .to_string();
        assert!(out.contains("word"), "{out}");
        assert!(!out.contains("PREAMBLE"), "{out}");
    }
}
