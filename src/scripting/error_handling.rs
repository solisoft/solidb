//! Enhanced Lua Error Handling Methods
//!
//! This module provides standardized error handling, assertion utilities,
//! and try-catch patterns for Lua scripts in SoliDB.

use mlua::{Function, Lua, Result as LuaResult, Value as LuaValue};
use serde_json::Value as JsonValue;

use crate::scripting::lua_to_json_value;

/// Create solidb.error(message, code) -> never returns function
pub fn create_error_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(move |_, (message, code): (String, Option<u16>)| {
        let error_code = code.unwrap_or(500);
        Err::<LuaValue, mlua::Error>(mlua::Error::RuntimeError(format!(
            "ERROR:{}:{}",
            error_code, message
        )))
    })
}

/// Create solidb.assert(condition, message) -> boolean or error function
#[allow(dead_code)]
pub fn create_assert_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(move |_, (condition, message): (bool, String)| {
        if !condition {
            return Err(mlua::Error::RuntimeError(format!("ASSERT:{}", message)));
        }
        Ok(true)
    })
}

/// Create solidb.try(fn, catch_fn) -> result function
pub fn create_try_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_async_function(
        move |lua, (try_fn, catch_fn): (Function, Option<Function>)| async move {
            match try_fn.call_async::<LuaValue>(LuaValue::Nil).await {
                Ok(result) => Ok(result),
                Err(e) => {
                    if let Some(catch) = catch_fn {
                        match catch
                            .call_async::<LuaValue>(LuaValue::String(
                                lua.create_string(e.to_string())?,
                            ))
                            .await
                        {
                            Ok(catch_result) => Ok(catch_result),
                            Err(catch_error) => Err(mlua::Error::RuntimeError(format!(
                                "Catch function failed: {}",
                                catch_error
                            ))),
                        }
                    } else {
                        Err(e)
                    }
                }
            }
        },
    )
}

/// Create solidb.panic(message) -> function for testing error handling
#[allow(dead_code)]
pub fn create_panic_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(move |_, message: String| {
        Err::<LuaValue, mlua::Error>(mlua::Error::RuntimeError(format!("PANIC:{}", message)))
    })
}

/// Create solidb.validate_condition(condition, error_message, error_code) function
pub fn create_validate_condition_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(
        move |_, (condition, message, code): (bool, String, Option<u16>)| {
            if !condition {
                let error_code = code.unwrap_or(400);
                return Err(mlua::Error::RuntimeError(format!(
                    "VALIDATION:{}:{}",
                    error_code, message
                )));
            }
            Ok(true)
        },
    )
}

/// Create solidb.check_permissions(user, required_permissions) function
pub fn create_check_permissions_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(
        move |lua, (user_json, required_json): (LuaValue, LuaValue)| {
            let user = lua_to_json_value(lua, user_json)?;
            let required = lua_to_json_value(lua, required_json)?;

            if let (JsonValue::Object(user_obj), JsonValue::Array(required_array)) =
                (&user, &required)
            {
                let user_permissions =
                    if let Some(JsonValue::Array(perm_array)) = user_obj.get("permissions") {
                        perm_array
                            .iter()
                            .filter_map(|p| p.as_str())
                            .collect::<std::collections::HashSet<_>>()
                    } else {
                        std::collections::HashSet::new()
                    };

                for perm in required_array {
                    if let Some(required_perm) = perm.as_str() {
                        if !user_permissions.contains(required_perm) {
                            return Err(mlua::Error::RuntimeError(format!(
                                "PERMISSION_DENIED:Missing permission: {}",
                                required_perm
                            )));
                        }
                    }
                }

                Ok(true)
            } else {
                Err(mlua::Error::RuntimeError(
                    "Invalid user or permissions format".to_string(),
                ))
            }
        },
    )
}

/// Create solidb.rate_limit(identifier, max_requests, window_seconds) function
pub fn create_rate_limit_function(lua: &Lua) -> LuaResult<Function> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    // Simple in-memory rate limiter
    struct RateLimiter {
        requests: Arc<Mutex<HashMap<String, Vec<u64>>>>,
    }

    impl RateLimiter {
        fn new() -> Self {
            Self {
                requests: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        fn check_limit(&self, identifier: &str, max_requests: u32, window_seconds: u64) -> bool {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();

            let mut requests = self.requests.lock().unwrap();
            let user_requests = requests
                .entry(identifier.to_string())
                .or_insert_with(Vec::new);

            // Remove old requests outside the window
            user_requests.retain(|&timestamp| timestamp > now - window_seconds);

            // Check if under limit
            if user_requests.len() < max_requests as usize {
                user_requests.push(now);
                true
            } else {
                false
            }
        }
    }

    use std::sync::OnceLock;
    static RATE_LIMITER: OnceLock<RateLimiter> = OnceLock::new();
    let limiter = RATE_LIMITER.get_or_init(RateLimiter::new);

    lua.create_function(
        move |_, (identifier, max_requests, window_seconds): (String, u32, u64)| {
            if limiter.check_limit(&identifier, max_requests, window_seconds) {
                Ok(true)
            } else {
                Err(mlua::Error::RuntimeError(format!(
                    "RATE_LIMIT:{}:Too many requests",
                    429
                )))
            }
        },
    )
}

/// Create solidb.timeout(ms, fn) -> result with timeout function
pub fn create_timeout_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_async_function(
        move |_lua, (timeout_ms, func): (u64, Function)| async move {
            match tokio::time::timeout(
                Duration::from_millis(timeout_ms),
                func.call_async::<LuaValue>(LuaValue::Nil),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(mlua::Error::RuntimeError(format!(
                    "TIMEOUT:Operation timed out after {}ms",
                    timeout_ms
                ))),
            }
        },
    )
}

