//! Conflict Resolution Strategies for Offline Sync
//!
//! When concurrent modifications occur (version vectors are concurrent),
//! a conflict resolution strategy determines how to merge or select the winner.

use crate::sync::version_vector::{ConflictInfo, VectorComparison, VersionVector};
use mlua::{Lua, Result as LuaResult, Value as LuaValue};
use serde_json::Value;
use std::sync::Arc;

/// Trait for conflict resolution strategies
#[async_trait::async_trait]
pub trait ConflictResolver: Send + Sync {
    /// Resolve a conflict between two versions of a document
    ///
    /// Returns the winning document and optionally a merged version
    async fn resolve(&self, conflict: &ConflictInfo) -> ConflictResolution;

    /// Get the name of this resolver
    fn name(&self) -> &'static str;
}

/// Result of conflict resolution
#[derive(Debug, Clone)]
pub enum ConflictResolution {
    /// Use the local version
    LocalWins,
    /// Use the remote version
    RemoteWins,
    /// Use a custom merged version
    Merged(Value),
    /// Keep both as conflicting (requires manual resolution)
    KeepBoth { local: Value, remote: Value },
}

/// Predefined conflict resolution strategies
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolutionStrategy {
    /// Last-write-wins based on HLC timestamp (default)
    LastWriteWins,
    /// Use the version with the higher lexicographical node ID (deterministic)
    Deterministic,
    /// Automatic merge for CRDT-compatible types
    AutomaticMerge,
    /// Keep both versions and require manual resolution
    Manual,
    /// Custom Lua script resolver
    CustomScript,
}

impl ConflictResolutionStrategy {
    /// Create a resolver for this strategy
    pub fn create_resolver(&self) -> Arc<dyn ConflictResolver> {
        match self {
            ConflictResolutionStrategy::LastWriteWins => Arc::new(LastWriteWinsResolver),
            ConflictResolutionStrategy::Deterministic => Arc::new(DeterministicResolver),
            ConflictResolutionStrategy::AutomaticMerge => Arc::new(AutomaticMergeResolver),
            ConflictResolutionStrategy::Manual => Arc::new(ManualResolver),
            ConflictResolutionStrategy::CustomScript => {
                // TODO: Implement Lua script resolver
                Arc::new(ManualResolver)
            }
        }
    }
}

/// Last-Write-Wins resolver using HLC timestamps
pub struct LastWriteWinsResolver;

#[async_trait::async_trait]
impl ConflictResolver for LastWriteWinsResolver {
    async fn resolve(&self, conflict: &ConflictInfo) -> ConflictResolution {
        let local_ts = conflict.local_vector.hlc_timestamp();
        let remote_ts = conflict.remote_vector.hlc_timestamp();

        if local_ts > remote_ts {
            ConflictResolution::LocalWins
        } else if remote_ts > local_ts {
            ConflictResolution::RemoteWins
        } else {
            // Timestamps equal - use counter as tiebreaker
            let local_cnt = conflict.local_vector.hlc_counter();
            let remote_cnt = conflict.remote_vector.hlc_counter();

            if local_cnt >= remote_cnt {
                ConflictResolution::LocalWins
            } else {
                ConflictResolution::RemoteWins
            }
        }
    }

    fn name(&self) -> &'static str {
        "last_write_wins"
    }
}

/// Deterministic resolver using node ID comparison
/// Always produces the same winner given the same conflict
pub struct DeterministicResolver;

#[async_trait::async_trait]
impl ConflictResolver for DeterministicResolver {
    async fn resolve(&self, conflict: &ConflictInfo) -> ConflictResolution {
        // Get the node that made the most recent change in each vector
        let local_node = conflict
            .local_vector
            .nodes()
            .max()
            .map(|s| s.as_str())
            .unwrap_or("");
        let remote_node = conflict
            .remote_vector
            .nodes()
            .max()
            .map(|s| s.as_str())
            .unwrap_or("");

        if local_node >= remote_node {
            ConflictResolution::LocalWins
        } else {
            ConflictResolution::RemoteWins
        }
    }

