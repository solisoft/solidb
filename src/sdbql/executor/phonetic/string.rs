//! Search/sanitize helpers. AQL string functions live in `builtins/string.rs`.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

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
}
