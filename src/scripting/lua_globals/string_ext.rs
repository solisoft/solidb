//! String library extensions for Lua

use crate::error::DbError;
use crate::scripting::string_utils::*;
use mlua::Lua;

/// Safe regex function with DoS protection
fn safe_regex(pattern: &str) -> Result<regex::Regex, String> {
    // Limit pattern length
    if pattern.len() > 1000 {
        return Err("Pattern too long".to_string());
    }

    // Compile with size limit
    regex::RegexBuilder::new(pattern)
        .size_limit(1024 * 1024) // 1MB compiled size limit
        .dfa_size_limit(1024 * 1024)
        .build()
        .map_err(|e| e.to_string())
}

/// Setup string library extensions (regex, slugify, etc.)
pub fn setup_string_extensions(lua: &Lua) -> Result<(), DbError> {
    let globals = lua.globals();

    let string_table: mlua::Table = globals
        .get("string")
        .map_err(|e| DbError::InternalError(format!("Failed to get string table: {}", e)))?;

    // string.regex(subject, pattern) - Use safe_regex to prevent DoS
    let regex_fn = lua
        .create_function(|_, (s, pattern): (String, String)| {
            let re = safe_regex(&pattern).map_err(|e| mlua::Error::RuntimeError(e))?;
            Ok(re.is_match(&s))
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create regex function: {}", e)))?;
    string_table
        .set("regex", regex_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set string.regex: {}", e)))?;

    // string.regex_replace(subject, pattern, replacement) - Use safe_regex to prevent DoS
    let regex_replace_fn = lua
        .create_function(|_, (s, pattern, replacement): (String, String, String)| {
            let re = safe_regex(&pattern).map_err(|e| mlua::Error::RuntimeError(e))?;
            Ok(re.replace_all(&s, replacement.as_str()).to_string())
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create regex_replace function: {}", e))
        })?;
    string_table
        .set("regex_replace", regex_replace_fn)
        .map_err(|e| {
            DbError::InternalError(format!("Failed to set string.regex_replace: {}", e))
        })?;

    // string.slugify(text) - URL-friendly strings
    let slugify_fn = create_slugify_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create slugify function: {}", e)))?;
    string_table
        .set("slugify", slugify_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set string.slugify: {}", e)))?;

    // string.truncate(text, length, suffix) - Text truncation
    let truncate_fn = create_truncate_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create truncate function: {}", e))
    })?;
    string_table
        .set("truncate", truncate_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set string.truncate: {}", e)))?;

    // string.template(template, vars) - String interpolation with {{var}} syntax
    let template_fn = create_template_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create template function: {}", e))
    })?;
    string_table
        .set("template", template_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set string.template: {}", e)))?;

    // string.split(text, delimiter) - Split string into array
    let split_fn = create_split_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create split function: {}", e)))?;
    string_table
        .set("split", split_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set string.split: {}", e)))?;

    // string.trim(text) - Trim whitespace
    let trim_fn = create_trim_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create trim function: {}", e)))?;
    string_table
        .set("trim", trim_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set string.trim: {}", e)))?;

    // string.pad_left(text, length, char) - Left pad string
    let pad_left_fn = create_pad_left_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create pad_left function: {}", e))
    })?;
    string_table
        .set("pad_left", pad_left_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set string.pad_left: {}", e)))?;

    // string.pad_right(text, length, char) - Right pad string
    let pad_right_fn = create_pad_right_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create pad_right function: {}", e))
    })?;
    string_table
        .set("pad_right", pad_right_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set string.pad_right: {}", e)))?;

    // string.capitalize(text) - Capitalize first letter
    let capitalize_fn = create_capitalize_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create capitalize function: {}", e))
    })?;
    string_table
        .set("capitalize", capitalize_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set string.capitalize: {}", e)))?;

    // string.title_case(text) - Title case (capitalize each word)
    let title_case_fn = create_title_case_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create title_case function: {}", e))
    })?;
    string_table
        .set("title_case", title_case_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set string.title_case: {}", e)))?;

    Ok(())
}