    fn name(&self) -> &'static str {
        "deterministic"
    }
}

/// Automatic merge resolver for CRDT-compatible documents
pub struct AutomaticMergeResolver;

#[async_trait::async_trait]
impl ConflictResolver for AutomaticMergeResolver {
    async fn resolve(&self, conflict: &ConflictInfo) -> ConflictResolution {
        // Try to merge CRDT fields automatically
        if let (Some(local), Some(remote)) = (&conflict.local_data, &conflict.remote_data) {
            if let Some(merged) = attempt_crdt_merge(local, remote) {
                return ConflictResolution::Merged(merged);
            }
        }

        // Fall back to last-write-wins if automatic merge not possible
        let resolver = LastWriteWinsResolver;
        resolver.resolve(conflict).await
    }

    fn name(&self) -> &'static str {
        "automatic_merge"
    }
}

/// Manual resolver that keeps both versions
pub struct ManualResolver;

#[async_trait::async_trait]
impl ConflictResolver for ManualResolver {
    async fn resolve(&self, conflict: &ConflictInfo) -> ConflictResolution {
        ConflictResolution::KeepBoth {
            local: conflict.local_data.clone().unwrap_or(Value::Null),
            remote: conflict.remote_data.clone().unwrap_or(Value::Null),
        }
    }

    fn name(&self) -> &'static str {
        "manual"
    }
}

/// Custom Lua script-based conflict resolver
///
/// The Lua script should return one of:
/// - "local" - use the local version
/// - "remote" - use the remote version
/// - A table - use as the merged document
///
/// Available globals in the script:
/// - local_doc: The local document (server version)
/// - remote_doc: The remote document (client version)
/// - key: The document key
/// - collection: The collection name
pub struct CustomScriptResolver {
    script: String,
}

impl CustomScriptResolver {
    /// Create a new custom script resolver
    pub fn new(script: impl Into<String>) -> Self {
        Self {
            script: script.into(),
        }
    }

    /// Convert a serde_json::Value to a Lua value
    fn json_to_lua(lua: &Lua, value: &Value) -> LuaResult<LuaValue> {
        match value {
            Value::Null => Ok(LuaValue::Nil),
            Value::Bool(b) => Ok(LuaValue::Boolean(*b)),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(LuaValue::Integer(i))
                } else if let Some(f) = n.as_f64() {
                    Ok(LuaValue::Number(f))
                } else {
                    Ok(LuaValue::Nil)
                }
            }
            Value::String(s) => lua.create_string(s).map(LuaValue::String),
            Value::Array(arr) => {
                let table = lua.create_table()?;
                for (i, v) in arr.iter().enumerate() {
                    table.set(i + 1, Self::json_to_lua(lua, v)?)?;
                }
                Ok(LuaValue::Table(table))
            }
            Value::Object(obj) => {
                let table = lua.create_table()?;
                for (k, v) in obj {
                    table.set(k.as_str(), Self::json_to_lua(lua, v)?)?;
                }
                Ok(LuaValue::Table(table))
            }
        }
    }

    /// Convert a Lua value to serde_json::Value
    fn lua_to_json(value: LuaValue) -> Option<Value> {
        match value {
            LuaValue::Nil => Some(Value::Null),
            LuaValue::Boolean(b) => Some(Value::Bool(b)),
            LuaValue::Integer(i) => Some(Value::Number(i.into())),
            LuaValue::Number(f) => serde_json::Number::from_f64(f).map(Value::Number),
            LuaValue::String(s) => Some(Value::String(s.to_string_lossy().to_string())),
            LuaValue::Table(table) => {
                // First, check if all keys are integers starting from 1
                let mut has_string_keys = false;
                let mut max_int_key = 0i64;
                let mut int_key_count = 0usize;

                // Iterate with general Value keys to detect the key types
                for (k, _) in table.clone().pairs::<LuaValue, LuaValue>().flatten() {
                    match k {
                        LuaValue::Integer(i) => {
                            int_key_count += 1;
                            if i > max_int_key {
                                max_int_key = i;
                            }
                        }
                        LuaValue::String(_) => {
                            has_string_keys = true;
                        }
                        _ => {}
                    }
                }

                // If it has string keys, treat as object
                // If it only has sequential integer keys from 1 to n, treat as array
                let is_array =
                    !has_string_keys && int_key_count > 0 && max_int_key == int_key_count as i64;

                if is_array {
                    let mut arr: Vec<(i64, Value)> = Vec::new();
                    for (k, v) in table.pairs::<i64, LuaValue>().flatten() {
                        if let Some(val) = Self::lua_to_json(v) {
                            arr.push((k, val));
                        }
                    }
                    // Sort by key and extract values
                    arr.sort_by_key(|(k, _)| *k);
                    Some(Value::Array(arr.into_iter().map(|(_, v)| v).collect()))
                } else {
                    let mut obj = serde_json::Map::new();
                    for (k, v) in table.pairs::<LuaValue, LuaValue>().flatten() {
                        let key = match k {
                            LuaValue::String(s) => s.to_string_lossy().to_string(),
                            LuaValue::Integer(i) => i.to_string(),
                            LuaValue::Number(n) => n.to_string(),
                            _ => continue,
                        };
                        if let Some(val) = Self::lua_to_json(v) {
                            obj.insert(key, val);
                        }
                    }
                    Some(Value::Object(obj))
                }
            }
            _ => None,
        }
    }
}

