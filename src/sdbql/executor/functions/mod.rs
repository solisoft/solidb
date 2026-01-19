//! Built-in functions for SDBQL
//!
//! This module organizes standalone built-in functions by category.
//! Complex functions requiring executor context remain in evaluate.rs.

pub mod type_fn;
pub mod math_fn;
pub mod string_fn;
pub mod array_fn;
