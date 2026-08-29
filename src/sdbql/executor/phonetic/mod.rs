use crate::error::DbResult;
use serde_json::Value;

#[allow(clippy::module_name_repetitions)]
pub mod date;
pub mod id;
pub mod math;
#[allow(clippy::module_inception)]
pub mod phonetic;
pub mod string;

pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    let name_upper = name.to_uppercase();
    let name = name_upper.as_str();

    // Route by name so UPPER/CONTAINS/MIN don't walk DATE_*/SQRT matches.
    if name.starts_with("DATE_")
        || name.starts_with("NOW")
        || name == "TIME_BUCKET"
        || name == "HUMAN_TIME"
        || name == "UUIDV4"
        || name == "UUIDV7"
    {
        return date::evaluate(name, args);
    }
    if matches!(name, "UUID" | "UUID_V4" | "UUID_V7" | "ULID" | "NANOID") {
        return id::evaluate(name, args);
    }
    if name.starts_with("SOUNDEX")
        || matches!(
            name,
            "METAPHONE"
                | "DOUBLE_METAPHONE"
                | "COLOGNE_PHONETIC"
                | "COLOGNE"
                | "CAVERPHONE"
                | "NYSIIS"
        )
    {
        return phonetic::evaluate(name, args);
    }
    if matches!(
        name,
        "HIGHLIGHT" | "SLUGIFY" | "SANITIZE" | "IS_EMAIL" | "IS_URL" | "IS_UUID" | "IS_BLANK"
    ) {
        return string::evaluate(name, args);
    }
    // Math duplicates live in builtins::math — do not shadow them here.

    Ok(None)
}
