#[tokio::test]
async fn verify_lua_time_functions() {
    use solidb::scripting::{ScriptEngine, ScriptContext, Script};
    use solidb::storage::StorageEngine;
    use std::sync::Arc;
    use std::collections::HashMap;

    // Helper to run a script and check for errors
    async fn verify_lua_script(code: &str) {
        use solidb::scripting::ScriptStats;
        let engine = ScriptEngine::new(Arc::new(StorageEngine::new("test_db".to_string()).unwrap()), Arc::new(ScriptStats::default()));
        let context = ScriptContext {
            method: "GET".to_string(),
            path: "/test".to_string(),
            query_params: HashMap::new(),
            params: HashMap::new(),
            headers: HashMap::new(),
            body: None,
            is_websocket: false,
        };

        // Wrap code in a function to allow early returns with error messages
        let script_code = format!(r#"
            return (function()
                {}
                return "OK"
            end)()
        "#, code);

        let script = Script {
            key: "test".to_string(),
            name: "test".to_string(),
            methods: vec!["GET".to_string()],
            path: "test".to_string(),
            database: "test_db".to_string(),
            collection: None,
            code: script_code,
            description: None,
            created_at: "0".to_string(),
            updated_at: "0".to_string(),
        };

        let result = engine.execute(&script, "test_db", &context).await;

        match result {
            Ok(res) => {
                let json = res.body;
                if let Some(s) = json.as_str() {
                    if s != "OK" {
                        panic!("Script check failed: {}", s);
                    }
                } else {
                    panic!("Script did not return a string result: {:?}", json);
                }
            },
            Err(e) => panic!("Script failed to execute: {}", e),
        }
    }

    verify_lua_script(r#"
        local now = time.now()
        if type(now) ~= "number" then return "time.now() returned " .. type(now) end
        if now < 1700000000 then return "time.now() value weird: " .. now end
        
        local now_ms = time.now_ms()
        if type(now_ms) ~= "number" then return "time.now_ms() returned " .. type(now_ms) end
        if now_ms < 1700000000000 then return "time.now_ms() value weird: " .. now_ms end
        
        local iso = time.iso()
        if type(iso) ~= "string" then return "time.iso() returned " .. type(iso) end
        if string.sub(iso, 1, 2) ~= "20" then return "time.iso() weird: " .. iso end

        -- Asynchronous sleep test
        local t1 = time.now_ms()
        time.sleep(50)
        local t2 = time.now_ms()
        local diff = t2 - t1
        if diff < 40 then return "time.sleep(50) took only " .. diff .. "ms" end
        
        -- Verify format and parse
        local t = 1700000000 -- 2023-11-14 22:13:20 UTC
        local fmt = time.format(t, "%Y-%m-%d")
        if fmt ~= "2023-11-14" then return "time.format error: " .. fmt end
        
        -- parse usually returns UTC float
        local parsed = time.parse("2023-11-14T22:13:20Z")
        -- floating point comparison
        if math.abs(parsed - 1700000000) > 0.001 then return "time.parse error: " .. parsed end
        
        -- Verify add and subtract
        local start = 1000
        local t_add = time.add(start, 1, "h")
        if t_add ~= 1000 + 3600 then return "time.add error: " .. t_add end
        
        local t_sub = time.subtract(start, 1, "m")
        if t_sub ~= 1000 - 60 then return "time.subtract error: " .. t_sub end
        
        local t_add_d = time.add(start, 2, "d")
        if t_add_d ~= 1000 + 2 * 86400 then return "time.add days error" end
    "#).await;
}
