//! Integration tests for Lua script optimization (pool, cache, index)
//!
//! These tests verify that the performance optimization components
//! (LuaPool, ScriptCache, ScriptIndex) work correctly when integrated
//! with the ScriptEngine.

use serde_json::json;
use solidb::scripting::engine::{LuaPool, ScriptCache, ScriptIndex};
use solidb::scripting::{Script, ScriptContext, ScriptEngine, ScriptStats, ScriptUser};
use solidb::storage::StorageEngine;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

// ============================================================================
// Test Helpers
// ============================================================================

fn create_test_env() -> (Arc<StorageEngine>, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = Arc::new(
        StorageEngine::new(tmp_dir.path().to_str().unwrap())
            .expect("Failed to create storage engine"),
    );
    engine.create_database("testdb".to_string()).unwrap();
    (engine, tmp_dir)
}

fn create_script(key: &str, code: &str) -> Script {
    Script {
        key: key.to_string(),
        name: format!("Test Script {}", key),
        methods: vec!["GET".to_string()],
        path: "/test".to_string(),
        database: "testdb".to_string(),
        service: "default".to_string(),
        collection: None,
        code: code.to_string(),
        description: None,
        created_at: "2024-01-01".to_string(),
        updated_at: "2024-01-01".to_string(),
    }
}

fn create_default_context() -> ScriptContext {
    ScriptContext {
        method: "GET".to_string(),
        path: "/test".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
        user: ScriptUser::anonymous(),
    }
}

// ============================================================================
// Pool Integration Tests
// ============================================================================

#[tokio::test]
async fn test_engine_uses_pool() {
    let (storage, _tmp) = create_test_env();
    let pool = Arc::new(LuaPool::new(2));
    let stats = Arc::new(ScriptStats::default());

    let engine = ScriptEngine::new(storage, stats).with_lua_pool(pool.clone());

    let script = create_script("s1", "return 42");
    let ctx = create_default_context();

    // Execute twice
    let result1 = engine.execute(&script, "testdb", &ctx).await.unwrap();
    let result2 = engine.execute(&script, "testdb", &ctx).await.unwrap();

    assert_eq!(result1.body, json!(42));
    assert_eq!(result2.body, json!(42));

    // Verify pool was used
    let pool_stats = pool.stats();
    assert_eq!(pool_stats.total_uses, 2, "Pool should have been used twice");
    assert_eq!(pool_stats.in_use, 0, "All states should be released");
}

#[tokio::test]
async fn test_engine_without_pool() {
    let (storage, _tmp) = create_test_env();
    let stats = Arc::new(ScriptStats::default());

    // Engine without pool
    let engine = ScriptEngine::new(storage, stats);

    let script = create_script("s1", "return { value = 'no pool' }");
    let ctx = create_default_context();

    let result = engine.execute(&script, "testdb", &ctx).await.unwrap();
    assert_eq!(result.body["value"], "no pool");
}

#[tokio::test]
async fn test_pool_isolation_between_requests() {
    let (storage, _tmp) = create_test_env();
    let pool = Arc::new(LuaPool::new(1)); // Single state to ensure same state is reused
    let stats = Arc::new(ScriptStats::default());

    let engine = ScriptEngine::new(storage, stats).with_lua_pool(pool.clone());

    // First request sets a global (which should be cleaned up)
    let script1 = create_script("s1", "test_var = 'should_be_cleared'; return test_var");
    let ctx = create_default_context();

    let result1 = engine.execute(&script1, "testdb", &ctx).await.unwrap();
    assert_eq!(result1.body, json!("should_be_cleared"));

    // Second request should not see the global
    let script2 = create_script("s2", "return test_var or 'nil'");
    let result2 = engine.execute(&script2, "testdb", &ctx).await.unwrap();
    assert_eq!(
        result2.body,
        json!("nil"),
        "Global from previous request should be cleared"
    );
}

// ============================================================================
// Cache Integration Tests
// ============================================================================

