use regex::Regex;
use serde_json::Value;

use super::super::utils::safe_regex;
use crate::error::{DbError, DbResult};

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "UPPER" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "UPPER requires 1 argument".to_string(),
                ));
            }
            let s = args[0].as_str().ok_or_else(|| {
                DbError::ExecutionError("UPPER: argument must be a string".to_string())
            })?;
            Ok(Some(Value::String(s.to_uppercase())))
        }
        "LOWER" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "LOWER requires 1 argument".to_string(),
                ));
            }
            let s = args[0].as_str().ok_or_else(|| {
                DbError::ExecutionError("LOWER: argument must be a string".to_string())
            })?;
            Ok(Some(Value::String(s.to_lowercase())))
        }
        "TRIM" => {
            if args.is_empty() || args.len() > 2 {
                return Err(DbError::ExecutionError(
                    "TRIM requires 1-2 arguments: value, [type/chars]".to_string(),
                ));
            }
            let value = args[0].as_str().ok_or_else(|| {
                DbError::ExecutionError("TRIM: first argument must be a string".to_string())
            })?;

            let (trim_mode, chars) = if args.len() == 2 {
                if args[1].is_number() {
                    // Type: 0=both, 1=left, 2=right
                    let t = args[1].as_i64().unwrap_or(0);
                    (Some(t), None)
                } else if args[1].is_string() {
                    (None, args[1].as_str())
                } else {
                    (Some(0), None)
                }
            } else {
                (Some(0), None)
            };

            let result = match (trim_mode, chars) {
                (Some(0), None) => value.trim(),
                (Some(1), None) => value.trim_start(),
                (Some(2), None) => value.trim_end(),
                (None, Some(c)) => value.trim_matches(|ch| c.contains(ch)),
                _ => value.trim(),
            };
            Ok(Some(Value::String(result.to_string())))
        }
        "SPLIT" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "SPLIT requires 2-3 arguments: value, separator, [limit]".to_string(),
                ));
            }

            let value = args[0].as_str().ok_or_else(|| {
                DbError::ExecutionError("SPLIT: first argument must be a string".to_string())
            })?;

            let separator = args[1].as_str().ok_or_else(|| {
                DbError::ExecutionError("SPLIT: second argument must be a string".to_string())
            })?;

            let limit = if args.len() > 2 {
                args[2]
                    .as_i64()
                    .or_else(|| args[2].as_f64().map(|f| f as i64))
            } else {
                None
            };

            let parts: Vec<Value> = match limit {
                Some(n) if n > 0 => value
                    .splitn(n as usize, separator)
                    .map(|s| Value::String(s.to_string()))
                    .collect(),
                Some(n) if n < 0 => {
                    let mut p: Vec<Value> = value
                        .rsplitn(n.abs() as usize, separator)
                        .map(|s| Value::String(s.to_string()))
                        .collect();
                    p.reverse();
                    p
                }
                _ => {
                    if separator.is_empty() {
                        value
                            .chars()
                            .map(|c| Value::String(c.to_string()))
                            .collect()
                    } else {
                        value
                            .split(separator)
                            .map(|s| Value::String(s.to_string()))
                            .collect()
                    }
                }
            };

            Ok(Some(Value::Array(parts)))
        }
        "HIGHLIGHT" => {
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

                // Sort terms by length descending to handle overlapping terms (longest first)
                terms.sort_by(|a, b| b.len().cmp(&a.len()));

                let mut result = String::new();
                let mut i = 0;
                let text_chars: Vec<char> = text.chars().collect();
                // Pre-convert terms to lowercase chars for comparison
                let terms_chars: Vec<Vec<char>> = terms
                    .iter()
                    .map(|t| t.to_lowercase().chars().collect())
                    .collect();

                while i < text_chars.len() {
                    let mut matched = false;
                    for term_chars in &terms_chars {
                        if i + term_chars.len() <= text_chars.len() {
                            let slice = &text_chars[i..i + term_chars.len()];
                            // Case-insensitive comparison
                            if slice.iter().zip(term_chars.iter()).all(|(c1, c2)| {
                                c1.to_lowercase().collect::<String>() == c2.to_string()
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
        "SLUGIFY" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "SLUGIFY requires exactly 1 argument".to_string(),
                ));
            }
            match &args[0] {
                Value::String(s) => {
                    let slug_text = slug::slugify(s);
                    Ok(Some(Value::String(slug_text)))
                }
                Value::Null => Ok(Some(Value::Null)),
                _ => Err(DbError::ExecutionError(
                    "SLUGIFY requires a string argument".to_string(),
                )),
            }
        }
        "SANITIZE" => {
            if args.is_empty() || args.len() > 2 {
                return Err(DbError::ExecutionError(
                    "SANITIZE requires 1 or 2 arguments (text, options?)".to_string(),
                ));
            }
            match &args[0] {
                Value::String(s) => {
                    let mut result = s.clone();

                    // Get options - can be a string or array of strings
                    let options: Vec<String> = if args.len() == 2 {
                        match &args[1] {
                            Value::String(opt) => vec![opt.to_lowercase()],
                            Value::Array(arr) => arr
                                .iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                                .collect(),
                            _ => vec!["trim".to_string()], // Default
                        }
                    } else {
                        vec!["trim".to_string()] // Default: just trim
                    };

                    for opt in &options {
                        match opt.as_str() {
                            "trim" => {
                                result = result.trim().to_string();
                            }
                            "lowercase" | "lower" => {
                                result = result.to_lowercase();
                            }
                            "uppercase" | "upper" => {
                                result = result.to_uppercase();
                            }
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
                                // Basic email sanitization: lowercase, trim, remove invalid chars
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
                                // URL-safe characters only
                                result = result.trim().to_string();
                                result = result
                                    .chars()
                                    .filter(|c| {
                                        c.is_alphanumeric()
                                            || *c == '-'
                                            || *c == '_'
                                            || *c == '.'
                                            || *c == '~'
                                            || *c == ':'
                                            || *c == '/'
                                            || *c == '?'
                                            || *c == '#'
                                            || *c == '['
                                            || *c == ']'
                                            || *c == '@'
                                            || *c == '!'
                                            || *c == '$'
                                            || *c == '&'
                                            || *c == '\''
                                            || *c == '('
                                            || *c == ')'
                                            || *c == '*'
                                            || *c == '+'
                                            || *c == ','
                                            || *c == ';'
                                            || *c == '='
                                            || *c == '%'
                                    })
                                    .collect();
                            }
                            "html" => {
                                // Escape HTML entities
                                result = result
                                    .replace('&', "&amp;")
                                    .replace('<', "&lt;")
                                    .replace('>', "&gt;")
                                    .replace('"', "&quot;")
                                    .replace('\'', "&#x27;");
                            }
                            "strip_html" => {
                                // Remove HTML tags using regex
                                let re = regex::Regex::new(r"<[^>]*>").unwrap();
                                result = re.replace_all(&result, "").to_string();
                            }
                            "normalize_whitespace" | "normalize" => {
                                // Replace multiple whitespace with single space
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
        "REGEX_REPLACE" => {
            if args.len() < 3 || args.len() > 4 {
                return Err(DbError::ExecutionError(
                    "REGEX_REPLACE requires 3-4 arguments: text, search, replacement, [caseInsensitive]"
                        .to_string(),
                ));
            }

            let text = args[0].as_str().ok_or_else(|| {
                DbError::ExecutionError(
                    "REGEX_REPLACE: first argument must be a string".to_string(),
                )
            })?;

            let search_pattern = args[1].as_str().ok_or_else(|| {
                DbError::ExecutionError(
                    "REGEX_REPLACE: second argument must be a string (regex)".to_string(),
                )
            })?;

            let replacement = args[2].as_str().ok_or_else(|| {
                DbError::ExecutionError(
                    "REGEX_REPLACE: third argument must be a string".to_string(),
                )
            })?;

            let case_insensitive = if args.len() > 3 {
                args[3].as_bool().unwrap_or(false)
            } else {
                false
            };

            let pattern = if case_insensitive {
                format!("(?i){}", search_pattern)
            } else {
                search_pattern.to_string()
            };

            // Use safe_regex to prevent DoS from malicious patterns
            let re = safe_regex(&pattern)
                .map_err(|e| DbError::ExecutionError(format!("REGEX_REPLACE: {}", e)))?;

            let result = re.replace_all(text, replacement).to_string();
            Ok(Some(Value::String(result)))
        }
        "CONTAINS" => {
            if args.len() < 2 || args.len() > 3 {
                return Err(DbError::ExecutionError(
                    "CONTAINS requires 2-3 arguments: text, search, [returnIndex]".to_string(),
                ));
            }

            let text = args[0].as_str().ok_or_else(|| {
                DbError::ExecutionError("CONTAINS: first argument must be a string".to_string())
            })?;

            let search = args[1].as_str().ok_or_else(|| {
                DbError::ExecutionError("CONTAINS: second argument must be a string".to_string())
            })?;

            let return_index = if args.len() > 2 {
                args[2].as_bool().unwrap_or(false)
            } else {
                false
            };

            if return_index {
                match text.find(search) {
                    Some(index) => Ok(Some(Value::Number(serde_json::Number::from(index)))),
                    None => Ok(Some(Value::Number(serde_json::Number::from(-1)))),
                }
            } else {
                Ok(Some(Value::Bool(text.contains(search))))
            }
        }
        "SUBSTITUTE" => {
            if args.len() < 2 || args.len() > 4 {
                return Err(DbError::ExecutionError(
                    "SUBSTITUTE requires 2-4 arguments".to_string(),
                ));
            }

            let text = args[0].as_str().ok_or_else(|| {
                DbError::ExecutionError("SUBSTITUTE: first argument must be a string".to_string())
            })?;

            let (limit, mapping_mode) = if args[1].is_object() {
                // Mapping mode: SUBSTITUTE(value, mapping, limit?)
                if args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "SUBSTITUTE with mapping requires 2-3 arguments".to_string(),
                    ));
                }
                let limit = if args.len() == 3 {
                    args[2]
                        .as_i64()
                        .or_else(|| args[2].as_f64().map(|f| f as i64))
                } else {
                    None
                };
                (limit, true)
            } else {
                // Replace mode: SUBSTITUTE(value, search, replace, limit?)
                if args.len() < 3 {
                    return Err(DbError::ExecutionError(
                        "SUBSTITUTE requires search and replace strings".to_string(),
                    ));
                }
                let limit = if args.len() == 4 {
                    args[3]
                        .as_i64()
                        .or_else(|| args[3].as_f64().map(|f| f as i64))
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
                let replacements_left = count_limit;

                for (search, replace_val) in mapping {
                    let replace = replace_val.as_str().unwrap_or("");
                    let replace_str = if replace_val.is_string() {
                        replace.to_string()
                    } else {
                        replace_val.to_string()
                    };

                    if let Some(limit_val) = replacements_left {
                        if limit_val == 0 {
                            break;
                        }
                        // Manual replacement with limit
                        let mut new_text = String::new();
                        let mut last_end = 0;
                        let mut count = 0;
                        for (start, part) in result.match_indices(search) {
                            if count >= limit_val {
                                break;
                            }
                            new_text.push_str(&result[last_end..start]);
                            new_text.push_str(&replace_str);
                            last_end = start + part.len();
                            count += 1;
                        }
                        new_text.push_str(&result[last_end..]);
                        result = new_text;
                    } else {
                        result = result.replace(search, &replace_str);
                    }
                }
                Ok(Some(Value::String(result)))
            } else {
                let search = args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "SUBSTITUTE: search argument must be a string".to_string(),
                    )
                })?;
                let replace = args[2].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "SUBSTITUTE: replace argument must be a string".to_string(),
                    )
                })?;

                if let Some(limit_val) = count_limit {
                    let mut new_text = String::new();
                    let mut last_end = 0;
                    let mut count = 0;
                    for (start, part) in text.match_indices(search) {
                        if count >= limit_val {
                            break;
                        }
                        new_text.push_str(&text[last_end..start]);
                        new_text.push_str(replace);
                        last_end = start + part.len();
                        count += 1;
                    }
                    new_text.push_str(&text[last_end..]);
                    Ok(Some(Value::String(new_text)))
                } else {
                    Ok(Some(Value::String(text.replace(search, replace))))
                }
            }
        }
        "TO_STRING" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "TO_STRING requires 1 argument: value".to_string(),
                ));
            }
            let val = &args[0];
            match val {
                Value::Null => Ok(Some(Value::String("".to_string()))),
                Value::String(s) => Ok(Some(Value::String(s.clone()))),
                _ => match serde_json::to_string(val) {
                    Ok(s) => Ok(Some(Value::String(s))),
                    Err(_) => Ok(Some(Value::String("".to_string()))),
                },
            }
        }
        "CAPITALIZE" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "CAPITALIZE requires 1 argument".to_string(),
                ));
            }
            match &args[0] {
                Value::String(s) => {
                    let mut chars = s.chars();
                    let result = match chars.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    };
                    Ok(Some(Value::String(result)))
                }
                Value::Null => Ok(Some(Value::Null)),
                _ => Err(DbError::ExecutionError(
                    "CAPITALIZE requires a string argument".to_string(),
                )),
            }
        }
        "TITLE_CASE" | "INITCAP" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "TITLE_CASE requires 1 argument".to_string(),
                ));
            }
            match &args[0] {
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
                Value::Null => Ok(Some(Value::Null)),
                _ => Err(DbError::ExecutionError(
                    "TITLE_CASE requires a string argument".to_string(),
                )),
            }
        }
        "ENCODE_URI" | "URL_ENCODE" | "ENCODE_URI_COMPONENT" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "ENCODE_URI requires 1 argument".to_string(),
                ));
            }
            match &args[0] {
                Value::String(s) => {
                    let encoded: String = s
                        .chars()
                        .map(|c| match c {
                            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => {
                                c.to_string()
                            }
                            _ => format!("%{:02X}", c as u32),
                        })
                        .collect();
                    Ok(Some(Value::String(encoded)))
                }
                Value::Null => Ok(Some(Value::Null)),
                _ => Err(DbError::ExecutionError(
                    "ENCODE_URI requires a string argument".to_string(),
                )),
            }
        }
        "DECODE_URI" | "URL_DECODE" | "DECODE_URI_COMPONENT" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "DECODE_URI requires 1 argument".to_string(),
                ));
            }
            match &args[0] {
                Value::String(s) => {
                    let mut result = String::new();
                    let mut chars = s.chars().peekable();
                    while let Some(c) = chars.next() {
                        if c == '%' {
                            let hex: String = chars.by_ref().take(2).collect();
                            if hex.len() == 2 {
                                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                                    result.push(byte as char);
                                } else {
                                    result.push('%');
                                    result.push_str(&hex);
                                }
                            } else {
                                result.push('%');
                                result.push_str(&hex);
                            }
                        } else if c == '+' {
                            result.push(' ');
                        } else {
                            result.push(c);
                        }
                    }
                    Ok(Some(Value::String(result)))
                }
                Value::Null => Ok(Some(Value::Null)),
                _ => Err(DbError::ExecutionError(
                    "DECODE_URI requires a string argument".to_string(),
                )),
            }
        }
        "IS_EMAIL" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "IS_EMAIL requires 1 argument".to_string(),
                ));
            }
            match &args[0] {
                Value::String(s) => {
                    let re =
                        Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap();
                    Ok(Some(Value::Bool(re.is_match(s))))
                }
                Value::Null => Ok(Some(Value::Bool(false))),
                _ => Ok(Some(Value::Bool(false))),
            }
        }
        "IS_URL" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "IS_URL requires 1 argument".to_string(),
                ));
            }
            match &args[0] {
                Value::String(s) => {
                    let re = Regex::new(r"^https?://[^\s/$.?#].[^\s]*$").unwrap();
                    Ok(Some(Value::Bool(re.is_match(s))))
                }
                Value::Null => Ok(Some(Value::Bool(false))),
                _ => Ok(Some(Value::Bool(false))),
            }
        }
        "IS_UUID" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "IS_UUID requires 1 argument".to_string(),
                ));
            }
            match &args[0] {
                Value::String(s) => {
                    let re = Regex::new(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$").unwrap();
                    Ok(Some(Value::Bool(re.is_match(s))))
                }
                Value::Null => Ok(Some(Value::Bool(false))),
                _ => Ok(Some(Value::Bool(false))),
            }
        }
        "IS_BLANK" => {
            if args.len() != 1 {
                return Err(DbError::ExecutionError(
                    "IS_BLANK requires 1 argument".to_string(),
                ));
            }
            match &args[0] {
                Value::String(s) => Ok(Some(Value::Bool(s.trim().is_empty()))),
                Value::Null => Ok(Some(Value::Bool(true))),
                _ => Ok(Some(Value::Bool(false))),
            }
        }
        _ => Ok(None),
    }
}
