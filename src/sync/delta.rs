//! Delta Sync - JSON Patch for Bandwidth-Efficient Synchronization
//!
//! Implements RFC 6902 JSON Patch format for sending only changed fields
//! instead of full documents, reducing sync bandwidth by 10-100x for large docs.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A JSON Patch operation (RFC 6902)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum PatchOperation {
    /// Add a new value at the target location
    Add {
        /// JSON Pointer path (e.g., "/foo/bar")
        path: String,
        /// Value to add
        value: Value,
    },
    /// Remove the value at the target location
    Remove {
        /// JSON Pointer path
        path: String,
    },
    /// Replace the value at the target location
    Replace {
        /// JSON Pointer path
        path: String,
        /// New value
        value: Value,
    },
    /// Copy value from one location to another
    Copy {
        /// Source JSON Pointer
        from: String,
        /// Destination JSON Pointer
        path: String,
    },
    /// Move value from one location to another
    Move {
        /// Source JSON Pointer
        from: String,
        /// Destination JSON Pointer
        path: String,
    },
    /// Test that value at location equals expected value
    Test {
        /// JSON Pointer path
        path: String,
        /// Expected value
        value: Value,
    },
}

/// A JSON Patch document (array of operations)
pub type JsonPatch = Vec<PatchOperation>;

/// Compute a JSON Patch that transforms `old` into `new`
pub fn compute_diff(old: &Value, new: &Value) -> JsonPatch {
    let mut patch = Vec::new();
    compute_diff_recursive(old, new, "", &mut patch);
    patch
}

/// Recursively compute differences
fn compute_diff_recursive(old: &Value, new: &Value, path: &str, patch: &mut JsonPatch) {
    match (old, new) {
        // Both are objects - compare fields
        (Value::Object(old_map), Value::Object(new_map)) => {
            // Check for removed fields
            for (key, _old_val) in old_map {
                if !new_map.contains_key(key) {
                    let full_path = if path.is_empty() {
                        format!("/{}", escape_json_pointer(key))
                    } else {
                        format!("{}/{}", path, escape_json_pointer(key))
                    };
                    patch.push(PatchOperation::Remove { path: full_path });
                }
            }

            // Check for added or modified fields
            for (key, new_val) in new_map {
                let full_path = if path.is_empty() {
                    format!("/{}", escape_json_pointer(key))
                } else {
                    format!("{}/{}", path, escape_json_pointer(key))
                };

                match old_map.get(key) {
                    Some(old_val) => {
                        if old_val != new_val {
                            // Field modified - recurse for nested objects or arrays
                            if (old_val.is_object() && new_val.is_object())
                                || (old_val.is_array() && new_val.is_array())
                            {
                                compute_diff_recursive(old_val, new_val, &full_path, patch);
                            } else {
                                patch.push(PatchOperation::Replace {
                                    path: full_path,
                                    value: new_val.clone(),
                                });
                            }
                        }
                    }
                    None => {
                        // Field added
                        patch.push(PatchOperation::Add {
                            path: full_path,
                            value: new_val.clone(),
                        });
                    }
                }
            }
        }

        // Both are arrays - element-by-element comparison
        (Value::Array(old_arr), Value::Array(new_arr)) => {
            // If arrays are equal, no patch needed
            if old_arr == new_arr {
                return;
            }

            let max_len = old_arr.len().max(new_arr.len());

            // If >50% size change (or significant absolute change), replace entirely
            // This avoids generating many small patches for drastically changed arrays
            let size_diff = (old_arr.len() as i64 - new_arr.len() as i64).abs();
            let threshold = (max_len / 2).max(5) as i64;
            if size_diff > threshold {
                patch.push(PatchOperation::Replace {
                    path: path.to_string(),
                    value: new.clone(),
                });
                return;
            }

            // Track removed elements (we process in reverse to avoid index shifting)
            let mut removes: Vec<usize> = Vec::new();

            // Element-by-element comparison
            for i in 0..max_len {
                let elem_path = format!("{}/{}", path, i);
                match (old_arr.get(i), new_arr.get(i)) {
                    (Some(old_e), Some(new_e)) if old_e != new_e => {
                        // Element modified - recurse for nested structures
                        compute_diff_recursive(old_e, new_e, &elem_path, patch);
                    }
                    (Some(_), None) => {
                        // Element removed
                        removes.push(i);
                    }
                    (None, Some(new_e)) => {
                        // Element added at end
                        patch.push(PatchOperation::Add {
                            path: format!("{}/-", path),
                            value: new_e.clone(),
                        });
                    }
                    _ => {
                        // Elements are equal or both None - no change needed
                    }
                }
            }

            // Process removes in reverse order to maintain correct indices
            for i in removes.into_iter().rev() {
                patch.push(PatchOperation::Remove {
                    path: format!("{}/{}", path, i),
                });
            }
        }

        // Different types or values - replace
        _ => {
            if old != new {
                patch.push(PatchOperation::Replace {
                    path: path.to_string(),
                    value: new.clone(),
                });
            }
        }
    }
}