#[tokio::test]
async fn test_engine_uses_cache() {
    let (storage, _tmp) = create_test_env();
    let pool = Arc::new(LuaPool::new(2));
    let cache = Arc::new(ScriptCache::new(10));
    let stats = Arc::new(ScriptStats::default());

    let engine = ScriptEngine::new(storage, stats)
        .with_lua_pool(pool.clone())
        .with_script_cache(cache.clone());

    let script = create_script("cached_script", "return { result = 'cached' }");
    let ctx = create_default_context();

    // First execution - cache miss
    let result1 = engine.execute(&script, "testdb", &ctx).await.unwrap();
    assert_eq!(result1.body["result"], "cached");

    let stats1 = cache.stats();
    assert_eq!(stats1.misses, 1, "First execution should be a cache miss");
    assert_eq!(stats1.entries, 1, "Bytecode should be cached");

    // Second execution - cache hit
    let result2 = engine.execute(&script, "testdb", &ctx).await.unwrap();
    assert_eq!(result2.body["result"], "cached");

    let stats2 = cache.stats();
    assert_eq!(stats2.hits, 1, "Second execution should be a cache hit");
    assert_eq!(stats2.misses, 1, "Misses should not increase");
}

#[tokio::test]
async fn test_cache_invalidation_on_code_change() {
    let (storage, _tmp) = create_test_env();
    let pool = Arc::new(LuaPool::new(2));
    let cache = Arc::new(ScriptCache::new(10));
    let stats = Arc::new(ScriptStats::default());

    let engine = ScriptEngine::new(storage, stats)
        .with_lua_pool(pool.clone())
        .with_script_cache(cache.clone());

    let ctx = create_default_context();

    // Execute with original code
    let script_v1 = create_script("versioned", "return 'v1'");
    let result1 = engine.execute(&script_v1, "testdb", &ctx).await.unwrap();
    assert_eq!(result1.body, json!("v1"));

    // Execute with updated code (same key, different code)
    let script_v2 = Script {
        code: "return 'v2'".to_string(),
        ..script_v1.clone()
    };
    let result2 = engine.execute(&script_v2, "testdb", &ctx).await.unwrap();
    assert_eq!(result2.body, json!("v2"), "Should execute updated code");

    // Both versions should create separate cache entries (since hash differs)
    let cache_stats = cache.stats();
    assert_eq!(
        cache_stats.entries, 2,
        "Different code versions should be cached separately"
    );
}

#[tokio::test]
async fn test_multiple_scripts_cached() {
    let (storage, _tmp) = create_test_env();
    let pool = Arc::new(LuaPool::new(2));
    let cache = Arc::new(ScriptCache::new(10));
    let stats = Arc::new(ScriptStats::default());

    let engine = ScriptEngine::new(storage, stats)
        .with_lua_pool(pool.clone())
        .with_script_cache(cache.clone());

    let ctx = create_default_context();

    // Execute multiple different scripts
    for i in 0..5 {
        let script = create_script(&format!("script_{}", i), &format!("return {}", i));
        let result = engine.execute(&script, "testdb", &ctx).await.unwrap();
        assert_eq!(result.body, json!(i));
    }

    // All should be cached
    let cache_stats = cache.stats();
    assert_eq!(cache_stats.entries, 5, "All 5 scripts should be cached");
    assert_eq!(cache_stats.misses, 5, "All should have been initial misses");

    // Re-execute all - should be cache hits
    for i in 0..5 {
        let script = create_script(&format!("script_{}", i), &format!("return {}", i));
        engine.execute(&script, "testdb", &ctx).await.unwrap();
    }

    let cache_stats = cache.stats();
    assert_eq!(
        cache_stats.hits, 5,
        "All re-executions should be cache hits"
    );
}

// ============================================================================
// ScriptIndex Integration Tests
// ============================================================================

