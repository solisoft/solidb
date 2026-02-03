//! Lua globals setup helpers
//!
//! This module contains helper functions for setting up Lua globals.
//! Each submodule handles a specific category of functions.

mod crypto;
mod http;
mod json;
mod stdlib;
mod string_ext;
mod table_ext;
mod time;
mod time_ext;

pub use crypto::setup_crypto_globals;
pub use http::create_fetch_function;
pub use json::{setup_json_globals, setup_json_globals_static};
pub use stdlib::setup_table_extensions;
pub use string_ext::setup_string_extensions;
pub use table_ext::setup_table_lib_extensions;
pub use time::setup_time_globals;
pub use time_ext::setup_time_ext_globals;
