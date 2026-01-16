//! String & Data Utilities for Lua Scripts
//!
//! This module provides string manipulation and table utilities
//! for Lua scripts in SoliDB.

use mlua::{Function, Lua, Result as LuaResult, Table, Value as LuaValue};

/// Create string.slugify(text) -> URL-friendly string
pub fn create_slugify_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|_lua, text: String| {
        let slug = text
            .to_lowercase()
            // Replace accented characters with ASCII equivalents
            .chars()
            .map(|c| match c {
                'á' | 'à' | 'ä' | 'â' | 'ã' => 'a',
                'é' | 'è' | 'ë' | 'ê' => 'e',
                'í' | 'ì' | 'ï' | 'î' => 'i',
                'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
                'ú' | 'ù' | 'ü' | 'û' => 'u',
                'ñ' => 'n',
                'ç' => 'c',
                'ß' => 's',
                _ => c,
            })
            .collect::<String>();

        // Replace non-alphanumeric with hyphens, collapse multiple hyphens
        let re = regex::Regex::new(r"[^a-z0-9]+").unwrap();
        let slug = re.replace_all(&slug, "-");

        // Trim leading/trailing hyphens
        let slug = slug.trim_matches('-').to_string();

        Ok(slug)
    })
}

/// Create string.truncate(text, length, suffix) -> truncated string
pub fn create_truncate_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(
        |_lua, (text, length, suffix): (String, usize, Option<String>)| {
            let suffix = suffix.unwrap_or_else(|| "...".to_string());

            if text.chars().count() <= length {
                return Ok(text);
            }

            // Ensure we have room for the suffix
            let suffix_len = suffix.chars().count();
            if length <= suffix_len {
                return Ok(suffix.chars().take(length).collect());
            }

            let truncate_at = length - suffix_len;
            let truncated: String = text.chars().take(truncate_at).collect();

            // Try to break at a word boundary (space)
            if let Some(last_space) = truncated.rfind(' ') {
                if last_space > truncate_at / 2 {
                    // Only use word boundary if it's not too far back
                    return Ok(format!("{}{}", &truncated[..last_space], suffix));
                }
            }

            Ok(format!("{}{}", truncated, suffix))
        },
    )
}

/// Create string.template(template, vars) -> interpolated string
/// Supports {{var}} syntax for variable substitution
pub fn create_template_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|_lua, (template, vars): (String, LuaValue)| {
        let mut result = template;

        if let LuaValue::Table(t) = vars {
            // Iterate over the table and replace placeholders
            for (key, value) in t.pairs::<String, LuaValue>().flatten() {
                let placeholder = format!("{{{{{}}}}}", key); // {{key}}
                let replacement = match value {
                    LuaValue::String(s) => s.to_str().map(|s| s.to_string()).unwrap_or_default(),
                    LuaValue::Integer(i) => i.to_string(),
                    LuaValue::Number(n) => n.to_string(),
                    LuaValue::Boolean(b) => b.to_string(),
                    LuaValue::Nil => String::new(),
                    _ => format!("{:?}", value),
                };
                result = result.replace(&placeholder, &replacement);
            }
        }

        Ok(result)
    })
}

/// Create string.split(text, delimiter) -> array of strings
pub fn create_split_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, (text, delimiter): (String, Option<String>)| {
        let delimiter = delimiter.unwrap_or_else(|| " ".to_string());
        let parts: Vec<&str> = text.split(&delimiter).collect();

        let table = lua.create_table()?;
        for (i, part) in parts.iter().enumerate() {
            table.set(i + 1, *part)?;
        }

        Ok(table)
    })
}

/// Create string.trim(text) -> trimmed string
pub fn create_trim_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|_lua, text: String| Ok(text.trim().to_string()))
}

/// Create string.pad_left(text, length, char) -> padded string
pub fn create_pad_left_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(
        |_lua, (text, length, pad_char): (String, usize, Option<String>)| {
            let pad_char = pad_char.unwrap_or_else(|| " ".to_string());
            let pad_char = pad_char.chars().next().unwrap_or(' ');
            let text_len = text.chars().count();

            if text_len >= length {
                return Ok(text);
            }

            let padding: String = std::iter::repeat_n(pad_char, length - text_len).collect();
            Ok(format!("{}{}", padding, text))
        },
    )
}

/// Create string.pad_right(text, length, char) -> padded string
pub fn create_pad_right_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(
        |_lua, (text, length, pad_char): (String, usize, Option<String>)| {
            let pad_char = pad_char.unwrap_or_else(|| " ".to_string());
            let pad_char = pad_char.chars().next().unwrap_or(' ');
            let text_len = text.chars().count();

            if text_len >= length {
                return Ok(text);
            }

            let padding: String = std::iter::repeat_n(pad_char, length - text_len).collect();
            Ok(format!("{}{}", text, padding))
        },
    )
}

/// Create string.capitalize(text) -> capitalized string (first letter uppercase)
pub fn create_capitalize_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|_lua, text: String| {
        let mut chars = text.chars();
        match chars.next() {
            None => Ok(String::new()),
            Some(first) => Ok(first.to_uppercase().chain(chars).collect()),
        }
    })
}

/// Create string.title_case(text) -> title case string (each word capitalized)
pub fn create_title_case_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|_lua, text: String| {
        let result = text
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                }
            })
            .collect::<Vec<String>>()
            .join(" ");
        Ok(result)
    })
}

