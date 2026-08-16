//! Search/sanitize helpers. AQL string functions live in `builtins/string.rs`.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

use crate::error::{DbError, DbResult};

static HTML_TAG_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]*>").unwrap());
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

fn highlight(args: &[Value]) -> DbResult<Option<Value>> {
    if let Some(Value::String(text)) = args.first() {
        let terms_arg = args.get(1);
        let mut terms: Vec<String> = Vec::new();

        match terms_arg {
            Some(Value::String(s)) => terms.push(s.clone()),
            Some(Value::Array(arr)) => {
                for v in arr {
                    if let Value::String(s) = v {
                        terms.push(s.clone());
                    }
                }
            }
            _ => {}
        }

        if terms.is_empty() {
            return Ok(Some(Value::String(text.clone())));
        }

        terms.sort_by_key(|b| std::cmp::Reverse(b.len()));

        let mut result = String::new();
        let mut i = 0;
        let text_chars: Vec<char> = text.chars().collect();
        let terms_chars: Vec<Vec<char>> = terms
            .iter()
            .map(|t| t.to_lowercase().chars().collect())
            .collect();

        while i < text_chars.len() {
            let mut matched = false;
            for term_chars in &terms_chars {
                if i + term_chars.len() <= text_chars.len() {
                    let slice = &text_chars[i..i + term_chars.len()];
                    if slice.iter().zip(term_chars.iter()).all(|(c1, c2)| {
                        c1.to_lowercase().next() == Some(*c2)
                            || c1.to_lowercase().collect::<String>() == c2.to_string()
                    }) {
                        result.push_str("<b>");
                        for k in 0..term_chars.len() {
                            result.push(text_chars[i + k]);
                        }
                        result.push_str("</b>");
                        i += term_chars.len();
                        matched = true;
                        break;
                    }
                }
            }

            if !matched {
                result.push(text_chars[i]);
                i += 1;
            }
        }
        Ok(Some(Value::String(result)))
    } else {
        Ok(Some(Value::Null))
    }
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
                        result = HTML_TAG_REGEX.replace_all(&result, "").to_string();
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