#[test]
fn test_script_index_basic_operations() {
    let index = ScriptIndex::new();

    // Insert scripts
    let script1 = Script {
        key: "hello".to_string(),
        name: "Hello".to_string(),
        methods: vec!["GET".to_string()],
        path: "hello".to_string(),
        database: "testdb".to_string(),
        service: "default".to_string(),
        collection: None,
        code: "return 'hello'".to_string(),
        description: None,
        created_at: "2024-01-01".to_string(),
        updated_at: "2024-01-01".to_string(),
    };

    index.insert(script1);

    // Find should work
    let found = index.find("testdb", "default", "hello", "GET");
    assert!(found.is_some());
    assert_eq!(found.unwrap().key, "hello");

    // Remove should work
    index.remove("hello", "testdb", "default");
    assert!(index.find("testdb", "default", "hello", "GET").is_none());
}

#[test]
fn test_script_index_parameter_paths() {
    let index = ScriptIndex::new();

    let script = Script {
        key: "user_detail".to_string(),
        name: "User Detail".to_string(),
        methods: vec!["GET".to_string()],
        path: "users/:id".to_string(),
        database: "testdb".to_string(),
        service: "default".to_string(),
        collection: None,
        code: "return {}".to_string(),
        description: None,
        created_at: "2024-01-01".to_string(),
        updated_at: "2024-01-01".to_string(),
    };

    index.insert(script);

    // Should match any ID
    assert!(index
        .find("testdb", "default", "users/123", "GET")
        .is_some());
    assert!(index
        .find("testdb", "default", "users/abc", "GET")
        .is_some());
    assert!(index
        .find("testdb", "default", "users/uuid-like-string", "GET")
        .is_some());

    // Should not match different paths
    assert!(index.find("testdb", "default", "users", "GET").is_none());
    assert!(index
        .find("testdb", "default", "users/123/posts", "GET")
        .is_none());
}

#[test]
fn test_script_index_multiple_methods() {
    let index = ScriptIndex::new();

    let script = Script {
        key: "crud".to_string(),
        name: "CRUD".to_string(),
        methods: vec![
            "GET".to_string(),
            "POST".to_string(),
            "PUT".to_string(),
            "DELETE".to_string(),
        ],
        path: "resource".to_string(),
        database: "testdb".to_string(),
        service: "default".to_string(),
        collection: None,
        code: "return {}".to_string(),
        description: None,
        created_at: "2024-01-01".to_string(),
        updated_at: "2024-01-01".to_string(),
    };

    index.insert(script);

    // All methods should work
    assert!(index.find("testdb", "default", "resource", "GET").is_some());
    assert!(index
        .find("testdb", "default", "resource", "POST")
        .is_some());
    assert!(index.find("testdb", "default", "resource", "PUT").is_some());
    assert!(index
        .find("testdb", "default", "resource", "DELETE")
        .is_some());

    // PATCH should not match
    assert!(index
        .find("testdb", "default", "resource", "PATCH")
        .is_none());
}

#[test]
fn test_script_index_database_isolation() {
    let index = ScriptIndex::new();

    // Same path in different databases
    let script1 = Script {
        key: "api1".to_string(),
        name: "API1".to_string(),
        methods: vec!["GET".to_string()],
        path: "api".to_string(),
        database: "db1".to_string(),
        service: "default".to_string(),
        collection: None,
        code: "return 'db1'".to_string(),
        description: None,
        created_at: "2024-01-01".to_string(),
        updated_at: "2024-01-01".to_string(),
    };

    let script2 = Script {
        key: "api2".to_string(),
        name: "API2".to_string(),
        methods: vec!["GET".to_string()],
        path: "api".to_string(),
        database: "db2".to_string(),
        service: "default".to_string(),
        collection: None,
        code: "return 'db2'".to_string(),
        description: None,
        created_at: "2024-01-01".to_string(),
        updated_at: "2024-01-01".to_string(),
    };

    index.insert(script1);
    index.insert(script2);

    // Each database should return its own script
    let found1 = index.find("db1", "default", "api", "GET").unwrap();
    let found2 = index.find("db2", "default", "api", "GET").unwrap();

    assert_eq!(found1.key, "api1");
    assert_eq!(found2.key, "api2");
}

// ============================================================================
// Combined Integration Tests
// ============================================================================