/// Create table.deep_merge(t1, t2) -> merged table (recursive)
pub fn create_deep_merge_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, (t1, t2): (Table, Table)| {
        deep_merge_tables(lua, &t1, &t2)?;
        Ok(t1)
    })
}

/// Recursively merge t2 into t1
fn deep_merge_tables(_lua: &Lua, t1: &Table, t2: &Table) -> LuaResult<()> {
    for pair in t2.pairs::<LuaValue, LuaValue>() {
        let (key, value2) = pair?;

        match (t1.get::<LuaValue>(key.clone())?, value2.clone()) {
            // Both are tables - merge recursively
            (LuaValue::Table(existing), LuaValue::Table(new_table)) => {
                deep_merge_tables(_lua, &existing, &new_table)?;
            }
            // Otherwise, overwrite with new value
            _ => {
                t1.set(key, value2)?;
            }
        }
    }
    Ok(())
}

/// Create table.keys(t) -> array of keys
pub fn create_keys_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, t: Table| {
        let keys = lua.create_table()?;
        let mut i = 1;

        for pair in t.pairs::<LuaValue, LuaValue>() {
            let (key, _) = pair?;
            keys.set(i, key)?;
            i += 1;
        }

        Ok(keys)
    })
}

/// Create table.values(t) -> array of values
pub fn create_values_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, t: Table| {
        let values = lua.create_table()?;
        let mut i = 1;

        for pair in t.pairs::<LuaValue, LuaValue>() {
            let (_, value) = pair?;
            values.set(i, value)?;
            i += 1;
        }

        Ok(values)
    })
}

/// Create table.contains(t, value) -> boolean
pub fn create_contains_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|_lua, (t, search_value): (Table, LuaValue)| {
        for pair in t.pairs::<LuaValue, LuaValue>() {
            let (_, value) = pair?;
            if values_equal(&value, &search_value) {
                return Ok(true);
            }
        }
        Ok(false)
    })
}

/// Compare two Lua values for equality
fn values_equal(a: &LuaValue, b: &LuaValue) -> bool {
    match (a, b) {
        (LuaValue::Nil, LuaValue::Nil) => true,
        (LuaValue::Boolean(a), LuaValue::Boolean(b)) => a == b,
        (LuaValue::Integer(a), LuaValue::Integer(b)) => a == b,
        (LuaValue::Number(a), LuaValue::Number(b)) => (a - b).abs() < f64::EPSILON,
        (LuaValue::Integer(a), LuaValue::Number(b)) => (*a as f64 - b).abs() < f64::EPSILON,
        (LuaValue::Number(a), LuaValue::Integer(b)) => (a - *b as f64).abs() < f64::EPSILON,
        (LuaValue::String(a), LuaValue::String(b)) => a.as_bytes() == b.as_bytes(),
        _ => false,
    }
}

/// Create table.filter(t, predicate) -> filtered table
pub fn create_filter_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, (t, predicate): (Table, Function)| {
        let result = lua.create_table()?;
        let mut i = 1;

        for pair in t.pairs::<LuaValue, LuaValue>() {
            let (key, value) = pair?;
            let keep: bool = predicate.call((value.clone(), key.clone()))?;
            if keep {
                result.set(i, value)?;
                i += 1;
            }
        }

        Ok(result)
    })
}

/// Create table.map(t, transform) -> transformed table
pub fn create_map_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, (t, transform): (Table, Function)| {
        let result = lua.create_table()?;
        let mut i = 1;

        for pair in t.pairs::<LuaValue, LuaValue>() {
            let (key, value) = pair?;
            let transformed: LuaValue = transform.call((value, key))?;
            result.set(i, transformed)?;
            i += 1;
        }

        Ok(result)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    #[test]
    fn test_slugify() {
        let lua = Lua::new();
        let slugify_fn = create_slugify_function(&lua).unwrap();

        let result: String = slugify_fn.call("Hello World!").unwrap();
        assert_eq!(result, "hello-world");

        let result: String = slugify_fn.call("  Multiple   Spaces  ").unwrap();
        assert_eq!(result, "multiple-spaces");

        let result: String = slugify_fn.call("Café résumé").unwrap();
        assert_eq!(result, "cafe-resume");
    }

    #[test]
    fn test_truncate() {
        let lua = Lua::new();
        let truncate_fn = create_truncate_function(&lua).unwrap();

        let result: String = truncate_fn
            .call(("Hello World", 8, None::<String>))
            .unwrap();
        assert!(result.len() <= 8);
        assert!(result.ends_with("..."));

        let result: String = truncate_fn.call(("Short", 10, None::<String>)).unwrap();
        assert_eq!(result, "Short");
    }

    #[test]
    fn test_template() {
        let lua = Lua::new();
        let template_fn = create_template_function(&lua).unwrap();

        let vars = lua.create_table().unwrap();
        vars.set("name", "Alice").unwrap();
        vars.set("age", 30).unwrap();

        let result: String = template_fn
            .call(("Hello {{name}}, you are {{age}}", LuaValue::Table(vars)))
            .unwrap();
        assert_eq!(result, "Hello Alice, you are 30");
    }

    #[test]
    fn test_title_case() {
        let lua = Lua::new();
        let title_case_fn = create_title_case_function(&lua).unwrap();

        let result: String = title_case_fn.call("hello world").unwrap();
        assert_eq!(result, "Hello World");
    }
}
