use solidb::scripting::{ScriptEngine, Script, ScriptContext, ScriptStats};
use solidb::storage::StorageEngine;
use std::sync::Arc;
use std::collections::HashMap;

#[tokio::test]
async fn verify_lua_security() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(StorageEngine::new(tmp_dir.path()).unwrap());
    let engine = ScriptEngine::new(storage, Arc::new(ScriptStats::default()));

    // Test 1: OS Execute
    let script_os = Script {
        key: "test_os".to_string(),
        name: "Test OS".to_string(),
        methods: vec!["GET".to_string()],
        path: "test".to_string(),
        database: "_system".to_string(),
        collection: None,
        code: r#"
            local handle = os.execute("echo 'pwned' > /tmp/pwned.txt")
            return { status = 200, body = "executed" }
        "#.to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let context = ScriptContext {
        method: "GET".to_string(),
        path: "test".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
    };

    let result_os = engine.execute(&script_os, "_system", &context).await;
    
    // We expect this to FAIL if security is enabled. Currently it succeeds or fails with runtime error if os.execute fails.
    // Ideally, `os` table should be nil.
    
    // Test 2: IO Open
    let script_io = Script {
        key: "test_io".to_string(),
        name: "Test IO".to_string(),
        methods: vec!["GET".to_string()],
        path: "test".to_string(),
        database: "_system".to_string(),
        collection: None,
        code: r#"
            local file = io.open("/tmp/pwned_io.txt", "w")
            file:write("pwned")
            file:close()
            return { status = 200, body = "written" }
        "#.to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };
    
    let result_io = engine.execute(&script_io, "_system", &context).await;

    // Test 3: Check if os table exists
    let script_check = Script {
        key: "check".to_string(),
        name: "Check".to_string(),
        methods: vec!["GET".to_string()],
        path: "check".to_string(),
        database: "_system".to_string(),
        collection: None,
        code: r#"
            if os == nil and io == nil then
                return { status = 200, body = "secure" }
            else
                return { status = 200, body = "insecure" }
            end
        "#.to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let result_check = engine.execute(&script_check, "_system", &context).await.unwrap();
    
    println!("Security Check Result: {:?}", result_check.body);
    
    // START_ASSERTIONS
    // In a secured environment, this should be "secure"
    if let serde_json::Value::String(s) = &result_check.body["body"] {
        assert_eq!(s, "secure", "Environment is not secure! os/io tables still exist.");
    } else {
        panic!("Unexpected response body type: {:?}", result_check.body);
    }

    // Also verify that os.execute fails (it should be nil, so the script effectively crashes or returns error if not handled)
    // Actually the script_os does: `local handle = os.execute(...)`. If os is nil, this raises error "attempt to index global 'os' (a nil value)"
    // So result_os should be an Err.

    match result_os {
        Err(e) => {
            let msg = e.to_string();
            assert!(msg.contains("attempt to index global 'os'") || msg.contains("a nil value"), "Unexpected error message: {}", msg);
        }
        Ok(_) => panic!("os.execute should have failed!"),
    }

    // Similarly for io
    match result_io {
        Err(e) => {
             let msg = e.to_string();
             assert!(msg.contains("attempt to index global 'io'") || msg.contains("a nil value"), "Unexpected error message: {}", msg);
        }
        Ok(_) => panic!("io.open should have failed!"),
    }
}
