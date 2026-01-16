//! Enhanced Lua Validation Methods
//!
//! This module provides JSON schema validation and input sanitization
//! capabilities for Lua scripts in SoliDB.

use jsonschema::Validator;
use mlua::{Function, Lua, Result as LuaResult, Value as LuaValue};
use serde_json::Value as JsonValue;

use crate::scripting::lua_to_json_value;

/// Create solidb.validate(data, schema) -> boolean function
pub fn create_validate_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, (data, schema): (LuaValue, LuaValue)| {
        let json_data = lua_to_json_value(lua, data)?;
        let json_schema = lua_to_json_value(lua, schema)?;

        match Validator::new(&json_schema) {
            Ok(validator) => {
                let is_valid = validator.is_valid(&json_data);
                Ok(is_valid)
            }
            Err(e) => Err(mlua::Error::RuntimeError(format!("Invalid schema: {}", e))),
        }
    })
}

/// Create solidb.validate_detailed(data, schema) -> table function
pub fn create_validate_detailed_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, (data, schema): (LuaValue, LuaValue)| {
        let json_data = lua_to_json_value(lua, data)?;
        let json_schema = lua_to_json_value(lua, schema)?;

        let validator = match Validator::new(&json_schema) {
            Ok(v) => v,
            Err(e) => return Err(mlua::Error::RuntimeError(format!("Invalid schema: {}", e))),
        };

        if validator.is_valid(&json_data) {
            let result = lua.create_table()?;
            result.set("valid", true)?;
            result.set("errors", lua.create_table()?)?;
            return Ok(LuaValue::Table(result));
        }

        let errors = lua.create_table()?;
        let mut error_count = 0;

        for error in validator.iter_errors(&json_data) {
            error_count += 1;
            let error_table = lua.create_table()?;
            error_table.set("message", error.to_string())?;
            error_table.set("path", error.instance_path().to_string())?;
            error_table.set("schema_path", error.schema_path().to_string())?;
            errors.set(error_count, error_table)?;

            // Limit to 50 errors to prevent memory issues
            if error_count >= 50 {
                break;
            }
        }

        let result = lua.create_table()?;
        result.set("valid", false)?;
        result.set("error_count", error_count)?;
        result.set("errors", errors)?;
        Ok(LuaValue::Table(result))
    })
}

/// Create solidb.sanitize(data, operations) -> cleaned_data function
pub fn create_sanitize_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, (data, operations): (LuaValue, LuaValue)| {
        let json_data = lua_to_json_value(lua, data)?;
        let json_ops = lua_to_json_value(lua, operations)?;

        let sanitized = sanitize_value(&json_data, &json_ops);
        json_to_lua(lua, &sanitized)
    })
}

/// Create solidb.typeof(value) -> string function
pub fn create_typeof_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|_lua, value: LuaValue| {
        let type_str = match value {
            LuaValue::String(_) => "string",
            LuaValue::Number(_) => "number",
            LuaValue::Boolean(_) => "boolean",
            LuaValue::Table(_) => "table",
            LuaValue::Function(_) => "function",
            LuaValue::Nil => "nil",
            LuaValue::LightUserData(_) => "userdata",
            LuaValue::Integer(_) => "number", // Lua's type() returns "number" for both
            _ => "unknown",
        };
        Ok(type_str.to_string())
    })
}

/// Helper function to sanitize JSON values based on operations
fn sanitize_value(value: &JsonValue, operations: &JsonValue) -> JsonValue {
    sanitize_value_with_field(value, operations, None)
}

/// Helper function to sanitize JSON values based on operations, with optional field context
fn sanitize_value_with_field(
    value: &JsonValue,
    operations: &JsonValue,
    field_name: Option<&str>,
) -> JsonValue {
    match value {
        JsonValue::Object(obj) => {
            let mut result = serde_json::Map::new();
            for (key, val) in obj {
                let sanitized_val = sanitize_value_with_field(val, operations, Some(key));
                let sanitized_key = if should_sanitize_key(key, operations) {
                    sanitize_string(key, operations, None)
                } else {
                    key.clone()
                };
                result.insert(sanitized_key, sanitized_val);
            }
            JsonValue::Object(result)
        }
        JsonValue::Array(arr) => {
            let result: Vec<JsonValue> = arr
                .iter()
                .map(|item| sanitize_value_with_field(item, operations, field_name))
                .collect();
            JsonValue::Array(result)
        }
        JsonValue::String(s) => JsonValue::String(sanitize_string(s, operations, field_name)),
        other => other.clone(),
    }
}

