//! Cross-tenant isolation of pooled Lua states, and the async bindings on
//! the pooled path.
//!
//! Audit C3: a pooled state is reused by every database on the instance, and
//! its shared library tables (`crypto`, `json`, `string`, `solidb`, ...)
//! used to keep whatever fields the previous script wrote. Audit A5: the
//! pooled path now runs on a blocking thread and drives the script with
//! `eval_async`, so async bindings must work there. Audit H6: `solidb.cache`
//! is namespaced per database.

use serde_json::json;
use solidb::scripting::engine::LuaPool;
use solidb::scripting::{Script, ScriptContext, ScriptEngine, ScriptStats, ScriptUser};
use solidb::storage::StorageEngine;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

fn setup() -> (ScriptEngine, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let storage = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).expect("storage"));
    storage.create_database("tenant_a".to_string()).unwrap();
    storage.create_database("tenant_b".to_string()).unwrap();
    // One state, so every request lands on the same one.
    let engine = ScriptEngine::new(storage, Arc::new(ScriptStats::default()))
        .with_lua_pool(Arc::new(LuaPool::new(1)));
    (engine, tmp)
}

fn script(db: &str, key: &str, code: &str) -> Script {
    Script {
        key: key.to_string(),
        name: key.to_string(),
        methods: vec!["GET".to_string()],
        path: key.to_string(),
        database: db.to_string(),
        service: "default".to_string(),
        collection: None,
        code: code.to_string(),
        description: None,
        created_at: "2026-01-01".to_string(),
        updated_at: "2026-01-01".to_string(),
    }
}

fn ctx() -> ScriptContext {
    ScriptContext {
        method: "GET".to_string(),
        path: "/".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
        user: ScriptUser::anonymous(),
    }
}

#[tokio::test]
async fn tenant_mutations_of_shared_tables_do_not_reach_the_next_tenant() {
    let (engine, _tmp) = setup();

    let attack = script(
        "tenant_a",
        "attack",
        r#"
        local refused = 0
        for _, f in ipairs({
            function() crypto.verify_password = function() return true end end,
            function() json.encode = function() return "evil" end end,
            function() string.__loot = solidb end,
            function() table.insert = function() end end,
        }) do
            if not pcall(f) then refused = refused + 1 end
        end
        rawset(crypto, "verify_password", function() return true end)
        rawset(string, "__loot", "stolen")
        collectgarbage("stop")
        return refused
        "#,
    );
    let r = engine.execute(&attack, "tenant_a", &ctx()).await.unwrap();
    assert_eq!(r.body, json!(4), "every plain write must be refused");

    let victim = script(
        "tenant_b",
        "victim",
        r#"
        local t = {}
        table.insert(t, "x")
        return {
            loot = string.__loot == nil,
            verify_is_native = rawget(crypto, "verify_password") == nil,
            json_ok = json.decode(json.encode({a = 1})).a == 1,
            insert_ok = t[1] == "x",
            upper = ("abc"):upper(),
            gc_running = collectgarbage("isrunning"),
        }
        "#,
    );
    let r = engine.execute(&victim, "tenant_b", &ctx()).await.unwrap();
    assert_eq!(
        r.body,
        json!({
            "loot": true,
            "verify_is_native": true,
            "json_ok": true,
            "insert_ok": true,
            "upper": "ABC",
            "gc_running": true,
        })
    );
}

#[tokio::test]
async fn async_bindings_work_on_the_pooled_path() {
    let (engine, _tmp) = setup();
    let s = script(
        "tenant_a",
        "async",
        r#"
        time.sleep(5)
        local h = crypto.hash_password("pw")
        return crypto.verify_password(h, "pw")
        "#,
    );
    let r = engine.execute(&s, "tenant_a", &ctx()).await.unwrap();
    assert_eq!(r.body, json!(true));
}

#[tokio::test]
async fn solidb_cache_is_per_database() {
    let (engine, _tmp) = setup();
    let put = script(
        "tenant_a",
        "put",
        r#"solidb.cache("iso_session", "a-secret", 60); return true"#,
    );
    engine.execute(&put, "tenant_a", &ctx()).await.unwrap();

    let get = script(
        "tenant_b",
        "get",
        r#"return solidb.cache_get("iso_session") or "none""#,
    );
    let r = engine.execute(&get, "tenant_b", &ctx()).await.unwrap();
    assert_eq!(r.body, json!("none"));

    let get_a = script(
        "tenant_a",
        "get_a",
        r#"return solidb.cache_get("iso_session") or "none""#,
    );
    let r = engine.execute(&get_a, "tenant_a", &ctx()).await.unwrap();
    assert_eq!(r.body, json!("a-secret"));
}

/// Audit A4: history replay re-creates functions without repeating the
/// side effects of the command that defined them.
#[tokio::test]
async fn repl_replays_functions_without_side_effects() {
    let (engine, _tmp) = setup();
    let history = vec![
        r#"f = function() return 7 end; replay_counter = (replay_counter or 0) + 1; db:collection("replayed"):insert({a = 1})"#.to_string(),
    ];
    let mut vars = HashMap::new();
    vars.insert("replay_counter".to_string(), json!(1));
    let mut out = Vec::new();
    let (value, updated) = engine
        .execute_repl(
            "return f()",
            "tenant_a",
            ScriptUser::anonymous(),
            &vars,
            &history,
            &mut out,
            5_000,
        )
        .await
        .expect("repl eval");
    assert_eq!(value, json!(7), "the function is re-created");
    assert_eq!(
        updated.get("replay_counter"),
        Some(&json!(1)),
        "the rest of the command is not re-executed against the session"
    );
}

/// Audit A4: the REPL honours its deadline.
#[tokio::test]
async fn repl_busy_loop_is_stopped() {
    let (engine, _tmp) = setup();
    let mut out = Vec::new();
    let started = std::time::Instant::now();
    let result = engine
        .execute_repl(
            "while true do end",
            "tenant_a",
            ScriptUser::anonymous(),
            &HashMap::new(),
            &[],
            &mut out,
            200,
        )
        .await;
    assert!(result.is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}
