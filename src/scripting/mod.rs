//! Lua Scripting Engine for Custom API Endpoints
//!
//! This module provides embedded Lua scripting capabilities that allow users
//! to create custom API endpoints with full access to database operations.

mod ai_bindings;
mod auth;
pub mod channel_manager;
mod conversion;
mod dev_tools;
pub mod engine;
mod error_handling;
mod file_handling;
mod http_helpers;
mod jwt;
mod lua_globals;
mod string_utils;
mod types;
mod validation;

pub use auth::ScriptUser;
pub use channel_manager::ChannelManager;
pub use conversion::{json_to_lua, lua_to_json_value, lua_value_to_json, matches_filter};
pub use engine::ScriptEngine;

pub use types::{Script, ScriptContext, ScriptResult, ScriptStats};