#[tokio::test]
async fn test_full_optimization_stack() {
    let (storage, _tmp) = create_test_env();
    let pool = Arc::new(LuaPool::new(4));
    let cache = Arc::new(ScriptCache::new(100));
    let stats = Arc::new(ScriptStats::default());

    let engine = ScriptEngine::new(storage.clone(), stats.clone())
        .with_lua_pool(pool.clone())
        .with_script_cache(cache.clone());

    let ctx = create_default_context();

    // Execute many scripts to exercise all optimization paths
    for i in 0..20 {
        let script = create_script(&format!("script_{}", i), &format!("return {{ n = {} }}", i));
        let result = engine.execute(&script, "testdb", &ctx).await.unwrap();
        assert_eq!(result.body["n"], json!(i));
    }

    // Re-execute some scripts (should hit cache)
    for i in 0..10 {
        let script = create_script(&format!("script_{}", i), &format!("return {{ n = {} }}", i));
        engine.execute(&script, "testdb", &ctx).await.unwrap();
    }

    // Verify statistics
    let pool_stats = pool.stats();
    assert_eq!(pool_stats.total_uses, 30, "Pool should have 30 total uses");
    assert_eq!(pool_stats.in_use, 0, "All pool states should be released");

    let cache_stats = cache.stats();
    assert_eq!(
        cache_stats.entries, 20,
        "All 20 unique scripts should be cached"
    );
    assert_eq!(cache_stats.misses, 20, "20 initial cache misses");
    assert_eq!(cache_stats.hits, 10, "10 cache hits from re-execution");
}

#[tokio::test]
async fn test_concurrent_script_execution_with_optimization() {
    let (storage, _tmp) = create_test_env();
    let pool = Arc::new(LuaPool::new(4));
    let cache = Arc::new(ScriptCache::new(100));
    let stats = Arc::new(ScriptStats::default());

    let engine = Arc::new(
        ScriptEngine::new(storage.clone(), stats.clone())
            .with_lua_pool(pool.clone())
            .with_script_cache(cache.clone()),
    );

    let ctx = create_default_context();

    // Spawn multiple concurrent tasks
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let eng = engine.clone();
            let context = ctx.clone();
            tokio::spawn(async move {
                let script =
                    create_script(&format!("concurrent_{}", i), &format!("return {}", i * 10));
                let result = eng.execute(&script, "testdb", &context).await.unwrap();
                result.body.as_i64().unwrap()
            })
        })
        .collect();

    // Collect results
    let mut results: Vec<i64> = vec![];
    for h in handles {
        results.push(h.await.unwrap());
    }

    // Sort and verify all expected values are present
    results.sort();
    let expected: Vec<i64> = (0..8).map(|i| i * 10).collect();
    assert_eq!(results, expected);

    // All states should be released
    let pool_stats = pool.stats();
    assert_eq!(
        pool_stats.in_use, 0,
        "All pool states should be released after concurrent execution"
    );
}

#[tokio::test]
async fn test_script_error_handling_with_optimization() {
    let (storage, _tmp) = create_test_env();
    let pool = Arc::new(LuaPool::new(2));
    let cache = Arc::new(ScriptCache::new(10));
    let stats = Arc::new(ScriptStats::default());

    let engine = ScriptEngine::new(storage, stats)
        .with_lua_pool(pool.clone())
        .with_script_cache(cache.clone());

    let ctx = create_default_context();

    // Execute a script with a runtime error
    let bad_script = create_script("bad", "return nil + 1"); // Runtime error
    let result = engine.execute(&bad_script, "testdb", &ctx).await;
    assert!(result.is_err(), "Script with error should fail");

    // Pool state should still be usable after error
    let good_script = create_script("good", "return 'recovered'");
    let result = engine.execute(&good_script, "testdb", &ctx).await.unwrap();
    assert_eq!(result.body, json!("recovered"));

    // States should be properly released
    let pool_stats = pool.stats();
    assert_eq!(
        pool_stats.in_use, 0,
        "All states should be released even after error"
    );
}