/// Create solidb.retry(max_attempts, delay_ms, fn) -> result function
pub fn create_retry_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_async_function(
        move |_lua, (max_attempts, delay_ms, func): (u32, u64, Function)| {
            let func_clone = func.clone();
            async move {
                for attempt in 1..=max_attempts {
                    match func_clone.call_async::<LuaValue>(LuaValue::Nil).await {
                        Ok(result) => return Ok(result),
                        Err(e) => {
                            if attempt == max_attempts {
                                return Err(mlua::Error::RuntimeError(format!(
                                    "RETRY_FAILED:All {} attempts failed. Last error: {}",
                                    max_attempts, e
                                )));
                            }
                            // Wait before retry
                            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        }
                    }
                }
                unreachable!()
            }
        },
    )
}

/// Create solidb.fallback(primary_fn, fallback_fn) -> try primary, fallback on error function
pub fn create_fallback_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_async_function(
        move |_lua, (primary_fn, fallback_fn): (Function, Function)| {
            async move {
                match primary_fn.call_async::<LuaValue>(LuaValue::Nil).await {
                    Ok(result) => Ok(result),
                    Err(primary_error) => {
                        // Log the primary error but use fallback
                        tracing::warn!(
                            "Primary function failed, using fallback: {}",
                            primary_error
                        );
                        fallback_fn.call_async::<LuaValue>(LuaValue::Nil).await
                    }
                }
            }
        },
    )
}

/// Create solidb.validate_input(input, rules) function
pub fn create_validate_input_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(
        move |lua, (input_value, rules_value): (LuaValue, LuaValue)| {
            let input = lua_to_json_value(lua, input_value)?;
            let rules = lua_to_json_value(lua, rules_value)?;

            if let JsonValue::Object(rules_obj) = rules {
                // Validate each field according to rules
                for (field_name, field_rules) in rules_obj {
                    if let JsonValue::Object(rules_map) = field_rules {
                        // Check required fields
                        if let Some(JsonValue::Bool(true)) = rules_map.get("required") {
                            if input.get(&field_name).is_none() {
                                return Err(mlua::Error::RuntimeError(format!(
                                    "VALIDATION:Required field missing: {}",
                                    field_name
                                )));
                            }
                        }

                        // Check type
                        if let Some(expected_type) = rules_map.get("type") {
                            if let (Some(expected_str), Some(field_value)) =
                                (expected_type.as_str(), input.get(&field_name))
                            {
                                let type_matches = match expected_str {
                                    "string" => field_value.is_string(),
                                    "number" => field_value.is_number(),
                                    "boolean" => field_value.is_boolean(),
                                    "array" => field_value.is_array(),
                                    "object" => field_value.is_object(),
                                    _ => false,
                                };

                                if !type_matches {
                                    return Err(mlua::Error::RuntimeError(format!(
                                        "VALIDATION:Field {} must be of type {}",
                                        field_name, expected_str
                                    )));
                                }
                            }
                        }

                        // Check minimum/maximum for numbers
                        if let (Some(field_value), Some(JsonValue::Number(min_val))) =
                            (input.get(&field_name), rules_map.get("min"))
                        {
                            if let Some(num) = field_value.as_f64() {
                                if num < min_val.as_f64().unwrap() {
                                    return Err(mlua::Error::RuntimeError(format!(
                                        "VALIDATION:Field {} must be at least {}",
                                        field_name, min_val
                                    )));
                                }
                            }
                        }

                        if let (Some(field_value), Some(JsonValue::Number(max_val))) =
                            (input.get(&field_name), rules_map.get("max"))
                        {
                            if let Some(num) = field_value.as_f64() {
                                if num > max_val.as_f64().unwrap() {
                                    return Err(mlua::Error::RuntimeError(format!(
                                        "VALIDATION:Field {} must be at most {}",
                                        field_name, max_val
                                    )));
                                }
                            }
                        }
                    }
                }
            }

            Ok(true)
        },
    )
}

use std::time::Duration;

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    #[test]
    fn test_error_function() {
        let lua = Lua::new();
        let error_fn = create_error_function(&lua).unwrap();

        let result: Result<LuaValue, _> = error_fn.call(("Test error".to_string(), Some(400)));
        match result {
            Ok(_) => panic!("Expected error"),
            Err(e) => assert!(e.to_string().contains("ERROR:400:Test error")),
        }
    }

    #[test]
    fn test_assert_success() {
        let lua = Lua::new();
        let assert_fn = create_assert_function(&lua).unwrap();

        let result: Result<bool, _> = assert_fn.call((true, "Should not fail".to_string()));
        assert_eq!(result.unwrap(), true);
    }

    #[test]
    fn test_assert_failure() {
        let lua = Lua::new();
        let assert_fn = create_assert_function(&lua).unwrap();

        let result: Result<LuaValue, _> = assert_fn.call((false, "Should fail".to_string()));
        match result {
            Ok(_) => panic!("Expected error"),
            Err(e) => assert!(e.to_string().contains("ASSERT:Should fail")),
        }
    }
}