/// Apply a JSON Patch to a document
pub fn apply_patch(doc: &mut Value, patch: &JsonPatch) -> Result<(), PatchError> {
    for op in patch {
        apply_operation(doc, op)?;
    }
    Ok(())
}

/// Apply a single patch operation
fn apply_operation(doc: &mut Value, op: &PatchOperation) -> Result<(), PatchError> {
    match op {
        PatchOperation::Add { path, value } => {
            let pointer = json_pointer::JsonPointer::new(path);
            pointer.add(doc, value.clone())?;
        }
        PatchOperation::Remove { path } => {
            let pointer = json_pointer::JsonPointer::new(path);
            pointer.remove(doc)?;
        }
        PatchOperation::Replace { path, value } => {
            let pointer = json_pointer::JsonPointer::new(path);
            pointer.replace(doc, value.clone())?;
        }
        PatchOperation::Copy { from, path } => {
            let from_ptr = json_pointer::JsonPointer::new(from);
            let value = from_ptr
                .get(doc)?
                .cloned()
                .ok_or(PatchError::PathNotFound)?;
            let to_ptr = json_pointer::JsonPointer::new(path);
            to_ptr.add(doc, value)?;
        }
        PatchOperation::Move { from, path } => {
            let from_ptr = json_pointer::JsonPointer::new(from);
            let value = from_ptr.remove(doc)?;
            let to_ptr = json_pointer::JsonPointer::new(path);
            to_ptr.add(doc, value)?;
        }
        PatchOperation::Test { path, value } => {
            let pointer = json_pointer::JsonPointer::new(path);
            let actual = pointer.get(doc)?.ok_or(PatchError::PathNotFound)?;
            if actual != value {
                return Err(PatchError::TestFailed);
            }
        }
    }
    Ok(())
}

/// Escape special characters for JSON Pointer (RFC 6901)
fn escape_json_pointer(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

/// Unescape JSON Pointer
fn _unescape_json_pointer(s: &str) -> String {
    s.replace("~1", "/").replace("~0", "~")
}

/// Error types for patch operations
#[derive(Debug, Clone, PartialEq)]
pub enum PatchError {
    PathNotFound,
    InvalidPath,
    TestFailed,
    CannotReplaceRoot,
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatchError::PathNotFound => write!(f, "Path not found"),
            PatchError::InvalidPath => write!(f, "Invalid JSON Pointer path"),
            PatchError::TestFailed => write!(f, "Test operation failed"),
            PatchError::CannotReplaceRoot => write!(f, "Cannot replace root document"),
        }
    }
}

impl std::error::Error for PatchError {}

/// JSON Pointer implementation (RFC 6901)
mod json_pointer {
    use super::{PatchError, Value};

    pub struct JsonPointer {
        tokens: Vec<String>,
    }

    impl JsonPointer {
        pub fn new(path: &str) -> Self {
            let tokens: Vec<String> = if path.is_empty() || path == "/" {
                vec![]
            } else {
                path.split('/')
                    .skip(1) // Skip empty first element from leading /
                    .map(|s| s.replace("~1", "/").replace("~0", "~"))
                    .collect()
            };
            Self { tokens }
        }