#[async_trait::async_trait]
impl ConflictResolver for CustomScriptResolver {
    async fn resolve(&self, conflict: &ConflictInfo) -> ConflictResolution {
        // Create a new Lua VM for each resolution (safer isolation)
        let lua = Lua::new();

        // Set up globals
        let globals = lua.globals();

        // Expose conflict data to Lua
        if let Ok(local_val) =
            Self::json_to_lua(&lua, &conflict.local_data.clone().unwrap_or(Value::Null))
        {
            let _ = globals.set("local_doc", local_val);
        }

        if let Ok(remote_val) =
            Self::json_to_lua(&lua, &conflict.remote_data.clone().unwrap_or(Value::Null))
        {
            let _ = globals.set("remote_doc", remote_val);
        }

        let _ = globals.set("key", conflict.document_key.clone());
        let _ = globals.set("collection", conflict.collection.clone());

        // Execute the script
        match lua.load(&self.script).eval::<LuaValue>() {
            Ok(result) => match &result {
                LuaValue::String(s) => {
                    let s_str = s.to_string_lossy();
                    match s_str.as_ref() {
                        "local" => ConflictResolution::LocalWins,
                        "remote" => ConflictResolution::RemoteWins,
                        _ => ConflictResolution::LocalWins, // Default
                    }
                }
                LuaValue::Table(_) => {
                    // Script returned a merged table
                    if let Some(merged) = Self::lua_to_json(result) {
                        ConflictResolution::Merged(merged)
                    } else {
                        ConflictResolution::LocalWins
                    }
                }
                _ => ConflictResolution::LocalWins,
            },
            Err(e) => {
                tracing::error!("Custom conflict resolver script error: {}", e);
                ConflictResolution::LocalWins
            }
        }
    }

    fn name(&self) -> &'static str {
        "custom_script"
    }
}

/// Create a custom script resolver with the given Lua script
pub fn create_custom_resolver(script: impl Into<String>) -> Arc<dyn ConflictResolver> {
    Arc::new(CustomScriptResolver::new(script))
}

/// Attempt to automatically merge two documents using CRDT rules
/// Returns None if automatic merge is not possible
fn attempt_crdt_merge(local: &Value, remote: &Value) -> Option<Value> {
    match (local, remote) {
        // Merge objects field by field
        (Value::Object(local_map), Value::Object(remote_map)) => {
            let mut merged = serde_json::Map::new();

            // Add all fields from local
            for (key, value) in local_map {
                merged.insert(key.clone(), value.clone());
            }

            // Merge or overwrite with remote fields
            for (key, remote_value) in remote_map {
                if let Some(local_value) = local_map.get(key) {
                    // Field exists in both - try to merge
                    if let Some(merged_value) = attempt_crdt_merge(local_value, remote_value) {
                        merged.insert(key.clone(), merged_value);
                    } else {
                        // Can't merge - use remote (LWW)
                        merged.insert(key.clone(), remote_value.clone());
                    }
                } else {
                    // Field only in remote - add it
                    merged.insert(key.clone(), remote_value.clone());
                }
            }

            Some(Value::Object(merged))
        }
        // For primitive types, use remote (LWW behavior for non-CRDT fields)
        _ => None,
    }
}

