//! Lua Scripting Engine for Custom API Endpoints
//!
//! This module provides embedded Lua scripting capabilities that allow users
//! to create custom API endpoints with full access to database operations.

//! The Lua runtime itself is the Cargo feature `lua` (off by default). Without
//! it, `engine` is `engine_off.rs`: scripts and services are stored and
//! routed as usual and refused when run, as `--no-lua` refuses them.

#[cfg(feature = "lua")]
mod ai_bindings;
mod auth;
pub mod channel_manager;
#[cfg(feature = "lua")]
pub(crate) mod conversion;
#[cfg(feature = "lua")]
mod dev_tools;
#[cfg(feature = "lua")]
pub mod engine;
#[cfg(not(feature = "lua"))]
#[path = "engine_off.rs"]
pub mod engine;
#[cfg(feature = "lua")]
mod error_handling;
#[cfg(feature = "lua")]
mod file_handling;
#[cfg(feature = "lua")]
mod http_helpers;
#[cfg(feature = "lua")]
mod jwt;
#[cfg(feature = "lua")]
mod lua_globals;
#[cfg(feature = "lua")]
pub mod response;
#[cfg(feature = "lua")]
mod string_utils;
mod types;
#[cfg(feature = "lua")]
mod validation;

pub use auth::ScriptUser;
pub use channel_manager::ChannelManager;
#[cfg(feature = "lua")]
pub use conversion::{json_to_lua, lua_to_json_value, lua_value_to_json, matches_filter};
pub use engine::{lua_disabled_error, lua_runtime_enabled, ScriptEngine};

pub use types::{Script, ScriptContext, ScriptResult, ScriptStats, Service};
