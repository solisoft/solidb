//! Forwards to the single implementation in `builtins/datetime.rs` so
//! date functions still hit on the first phonetic dispatch slot.

use crate::error::DbResult;
use crate::sdbql::executor::builtins::datetime;
use serde_json::Value;

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    datetime::evaluate(name, args)
}