        pub fn get<'a>(&self, doc: &'a Value) -> Result<Option<&'a Value>, PatchError> {
            let mut current = doc;
            for token in &self.tokens {
                current = match current {
                    Value::Object(map) => map.get(token),
                    Value::Array(arr) => {
                        if token == "-" {
                            // Special case: end of array
                            arr.last()
                        } else {
                            token.parse::<usize>().ok().and_then(|i| arr.get(i))
                        }
                    }
                    _ => return Err(PatchError::InvalidPath),
                }
                .ok_or(PatchError::PathNotFound)?;
            }
            Ok(Some(current))
        }

        pub fn add(&self, doc: &mut Value, value: Value) -> Result<(), PatchError> {
            if self.tokens.is_empty() {
                return Err(PatchError::CannotReplaceRoot);
            }

            let mut current = doc;
            let last = self.tokens.len() - 1;

            for (i, token) in self.tokens.iter().enumerate() {
                if i == last {
                    // Final token - add the value
                    match current {
                        Value::Object(map) => {
                            map.insert(token.clone(), value);
                        }
                        Value::Array(arr) => {
                            if token == "-" {
                                arr.push(value);
                            } else {
                                let idx = token
                                    .parse::<usize>()
                                    .map_err(|_| PatchError::InvalidPath)?;
                                if idx > arr.len() {
                                    return Err(PatchError::PathNotFound);
                                }
                                arr.insert(idx, value);
                            }
                        }
                        _ => return Err(PatchError::InvalidPath),
                    }
                    return Ok(());
                }

                // Navigate deeper
                current = match current {
                    Value::Object(map) => map.get_mut(token).ok_or(PatchError::PathNotFound)?,
                    Value::Array(arr) => {
                        let idx = token
                            .parse::<usize>()
                            .map_err(|_| PatchError::InvalidPath)?;
                        arr.get_mut(idx).ok_or(PatchError::PathNotFound)?
                    }
                    _ => return Err(PatchError::InvalidPath),
                };
            }

            Ok(())
        }

        pub fn remove(&self, doc: &mut Value) -> Result<Value, PatchError> {
            if self.tokens.is_empty() {
                return Err(PatchError::CannotReplaceRoot);
            }

            let mut current = doc;
            let last = self.tokens.len() - 1;

            for (i, token) in self.tokens.iter().enumerate() {
                if i == last {
                    // Final token - remove the value
                    return match current {
                        Value::Object(map) => map.remove(token).ok_or(PatchError::PathNotFound),
                        Value::Array(arr) => {
                            let idx = token
                                .parse::<usize>()
                                .map_err(|_| PatchError::InvalidPath)?;
                            if idx >= arr.len() {
                                return Err(PatchError::PathNotFound);
                            }
                            Ok(arr.remove(idx))
                        }
                        _ => Err(PatchError::InvalidPath),
                    };
                }

                // Navigate deeper
                current = match current {
                    Value::Object(map) => map.get_mut(token).ok_or(PatchError::PathNotFound)?,
                    Value::Array(arr) => {
                        let idx = token
                            .parse::<usize>()
                            .map_err(|_| PatchError::InvalidPath)?;
                        arr.get_mut(idx).ok_or(PatchError::PathNotFound)?
                    }
                    _ => return Err(PatchError::InvalidPath),
                };
            }

            Err(PatchError::PathNotFound)
        }

        pub fn replace(&self, doc: &mut Value, value: Value) -> Result<(), PatchError> {
            if self.tokens.is_empty() {
                // Replace root
                *doc = value;
                return Ok(());
            }

            let mut current = doc;
            let last = self.tokens.len() - 1;

            for (i, token) in self.tokens.iter().enumerate() {
                if i == last {
                    // Final token - replace the value
                    match current {
                        Value::Object(map) => {
                            if !map.contains_key(token) {
                                return Err(PatchError::PathNotFound);
                            }
                            map.insert(token.clone(), value);
                        }
                        Value::Array(arr) => {
                            let idx = token
                                .parse::<usize>()
                                .map_err(|_| PatchError::InvalidPath)?;
                            if idx >= arr.len() {
                                return Err(PatchError::PathNotFound);
                            }
                            arr[idx] = value;
                        }
                        _ => return Err(PatchError::InvalidPath),
                    }
                    return Ok(());
                }

                // Navigate deeper
                current = match current {
                    Value::Object(map) => map.get_mut(token).ok_or(PatchError::PathNotFound)?,
                    Value::Array(arr) => {
                        let idx = token
                            .parse::<usize>()
                            .map_err(|_| PatchError::InvalidPath)?;
                        arr.get_mut(idx).ok_or(PatchError::PathNotFound)?
                    }
                    _ => return Err(PatchError::InvalidPath),
                };
            }

            Ok(())
        }
    }
}

