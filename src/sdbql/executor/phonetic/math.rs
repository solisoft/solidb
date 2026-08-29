//! Math functions live in `builtins/math.rs`. This slot is unused.

use crate::error::DbResult;
use serde_json::Value;

pub fn evaluate(_name: &str, _args: &[Value]) -> DbResult<Option<Value>> {
    Ok(None)
}