/// Detect conflicts between two version vectors
pub fn detect_conflict(
    local_vector: &VersionVector,
    remote_vector: &VersionVector,
) -> Option<VectorComparison> {
    let comparison = local_vector.compare(remote_vector);

    match comparison {
        VectorComparison::Concurrent => Some(comparison),
        _ => None,
    }
}

/// Convenience function to resolve a conflict using a strategy
pub async fn resolve_conflict(
    strategy: ConflictResolutionStrategy,
    conflict: &ConflictInfo,
) -> ConflictResolution {
    let resolver = strategy.create_resolver();
    resolver.resolve(conflict).await
}

/// Apply conflict resolution to get the final document value
pub fn apply_resolution(
    resolution: &ConflictResolution,
    local: Option<&Value>,
    remote: Option<&Value>,
) -> Option<Value> {
    match resolution {
        ConflictResolution::LocalWins => local.cloned(),
        ConflictResolution::RemoteWins => remote.cloned(),
        ConflictResolution::Merged(merged) => Some(merged.clone()),
        ConflictResolution::KeepBoth { local, remote } => {
            // Create a conflict marker document
            let mut conflict_doc = serde_json::Map::new();
            conflict_doc.insert("_conflict".to_string(), Value::Bool(true));
            conflict_doc.insert("_local".to_string(), local.clone());
            conflict_doc.insert("_remote".to_string(), remote.clone());
            Some(Value::Object(conflict_doc))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::version_vector::VersionVector;
    use serde_json::json;

    fn create_conflict_info(local_ts: u64, remote_ts: u64) -> ConflictInfo {
        let mut local_vector = VersionVector::new();
        local_vector.set_hlc(local_ts, 0);
        local_vector.increment("node-1");

        let mut remote_vector = VersionVector::new();
        remote_vector.set_hlc(remote_ts, 0);
        remote_vector.increment("node-2");

        ConflictInfo {
            document_key: "test-1".to_string(),
            collection: "test".to_string(),
            local_vector,
            remote_vector,
            local_data: Some(json!({"field": "local"})),
            remote_data: Some(json!({"field": "remote"})),
            detected_at: 0,
        }
    }

    #[tokio::test]
    async fn test_last_write_wins_local() {
        let resolver = LastWriteWinsResolver;
        let conflict = create_conflict_info(100, 50); // Local is newer

        let result = resolver.resolve(&conflict).await;
        assert!(matches!(result, ConflictResolution::LocalWins));
    }

    #[tokio::test]
    async fn test_last_write_wins_remote() {
        let resolver = LastWriteWinsResolver;
        let conflict = create_conflict_info(50, 100); // Remote is newer

        let result = resolver.resolve(&conflict).await;
        assert!(matches!(result, ConflictResolution::RemoteWins));
    }

    #[tokio::test]
    async fn test_deterministic_resolution() {
        let resolver = DeterministicResolver;
        let conflict = create_conflict_info(50, 50); // Same timestamp

        let result = resolver.resolve(&conflict).await;
        // node-2 > node-1 (lexicographically '2' > '1'), so remote wins
        assert!(matches!(result, ConflictResolution::RemoteWins));
    }

    #[tokio::test]
    async fn test_manual_resolution() {
        let resolver = ManualResolver;
        let conflict = create_conflict_info(50, 50);

        let result = resolver.resolve(&conflict).await;
        assert!(matches!(result, ConflictResolution::KeepBoth { .. }));
    }

    #[test]
    fn test_crdt_merge_objects() {
        let local = json!({
            "name": "Alice",
            "age": 30,
            "crdt_counter": { "_type": "GCounter", "value": 5 }
        });
        let remote = json!({
            "name": "Alice",
            "age": 31,
            "email": "alice@example.com"
        });

        let merged = attempt_crdt_merge(&local, &remote);
        assert!(merged.is_some());

        let merged = merged.unwrap();
        assert_eq!(merged["name"], "Alice");
        assert_eq!(merged["age"], 31); // Remote wins for age
        assert_eq!(merged["email"], "alice@example.com");
    }

    #[tokio::test]
    async fn test_custom_script_returns_local() {
        let script = r#"
            return "local"
        "#;
        let resolver = CustomScriptResolver::new(script);
        let conflict = create_conflict_info(50, 100);

        let result = resolver.resolve(&conflict).await;
        assert!(matches!(result, ConflictResolution::LocalWins));
    }

    #[tokio::test]
    async fn test_custom_script_returns_remote() {
        let script = r#"
            return "remote"
        "#;
        let resolver = CustomScriptResolver::new(script);
        let conflict = create_conflict_info(50, 100);

        let result = resolver.resolve(&conflict).await;
        assert!(matches!(result, ConflictResolution::RemoteWins));
    }

    #[tokio::test]
    async fn test_custom_script_returns_merged() {
        let script = r#"
            -- Merge local and remote, taking name from local and field from remote
            local result = {}
            if local_doc and local_doc.name then
                result.name = local_doc.name
            else
                result.name = "unknown"
            end
            if remote_doc and remote_doc.field then
                result.field = remote_doc.field
            end
            return result
        "#;

        // Create conflict with named fields
        let mut local_vector = VersionVector::new();
        local_vector.set_hlc(50, 0);
        local_vector.increment("node-1");

        let mut remote_vector = VersionVector::new();
        remote_vector.set_hlc(100, 0);
        remote_vector.increment("node-2");

        let conflict = ConflictInfo {
            document_key: "test-1".to_string(),
            collection: "test".to_string(),
            local_vector,
            remote_vector,
            local_data: Some(json!({"name": "Alice", "field": "local_value"})),
            remote_data: Some(json!({"name": "Bob", "field": "remote_value"})),
            detected_at: 0,
        };

        let resolver = CustomScriptResolver::new(script);
        let result = resolver.resolve(&conflict).await;

        match result {
            ConflictResolution::Merged(merged) => {
                assert_eq!(merged["name"], "Alice");
                assert_eq!(merged["field"], "remote_value");
            }
            _ => panic!("Expected Merged resolution"),
        }
    }

    #[tokio::test]
    async fn test_custom_script_has_access_to_key() {
        let script = r#"
            -- Return remote if key starts with "important"
            if string.sub(key, 1, 9) == "important" then
                return "remote"
            else
                return "local"
            end
        "#;

        let mut local_vector = VersionVector::new();
        local_vector.set_hlc(50, 0);
        local_vector.increment("node-1");

        let mut remote_vector = VersionVector::new();
        remote_vector.set_hlc(100, 0);
        remote_vector.increment("node-2");

        let conflict = ConflictInfo {
            document_key: "important-doc-1".to_string(),
            collection: "test".to_string(),
            local_vector,
            remote_vector,
            local_data: Some(json!({"field": "local"})),
            remote_data: Some(json!({"field": "remote"})),
            detected_at: 0,
        };

        let resolver = CustomScriptResolver::new(script);
        let result = resolver.resolve(&conflict).await;
        assert!(matches!(result, ConflictResolution::RemoteWins));
    }

    #[tokio::test]
    async fn test_custom_script_error_defaults_to_local() {
        let script = r#"
            -- Invalid Lua that will error
            this_function_does_not_exist()
        "#;
        let resolver = CustomScriptResolver::new(script);
        let conflict = create_conflict_info(50, 100);

        // Should default to LocalWins on error
        let result = resolver.resolve(&conflict).await;
        assert!(matches!(result, ConflictResolution::LocalWins));
    }
}