/// Utility functions for delta sync
pub mod utils {
    use super::*;

    /// Check if a patch is empty (no changes)
    pub fn is_empty_patch(patch: &JsonPatch) -> bool {
        patch.is_empty()
    }

    /// Get patch size in bytes (for batching decisions)
    pub fn patch_size(patch: &JsonPatch) -> usize {
        serde_json::to_string(patch).map(|s| s.len()).unwrap_or(0)
    }

    /// Check if full document would be smaller than patch
    pub fn should_use_patch(_old: &Value, new: &Value, patch: &JsonPatch) -> bool {
        let patch_bytes = patch_size(patch);
        let full_bytes = serde_json::to_string(new)
            .map(|s| s.len())
            .unwrap_or(usize::MAX);

        // Use patch if it's at least 20% smaller
        patch_bytes < (full_bytes * 8 / 10)
    }

    /// Create a patch that only includes specific fields
    pub fn create_partial_patch(old: &Value, new: &Value, fields: &[&str]) -> JsonPatch {
        let full_patch = compute_diff(old, new);

        full_patch
            .into_iter()
            .filter(|op| {
                let path = match op {
                    PatchOperation::Add { path, .. } => path,
                    PatchOperation::Remove { path } => path,
                    PatchOperation::Replace { path, .. } => path,
                    PatchOperation::Copy { path, .. } => path,
                    PatchOperation::Move { path, .. } => path,
                    PatchOperation::Test { path, .. } => path,
                };

                // Check if path starts with any of the allowed fields
                fields.iter().any(|field| {
                    let field_path = format!("/{}", field);
                    path == &field_path || path.starts_with(&format!("{}/", field_path))
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_diff_add_field() {
        let old = serde_json::json!({"name": "Alice"});
        let new = serde_json::json!({"name": "Alice", "age": 30});

        let patch = compute_diff(&old, &new);
        assert_eq!(patch.len(), 1);
        assert!(matches!(&patch[0], PatchOperation::Add { path, value } 
            if path == "/age" && value == &serde_json::json!(30)));
    }

    #[test]
    fn test_compute_diff_remove_field() {
        let old = serde_json::json!({"name": "Alice", "age": 30});
        let new = serde_json::json!({"name": "Alice"});

        let patch = compute_diff(&old, &new);
        assert_eq!(patch.len(), 1);
        assert!(matches!(&patch[0], PatchOperation::Remove { path } 
            if path == "/age"));
    }

    #[test]
    fn test_compute_diff_replace_field() {
        let old = serde_json::json!({"name": "Alice", "age": 30});
        let new = serde_json::json!({"name": "Alice", "age": 31});

        let patch = compute_diff(&old, &new);
        assert_eq!(patch.len(), 1);
        assert!(matches!(&patch[0], PatchOperation::Replace { path, value } 
            if path == "/age" && value == &serde_json::json!(31)));
    }

    #[test]
    fn test_apply_patch_add() {
        let mut doc = serde_json::json!({"name": "Alice"});
        let patch = vec![PatchOperation::Add {
            path: "/age".to_string(),
            value: serde_json::json!(30),
        }];

        apply_patch(&mut doc, &patch).unwrap();
        assert_eq!(doc["age"], 30);
    }

    #[test]
    fn test_apply_patch_replace() {
        let mut doc = serde_json::json!({"name": "Alice", "age": 30});
        let patch = vec![PatchOperation::Replace {
            path: "/age".to_string(),
            value: serde_json::json!(31),
        }];

        apply_patch(&mut doc, &patch).unwrap();
        assert_eq!(doc["age"], 31);
    }

    #[test]
    fn test_apply_patch_remove() {
        let mut doc = serde_json::json!({"name": "Alice", "age": 30});
        let patch = vec![PatchOperation::Remove {
            path: "/age".to_string(),
        }];

        apply_patch(&mut doc, &patch).unwrap();
        assert!(doc.get("age").is_none());
    }

    #[test]
    fn test_nested_diff() {
        let old = serde_json::json!({
            "user": {"name": "Alice", "age": 30}
        });
        let new = serde_json::json!({
            "user": {"name": "Alice", "age": 31}
        });

        let patch = compute_diff(&old, &new);
        assert!(patch.iter().any(|op| matches!(op,
            PatchOperation::Replace { path, .. } if path == "/user/age"
        )));
    }

    #[test]
    fn test_json_pointer_escape() {
        let old = serde_json::json!({});
        let new = serde_json::json!({"foo/bar": "value"});

        let patch = compute_diff(&old, &new);
        assert!(patch.iter().any(|op| matches!(op,
            PatchOperation::Add { path, .. } if path == "/foo~1bar"
        )));
    }

    #[test]
    fn test_should_use_patch() {
        let old = serde_json::json!({
            "large_field": "x".repeat(1000),
            "small_field": "y"
        });
        let new = serde_json::json!({
            "large_field": "x".repeat(1000),
            "small_field": "z"
        });

        let patch = compute_diff(&old, &new);
        assert!(utils::should_use_patch(&old, &new, &patch));
    }

    #[test]
    fn test_array_diff_element_change() {
        let old = serde_json::json!({"items": [1, 2, 3]});
        let new = serde_json::json!({"items": [1, 5, 3]});

        let patch = compute_diff(&old, &new);

        // Should have a replace operation for index 1
        assert!(patch.iter().any(|op| matches!(op,
            PatchOperation::Replace { path, value } if path == "/items/1" && value == &serde_json::json!(5)
        )));
    }

    #[test]
    fn test_array_diff_add_element() {
        let old = serde_json::json!({"items": [1, 2]});
        let new = serde_json::json!({"items": [1, 2, 3]});

        let patch = compute_diff(&old, &new);

        // Should have an add operation for the new element
        assert!(patch.iter().any(|op| matches!(op,
            PatchOperation::Add { path, value } if path == "/items/-" && value == &serde_json::json!(3)
        )));
    }

    #[test]
    fn test_array_diff_remove_element() {
        let old = serde_json::json!({"items": [1, 2, 3]});
        let new = serde_json::json!({"items": [1, 2]});

        let patch = compute_diff(&old, &new);

        // Should have a remove operation for index 2
        assert!(patch.iter().any(|op| matches!(op,
            PatchOperation::Remove { path } if path == "/items/2"
        )));
    }

    #[test]
    fn test_array_diff_large_change_replaces() {
        // When array size changes dramatically (>50%), just replace the whole array
        let old = serde_json::json!({"items": [1, 2]});
        let new = serde_json::json!({"items": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]});

        let patch = compute_diff(&old, &new);

        // Should replace the entire array since the size change is significant
        assert!(patch.iter().any(|op| matches!(op,
            PatchOperation::Replace { path, .. } if path == "/items"
        )));
    }

    #[test]
    fn test_array_diff_nested_objects() {
        let old = serde_json::json!({
            "users": [
                {"name": "Alice", "age": 30},
                {"name": "Bob", "age": 25}
            ]
        });
        let new = serde_json::json!({
            "users": [
                {"name": "Alice", "age": 31},
                {"name": "Bob", "age": 25}
            ]
        });

        let patch = compute_diff(&old, &new);

        // Should have a replace operation for the nested age field
        assert!(patch.iter().any(|op| matches!(op,
            PatchOperation::Replace { path, value } if path == "/users/0/age" && value == &serde_json::json!(31)
        )));
    }
}