/// Check if a key should be sanitized
fn should_sanitize_key(key: &str, operations: &JsonValue) -> bool {
    if let JsonValue::Object(ops) = operations {
        if let Some(JsonValue::Bool(true)) = ops.get("trim_keys") {
            return true;
        }

        if let Some(JsonValue::Bool(true)) = ops.get("lowercase_keys") {
            return true;
        }

        if let Some(JsonValue::Array(keys)) = ops.get("sanitize_keys") {
            for key_to_sanitize in keys {
                if let JsonValue::String(k) = key_to_sanitize {
                    if k == key {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Sanitize a string based on operations, with optional field context
fn sanitize_string(s: &str, operations: &JsonValue, field_name: Option<&str>) -> String {
    let mut result = s.to_string();

    if let JsonValue::Object(ops) = operations {
        // Trim whitespace - check for field-specific trim
        let should_trim = match ops.get("trim") {
            Some(JsonValue::Bool(true)) => true,
            Some(JsonValue::Array(fields)) => {
                if let Some(field) = field_name {
                    fields.iter().any(|f| f.as_str() == Some(field))
                } else {
                    false
                }
            }
            _ => false,
        } || ops.get("trim_keys").is_some();

        if should_trim {
            // Trim leading/trailing whitespace and normalize internal whitespace
            result = result.split_whitespace().collect::<Vec<_>>().join(" ");
        }

        // Convert to lowercase - check for field-specific lowercase
        let should_lowercase =
            match ops.get("lowercase") {
                Some(JsonValue::Bool(true)) => true,
                Some(JsonValue::Array(fields)) => {
                    if let Some(field) = field_name {
                        fields.iter().any(|f| f.as_str() == Some(field))
                    } else {
                        false
                    }
                }
                _ => false,
            } || matches!(ops.get("lowercase_keys"), Some(JsonValue::Bool(true)));

        if should_lowercase {
            result = result.to_lowercase();
        }

        // Convert to uppercase - check for field-specific uppercase
        let should_uppercase = match ops.get("uppercase") {
            Some(JsonValue::Bool(true)) => true,
            Some(JsonValue::Array(fields)) => {
                if let Some(field) = field_name {
                    fields.iter().any(|f| f.as_str() == Some(field))
                } else {
                    false
                }
            }
            _ => false,
        };

        if should_uppercase {
            result = result.to_uppercase();
        }

        // Normalize whitespace
        if ops.get("normalize_whitespace").is_some() {
            result = result.split_whitespace().collect::<Vec<_>>().join(" ");
        }

        // Remove HTML tags
        if ops.get("strip_html").is_some() {
            // Simple HTML tag removal
            let re = regex::Regex::new(r"<[^>]*>").unwrap();
            result = re.replace_all(&result, "").to_string();
        }
    }

    result
}

/// Convert JSON value to Lua value (copied from main module)
fn json_to_lua(lua: &Lua, json: &JsonValue) -> LuaResult<LuaValue> {
    match json {
        JsonValue::Null => Ok(LuaValue::Nil),
        JsonValue::Bool(b) => Ok(LuaValue::Boolean(*b)),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(LuaValue::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(LuaValue::Number(f))
            } else {
                Ok(LuaValue::Nil)
            }
        }
        JsonValue::String(s) => Ok(LuaValue::String(lua.create_string(s)?)),
        JsonValue::Array(arr) => {
            let table = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                table.set(i + 1, json_to_lua(lua, v)?)?;
            }
            Ok(LuaValue::Table(table))
        }
        JsonValue::Object(obj) => {
            let table = lua.create_table()?;
            for (k, v) in obj {
                table.set(k.clone(), json_to_lua(lua, v)?)?;
            }
            Ok(LuaValue::Table(table))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_sanitize_string_trim() {
        let operations = json!({"trim": true});
        let result = sanitize_string("  hello world  ", &operations, None);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_sanitize_string_lowercase() {
        let operations = json!({"lowercase": true});
        let result = sanitize_string("HELLO WORLD", &operations, None);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_sanitize_string_normalize_whitespace() {
        let operations = json!({"normalize_whitespace": true});
        let result = sanitize_string("  hello    world   test  ", &operations, None);
        assert_eq!(result, "hello world test");
    }

    #[test]
    fn test_sanitize_string_strip_html() {
        let operations = json!({"strip_html": true});
        let result = sanitize_string("<p>Hello <b>World</b></p>", &operations, None);
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn test_sanitize_object() {
        let operations = json!({"trim": true, "lowercase": true});
        let input = json!({
            "name": "  Alice Smith  ",
            "email": "ALICE@EXAMPLE.COM",
            "age": 30
        });
        let result = sanitize_value(&input, &operations);

        assert_eq!(result["name"], "alice smith");
        assert_eq!(result["email"], "alice@example.com");
        assert_eq!(result["age"], 30); // Numbers unchanged
    }
}
