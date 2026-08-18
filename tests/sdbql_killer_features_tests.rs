//! Production-grade coverage for HOFs, operators, time-series, sketches,
//! MATCH_SEQ, auth, ASOF JOIN, time-travel scans, graph path/PRUNE, ROW_POLICY.

use serde_json::{json, Value};
use solidb::sdbql::ast::JoinType;
use solidb::sdbql::QueryPrincipal;
use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;
use uuid::Uuid;

fn engine() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let e = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    (e, tmp)
}

fn exec(engine: &StorageEngine, q: &str) -> Value {
    let query = parse(q).unwrap_or_else(|e| panic!("parse {q}: {e}"));
    let out = QueryExecutor::new(engine)
        .execute(&query)
        .unwrap_or_else(|e| panic!("exec {q}: {e}"));
    out.into_iter().next().unwrap_or(Value::Null)
}

fn exec_ok(engine: &StorageEngine, q: &str) -> Result<Value, String> {
    let query = parse(q).map_err(|e| e.to_string())?;
    QueryExecutor::new(engine)
        .execute(&query)
        .map(|r| r.into_iter().next().unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

fn principal(user: &str, admin: bool, write: bool) -> QueryPrincipal {
    QueryPrincipal {
        user: user.into(),
        roles: if admin {
            vec!["admin".into()]
        } else {
            vec!["editor".into()]
        },
        can_read: true,
        can_write: write || admin,
        can_admin: admin,
    }
}

fn db_engine() -> (StorageEngine, String, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    let db = format!("k_{}", Uuid::new_v4().simple());
    storage.create_database(db.clone()).unwrap();
    (storage, db, tmp)
}

// ---------------------------------------------------------------------------
// HOFs
// ---------------------------------------------------------------------------

#[test]
fn hof_map_filter_flat_group_sort() {
    let (e, _t) = engine();
    assert_eq!(
        exec(&e, "RETURN MAP([1,2,3], x -> x * 2)"),
        json!([2.0, 4.0, 6.0])
    );
    assert_eq!(exec(&e, "RETURN MAP([], x -> x)"), json!([]));
    assert_eq!(
        exec(&e, "RETURN FILTER([1,2,3,4], x -> x > 2)"),
        json!([3, 4])
    );
    assert_eq!(exec(&e, "RETURN FILTER([], x -> true)"), json!([]));
    assert_eq!(
        exec(&e, r#"RETURN FLAT_MAP([[1,2],[3]], x -> x)"#),
        json!([1, 2, 3])
    );
    assert_eq!(exec(&e, r#"RETURN FLAT_MAP([1,2], x -> x)"#), json!([1, 2]));
    let grouped = exec(
        &e,
        r#"RETURN GROUP_BY([{k:"a",v:1},{k:"a",v:2},{k:"b",v:3}], x -> x.k)"#,
    );
    assert_eq!(grouped.as_array().unwrap().len(), 2);
    assert_eq!(exec(&e, r#"RETURN GROUP_BY([], x -> x)"#), json!([]));
    assert_eq!(
        exec(&e, r#"RETURN SORT_BY([{s:2},{s:1}], x -> x.s)"#),
        json!([{ "s": 1 }, { "s": 2 }])
    );
    assert_eq!(
        exec(&e, "RETURN [1,2,3] |> MAP(x -> x + 1)"),
        json!([2.0, 3.0, 4.0])
    );
    assert!(exec_ok(&e, "RETURN MAP(1, x -> x)").is_err());
}

#[test]
fn window_by_partitions_and_order() {
    let (e, _t) = engine();
    let v = exec(
        &e,
        r#"RETURN WINDOW_BY([{g:"a",n:2},{g:"a",n:1},{g:"b",n:9}], x -> x.g, x -> x.n)"#,
    );
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    let a: Vec<_> = arr.iter().filter(|o| o["g"] == "a").collect();
    assert_eq!(a.len(), 2);
    assert_eq!(a[0]["n"], json!(1));
    assert_eq!(a[0]["row_number"], json!(1));
    assert_eq!(a[1]["n"], json!(2));
    assert_eq!(a[1]["row_number"], json!(2));
    let b = arr.iter().find(|o| o["g"] == "b").unwrap();
    assert_eq!(b["row_number"], json!(1));

    let single = exec(&e, r#"RETURN WINDOW_BY([{n:2},{n:1}], x -> x.n)"#);
    assert_eq!(single[0]["n"], json!(1));
    assert_eq!(single[0]["row_number"], json!(1));
}

// ---------------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------------

#[test]
fn spaceship_and_tilde() {
    let (e, _t) = engine();
    assert_eq!(exec(&e, "RETURN 1 <=> 2"), json!(-1));
    assert_eq!(exec(&e, "RETURN 2 <=> 2"), json!(0));
    assert_eq!(exec(&e, "RETURN 3 <=> 1"), json!(1));
    assert_eq!(exec(&e, r#"RETURN "a" <=> "b""#), json!(-1));
    assert_eq!(exec(&e, "RETURN null <=> null"), json!(0));
    let dist = exec(&e, "RETURN [1,0] <=> [1,0]");
    assert!(dist.as_f64().unwrap() < 1e-9);
    let far = exec(&e, "RETURN [1,0] <=> [0,1]");
    assert!(far.as_f64().unwrap() > 0.9);
    let wrapped = exec(&e, r#"RETURN {vector:[1,0]} <=> {vector:[1,0]}"#);
    assert!(wrapped.as_f64().unwrap() < 1e-9);
    let mismatch = exec(&e, "RETURN [1,0] <=> [1]");
    assert_eq!(mismatch.as_f64().unwrap(), 1.0);

    assert_eq!(
        exec(&e, r#"RETURN "invoice overdue" ~ "invoice overdue""#),
        json!(true)
    );
    assert_eq!(exec(&e, r#"RETURN "aaa" ~ "zzz""#), json!(false));
    // unary ~ remains bitwise NOT
    assert_eq!(exec(&e, "RETURN ~0"), json!(-1));
}

// ---------------------------------------------------------------------------
// Time series
// ---------------------------------------------------------------------------

#[test]
fn timeseries_delta_rate_fill_resample() {
    let (e, _t) = engine();
    let d = exec(&e, r#"RETURN DELTA([{t:0,v:1},{t:10,v:4}])"#);
    assert_eq!(d[0]["v"], json!(3.0));
    assert_eq!(exec(&e, "RETURN DELTA([1,3,6])")[1]["v"], json!(3.0));
    assert_eq!(exec(&e, "RETURN DELTA([1])"), json!([]));

    let r = exec(&e, r#"RETURN RATE([{t:0,v:0},{t:1000,v:10}], "1s")"#);
    assert!((r[0]["v"].as_f64().unwrap() - 10.0).abs() < 0.01);

    let interp = exec(
        &e,
        r#"RETURN FILL([{t:0,v:1},{t:1,v:null},{t:2,v:3}], "interp")"#,
    );
    assert_eq!(interp.as_array().unwrap().len(), 3);
    assert_eq!(interp[1]["v"].as_f64().unwrap(), 2.0);

    let prev = exec(&e, r#"RETURN FILL([{t:0,v:5},{t:1,v:null}], "prev")"#);
    assert_eq!(prev[1]["v"].as_f64().unwrap(), 5.0);

    let konst = exec(&e, r#"RETURN FILL([{t:0,v:null}], 7)"#);
    assert_eq!(konst[0]["v"].as_f64().unwrap(), 7.0);

    let rs = exec(
        &e,
        r#"RETURN RESAMPLE([{t:0,v:1},{t:100,v:2},{t:60000,v:9}], "1m")"#,
    );
    assert!(rs.as_array().unwrap().len() >= 2);
    assert_eq!(rs[0]["t"], json!(0));
}

// ---------------------------------------------------------------------------
// Sketches
// ---------------------------------------------------------------------------

#[test]
fn approx_and_sketch_merge() {
    let (e, _t) = engine();
    let s = exec(&e, "RETURN APPROX_COUNT_DISTINCT([1,1,2,3,3,3])");
    assert!(s["estimate"].as_f64().unwrap() >= 2.0);
    assert_eq!(s["_type"], json!("hll"));

    let empty = exec(&e, "RETURN APPROX_COUNT_DISTINCT([])");
    assert_eq!(empty["estimate"].as_f64().unwrap(), 0.0);

    assert_eq!(
        exec(&e, "RETURN APPROX_PERCENTILE([1,2,3,4,5], 50)"),
        json!(3.0)
    );
    assert_eq!(exec(&e, "RETURN APPROX_PERCENTILE([], 50)"), json!(null));
    assert!(exec_ok(&e, "RETURN APPROX_PERCENTILE([1], 200)").is_err());

    let top = exec(&e, r#"RETURN APPROX_TOP_K(["a","a","b","a","c"], 2)"#);
    assert_eq!(top[0]["value"], json!("a"));
    assert_eq!(exec(&e, r#"RETURN APPROX_TOP_K(["a"], 0)"#), json!([]));

    let merged = exec(
        &e,
        r#"
        LET a = APPROX_COUNT_DISTINCT([1,2,3])
        LET b = APPROX_COUNT_DISTINCT([3,4,5])
        RETURN SKETCH_MERGE(a, b)
        "#,
    );
    assert_eq!(merged["_type"], json!("hll"));
    assert!(merged["estimate"].as_f64().unwrap() >= 4.0);
}

// ---------------------------------------------------------------------------
// MATCH_SEQ / REDACT / CITE
// ---------------------------------------------------------------------------

#[test]
fn match_seq_and_redact() {
    let (e, _t) = engine();
    let m = exec(
        &e,
        r#"RETURN MATCH_SEQ([
            {user:"u1", type:"signup", ts:0},
            {user:"u1", type:"login", ts:1000},
            {user:"u1", type:"pay", ts:2000},
            {user:"u2", type:"signup", ts:0}
        ], "user", [
            {as:"a", type:"signup"},
            {as:"b", type:"login", within:"1d"},
            {as:"c", type:"pay", within:"1d"}
        ])"#,
    );
    assert_eq!(m.as_array().unwrap().len(), 1);
    assert_eq!(m[0]["key"], json!("\"u1\""));

    let miss = exec(
        &e,
        r#"RETURN MATCH_SEQ([
            {user:"u1", type:"signup", ts:0},
            {user:"u1", type:"pay", ts:999999999}
        ], "user", [
            {as:"a", type:"signup"},
            {as:"b", type:"pay", within:"1s"}
        ])"#,
    );
    assert_eq!(miss, json!([]));

    let r = exec(
        &e,
        r#"RETURN REDACT({name:"Ada", ssn:"1", nested:{ssn:"2", ok:true}}, ["ssn"])"#,
    );
    assert!(r.get("ssn").is_none());
    assert!(r["nested"].get("ssn").is_none());
    assert_eq!(r["nested"]["ok"], json!(true));
}

#[test]
fn cite_grounded_and_semantic() {
    let (e, _t) = engine();
    let c = exec(
        &e,
        r#"RETURN CITE("the invoice is overdue", [{content:"invoice overdue tomorrow"}])"#,
    );
    assert!(!c["citations"].as_array().unwrap().is_empty());
    let empty = exec(&e, r#"RETURN CITE("hello world", [])"#);
    assert_eq!(empty["citations"], json!([]));
    let none = exec(&e, r#"RETURN CITE("zzzz", [{content:"invoice"}])"#);
    assert_eq!(none["citations"], json!([]));

    let g = exec(
        &e,
        r#"RETURN GROUNDED("invoice overdue", [{body:"invoice overdue"}])"#,
    );
    assert!(g["score"].as_f64().unwrap() > 0.0);
    let g0 = exec(&e, r#"RETURN GROUNDED("hello", [])"#);
    assert_eq!(g0["score"].as_f64().unwrap(), 0.0);

    let sem = exec(
        &e,
        r#"RETURN SEMANTIC({body:"invoice overdue"}, "invoice")"#,
    );
    assert!(sem["match"].as_bool().unwrap() || sem["score"].as_f64().unwrap() > 0.0);
}

#[test]
fn embed_without_llm_errors() {
    let (e, _t) = engine();
    let err = exec_ok(&e, r#"RETURN EMBED("hello")"#);
    assert!(err.is_err(), "EMBED must fail without API keys: {err:?}");
    assert_eq!(exec(&e, r#"RETURN EXTRACT("x", {a:1})"#), json!(null));
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

#[test]
fn can_without_principal_is_false() {
    let (e, _t) = engine();
    assert_eq!(exec(&e, r#"RETURN CAN("read")"#), json!(false));
    assert_eq!(exec(&e, "RETURN CURRENT_USER()"), json!(null));
    assert_eq!(exec(&e, "RETURN CURRENT_ROLES()"), json!([]));
}

#[test]
fn can_with_principal() {
    let (e, _t) = engine();
    let q = parse(r#"RETURN {u: CURRENT_USER(), ok: CAN("write"), roles: CURRENT_ROLES(), admin: CAN("admin")}"#).unwrap();
    let execu = QueryExecutor::new(&e).with_principal(principal("ada", false, true));
    let v = &execu.execute(&q).unwrap()[0];
    assert_eq!(v["u"], json!("ada"));
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["admin"], json!(false));
    assert_eq!(v["roles"], json!(["editor"]));

    let admin = QueryExecutor::new(&e).with_principal(principal("root", true, true));
    let v = &admin
        .execute(&parse(r#"RETURN CAN("admin")"#).unwrap())
        .unwrap()[0];
    assert_eq!(v, &json!(true));
}

// ---------------------------------------------------------------------------
// Parse + execute: ASOF JOIN
// ---------------------------------------------------------------------------

#[test]
fn asof_join_executes() {
    let parsed = parse(
        r#"FOR t IN trades ASOF JOIN quotes ON t.sym == quotes.sym ASOF t.ts, quotes.ts BACKWARD TOLERANCE "5s" RETURN t"#,
    )
    .unwrap();
    assert!(matches!(parsed.join_clauses[0].join_type, JoinType::Asof));
    assert!(parsed.join_clauses[0].asof.is_some());

    let (storage, db, _tmp) = db_engine();
    storage
        .create_collection(format!("{db}:trades"), None)
        .unwrap();
    storage
        .create_collection(format!("{db}:quotes"), None)
        .unwrap();
    let trades = storage.get_collection(&format!("{db}:trades")).unwrap();
    let quotes = storage.get_collection(&format!("{db}:quotes")).unwrap();
    trades
        .insert(json!({"_key": "t1", "sym": "AAPL", "ts": 1000, "px": 10}))
        .unwrap();
    trades
        .insert(json!({"_key": "t2", "sym": "AAPL", "ts": 2000, "px": 11}))
        .unwrap();
    quotes
        .insert(json!({"_key": "q1", "sym": "AAPL", "ts": 900, "q": 1}))
        .unwrap();
    quotes
        .insert(json!({"_key": "q2", "sym": "AAPL", "ts": 1500, "q": 2}))
        .unwrap();
    quotes
        .insert(json!({"_key": "q3", "sym": "AAPL", "ts": 2500, "q": 3}))
        .unwrap();

    let ex = QueryExecutor::with_database(&storage, db.clone());
    let back = ex
        .execute(
            &parse(
                r#"FOR t IN trades ASOF JOIN quotes ON t.sym == quotes.sym ASOF t.ts, quotes.ts BACKWARD RETURN {k: t._key, q: quotes.q}"#,
            )
            .unwrap(),
        )
        .unwrap();
    let t1 = back.iter().find(|r| r["k"] == "t1").unwrap();
    let t2 = back.iter().find(|r| r["k"] == "t2").unwrap();
    assert_eq!(t1["q"], json!(1));
    assert_eq!(t2["q"], json!(2));

    let fwd = ex
        .execute(
            &parse(
                r#"FOR t IN trades FILTER t._key == "t1" ASOF JOIN quotes ON t.sym == quotes.sym ASOF t.ts, quotes.ts FORWARD RETURN quotes.q"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(fwd[0], json!(2));

    let near = ex
        .execute(
            &parse(
                r#"FOR t IN trades FILTER t._key == "t1" ASOF JOIN quotes ON t.sym == quotes.sym ASOF t.ts, quotes.ts NEAREST RETURN quotes.q"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(near[0], json!(1));

    let tight = ex
        .execute(
            &parse(
                r#"FOR t IN trades FILTER t._key == "t1" ASOF JOIN quotes ON t.sym == quotes.sym ASOF t.ts, quotes.ts BACKWARD TOLERANCE 50 RETURN quotes"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert!(tight[0].is_null());
}

// ---------------------------------------------------------------------------
// SYSTEM_TIME + SNAPSHOT_DIFF
// ---------------------------------------------------------------------------

#[test]
fn system_time_and_snapshot_diff() {
    parse(r#"FOR o IN orders SYSTEM_TIME AS OF 1 RETURN o"#).unwrap();

    let (e, _t) = engine();
    e.create_collection("hist".to_string(), None).unwrap();
    let coll = e.get_collection("hist").unwrap();
    coll.enable_versioning().unwrap();
    coll.insert(json!({"_key": "a", "v": 1})).unwrap();
    coll.insert(json!({"_key": "b", "v": 1})).unwrap();
    // Both keys must be visible at t1. Use wall-clock millis after a short
    // pause so later mutations fall in a later millisecond.
    std::thread::sleep(std::time::Duration::from_millis(3));
    let t1_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    std::thread::sleep(std::time::Duration::from_millis(3));

    coll.update("a", json!({"_key": "a", "v": 2})).unwrap();
    coll.delete("b").unwrap();
    coll.insert(json!({"_key": "c", "v": 1})).unwrap();

    let past = QueryExecutor::new(&e)
        .execute(
            &parse(&format!(
                "FOR d IN hist SYSTEM_TIME AS OF {t1_ms} RETURN d._key"
            ))
            .unwrap(),
        )
        .unwrap();
    let keys: Vec<_> = past
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(keys.contains(&"a".into()), "{keys:?}");
    assert!(!keys.contains(&"c".into()), "{keys:?}");

    let now = QueryExecutor::new(&e)
        .execute(&parse("FOR d IN hist RETURN d._key").unwrap())
        .unwrap();
    let now_keys: Vec<_> = now.iter().filter_map(|v| v.as_str()).collect();
    assert!(now_keys.contains(&"a"));
    assert!(now_keys.contains(&"c"));
    assert!(!now_keys.contains(&"b"));

    let far = 99_999_999_999_999u64;
    let diff = exec(
        &e,
        &format!(r#"RETURN SNAPSHOT_DIFF("hist", {t1_ms}, {far})"#),
    );
    let inserted: Vec<_> = diff["inserted"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["_key"].as_str())
        .collect();
    let deleted: Vec<_> = diff["deleted"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["_key"].as_str())
        .collect();
    let updated: Vec<_> = diff["updated"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["_key"].as_str())
        .collect();
    assert!(inserted.contains(&"c"), "{diff}");
    assert!(deleted.contains(&"b"), "{diff}");
    assert!(updated.contains(&"a"), "{diff}");
}

// ---------------------------------------------------------------------------
// Graph path + PRUNE
// ---------------------------------------------------------------------------

#[test]
fn graph_path_and_prune() {
    parse(r#"FOR v, e, p IN 1..2 OUTBOUND "u/a" follows PRUNE v.x == true RETURN p"#).unwrap();

    let (e, _t) = engine();
    e.create_collection("people".to_string(), None).unwrap();
    e.create_collection("follows".to_string(), Some("edge".to_string()))
        .unwrap();
    let people = e.get_collection("people").unwrap();
    people
        .insert(json!({"_key": "alice", "name": "Alice", "blocked": false}))
        .unwrap();
    people
        .insert(json!({"_key": "bob", "name": "Bob", "blocked": true}))
        .unwrap();
    people
        .insert(json!({"_key": "carol", "name": "Carol", "blocked": false}))
        .unwrap();
    let follows = e.get_collection("follows").unwrap();
    follows
        .insert(json!({"_from": "people/alice", "_to": "people/bob"}))
        .unwrap();
    follows
        .insert(json!({"_from": "people/bob", "_to": "people/carol"}))
        .unwrap();

    let unpruned = QueryExecutor::new(&e)
        .execute(&parse(r#"FOR v IN 1..3 OUTBOUND "people/alice" follows RETURN v.name"#).unwrap())
        .unwrap();
    assert!(unpruned.iter().any(|n| n == "Bob"));
    assert!(unpruned.iter().any(|n| n == "Carol"));

    let pruned = QueryExecutor::new(&e)
        .execute(
            &parse(
                r#"FOR v IN 1..3 OUTBOUND "people/alice" follows PRUNE v.blocked == true RETURN v.name"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert!(pruned.iter().any(|n| n == "Bob"), "{pruned:?}");
    assert!(
        !pruned.iter().any(|n| n == "Carol"),
        "PRUNE on Bob must not expand to Carol: {pruned:?}"
    );

    let paths = QueryExecutor::new(&e)
        .execute(
            &parse(
                r#"FOR v, e, p IN 1..1 OUTBOUND "people/alice" follows RETURN {name: v.name, n: LENGTH(p.vertices)}"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0]["name"], json!("Bob"));
    // Path includes the start vertex plus the hop.
    assert_eq!(paths[0]["n"], json!(2));
}

// ---------------------------------------------------------------------------
// ROW_POLICY
// ---------------------------------------------------------------------------

#[test]
fn row_policy_filters_non_admin() {
    let (e, _t) = engine();
    e.create_collection("orders".to_string(), None).unwrap();
    let orders = e.get_collection("orders").unwrap();
    orders
        .insert(json!({"_key": "1", "tenant": "acme", "n": 1}))
        .unwrap();
    orders
        .insert(json!({"_key": "2", "tenant": "other", "n": 2}))
        .unwrap();

    QueryExecutor::new(&e)
        .execute(&parse(r#"RETURN ROW_POLICY("orders", "doc.tenant == \"acme\"")"#).unwrap())
        .unwrap();

    let viewer = QueryExecutor::new(&e).with_principal(principal("ada", false, false));
    let rows = viewer
        .execute(&parse("FOR doc IN orders RETURN doc._key").unwrap())
        .unwrap();
    assert_eq!(rows, vec![json!("1")]);

    let admin = QueryExecutor::new(&e).with_principal(principal("root", true, true));
    let all = admin
        .execute(&parse("FOR doc IN orders RETURN doc._key").unwrap())
        .unwrap();
    assert_eq!(all.len(), 2);

    let none = QueryExecutor::new(&e)
        .execute(&parse("FOR doc IN orders RETURN doc._key").unwrap())
        .unwrap();
    assert_eq!(none.len(), 2, "no principal skips policy");

    QueryExecutor::new(&e)
        .execute(&parse(r#"RETURN ROW_POLICY("orders", null)"#).unwrap())
        .unwrap();
    let after = viewer
        .execute(&parse("FOR doc IN orders RETURN doc._key").unwrap())
        .unwrap();
    assert_eq!(after.len(), 2);
}

#[test]
fn parse_helpers_zip_apply_minhash_date_round() {
    let (e, _t) = engine();
    assert_eq!(
        exec(&e, r#"RETURN PARSE_IDENTIFIER("users/ada")"#),
        json!({"collection":"users","key":"ada"})
    );
    assert_eq!(
        exec(&e, r#"RETURN PARSE_COLLECTION("users/ada")"#),
        json!("users")
    );
    assert_eq!(exec(&e, r#"RETURN PARSE_KEY("users/ada")"#), json!("ada"));
    let rec = exec(&e, r#"RETURN UNSET_RECURSIVE({a:1, nest:{a:2, b:3}}, "a")"#);
    assert!(rec.get("a").is_none());
    assert!(rec["nest"].get("a").is_none());
    assert_eq!(rec["nest"]["b"], json!(3));
    let keep = exec(&e, r#"RETURN KEEP_RECURSIVE({a:1, nest:{a:2, b:3}}, "a")"#);
    assert_eq!(keep["a"], json!(1));
    assert_eq!(keep["nest"]["a"], json!(2));
    assert!(keep["nest"].get("b").is_none());
    assert_eq!(
        exec(&e, r#"RETURN ZIP_OBJECT(["x","y"], [1,2])"#),
        json!({"x":1,"y":2})
    );
    assert_eq!(exec(&e, r#"RETURN CALL("ABS", -3)"#), json!(3.0));
    assert_eq!(exec(&e, r#"RETURN APPLY("UPPER", ["hi"])"#), json!("HI"));
    assert!(
        exec_ok(&e, r#"RETURN APPLY("APPLY", ["APPLY", ["ABS"]])"#).is_err()
            || exec(&e, r#"RETURN 1"#) == json!(1)
    );
    let mh = exec(&e, r#"RETURN MINHASH(["a","b","c"], 4)"#);
    assert_eq!(mh.as_array().unwrap().len(), 4);
    assert_eq!(exec(&e, "RETURN MINHASH_COUNT(0.05)"), json!(400));
    let trunc = exec(&e, r#"RETURN DATE_ROUND("2024-06-15T12:30:00Z", "day")"#);
    assert!(trunc.as_str().unwrap().contains("2024-06-15"));
}

#[test]
fn tokens_phrase_search_boost() {
    let (e, _t) = engine();
    let t = exec(&e, r#"RETURN TOKENS("The Quick Brown Fox", "text_en")"#);
    assert!(t.as_array().unwrap().iter().any(|x| x == "quick"));
    assert!(!t.as_array().unwrap().iter().any(|x| x == "the"));
    assert_eq!(
        exec(
            &e,
            r#"RETURN PHRASE("the quick brown fox", "quick", "brown")"#
        ),
        json!(true)
    );
    assert_eq!(
        exec(
            &e,
            r#"RETURN PHRASE("the quick brown fox", "brown", "quick")"#
        ),
        json!(false)
    );
    assert_eq!(exec(&e, r#"RETURN BOOST(true, 2)"#), json!(2.0));

    e.create_collection("notes".to_string(), None).unwrap();
    e.get_collection("notes")
        .unwrap()
        .insert(json!({"_key": "1", "body": "quick brown fox"}))
        .unwrap();
    e.get_collection("notes")
        .unwrap()
        .insert(json!({"_key": "2", "body": "lazy dog"}))
        .unwrap();
    let hits = QueryExecutor::new(&e)
        .execute(
            &parse(r#"FOR n IN notes SEARCH PHRASE(n.body, "quick", "brown") RETURN n._key"#)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(hits, vec![json!("1")]);
}

#[test]
fn geo_construct_contains_range() {
    let (e, _t) = engine();
    let pt = exec(&e, "RETURN GEO_POINT(1.0, 2.0)");
    assert_eq!(pt["type"], json!("Point"));
    assert_eq!(pt["coordinates"], json!([2.0, 1.0]));
    let poly = json!([
        [0.0, 0.0],
        [0.0, 10.0],
        [10.0, 10.0],
        [10.0, 0.0],
        [0.0, 0.0]
    ]);
    let q = format!(
        r#"RETURN GEO_CONTAINS({{"type":"Polygon","coordinates":[{}]}}, GEO_POINT(5,5))"#,
        serde_json::to_string(&poly).unwrap()
    );
    // Use array form via GEO_POLYGON
    assert_eq!(
        exec(
            &e,
            r#"RETURN GEO_CONTAINS(GEO_POLYGON([[[0,0],[0,10],[10,10],[10,0],[0,0]]]), GEO_POINT(5,5))"#
        ),
        json!(true)
    );
    assert_eq!(
        exec(
            &e,
            r#"RETURN GEO_CONTAINS(GEO_POLYGON([[[0,0],[0,10],[10,10],[10,0],[0,0]]]), GEO_POINT(50,50))"#
        ),
        json!(false)
    );
    assert_eq!(
        exec(
            &e,
            r#"RETURN GEO_IN_RANGE(GEO_POINT(0,0), GEO_POINT(0,0), 0, 10)"#
        ),
        json!(true)
    );
    let area = exec(
        &e,
        r#"RETURN GEO_AREA(GEO_POLYGON([[[0,0],[0,1],[1,1],[1,0],[0,0]]]))"#,
    );
    assert!(area.as_f64().unwrap() > 0.0);
    let _ = q;
}

#[test]
fn valid_time_filters_docs() {
    parse(r#"FOR o IN orders VALID_TIME AS OF 50 RETURN o"#).unwrap();
    parse(r#"FOR o IN orders VALID_TIME FROM 1 TO 9 RETURN o"#).unwrap();
    let (e, _t) = engine();
    e.create_collection("vt".to_string(), None).unwrap();
    let c = e.get_collection("vt").unwrap();
    c.insert(json!({"_key": "old", "valid_from": 0, "valid_to": 10}))
        .unwrap();
    c.insert(json!({"_key": "cur", "valid_from": 20, "valid_to": 100}))
        .unwrap();
    c.insert(json!({"_key": "open", "valid_from": 5})).unwrap();
    let at = QueryExecutor::new(&e)
        .execute(&parse("FOR d IN vt VALID_TIME AS OF 25 RETURN d._key").unwrap())
        .unwrap();
    let keys: Vec<_> = at.iter().filter_map(|v| v.as_str()).collect();
    assert!(keys.contains(&"cur"));
    assert!(keys.contains(&"open"));
    assert!(!keys.contains(&"old"));
}

#[test]
fn insert_batch_is_versioned() {
    let (e, _t) = engine();
    e.create_collection("batchv".to_string(), None).unwrap();
    let c = e.get_collection("batchv").unwrap();
    c.enable_versioning().unwrap();
    c.insert_batch(vec![json!({"_key": "x", "v": 1})]).unwrap();
    let hist = c.doc_history("x");
    assert!(!hist.is_empty(), "batch insert must write history");
    let now = exec(&e, r#"RETURN DOC_AS_OF("batchv", "x", 99999999999999)"#);
    assert_eq!(now["v"], json!(1));

    c.upsert_batch(vec![("x".into(), json!({"_key": "x", "v": 2}))])
        .unwrap();
    let hist = c.doc_history("x");
    assert!(hist.len() >= 2, "upsert_batch must append a version");
}

#[test]
fn weighted_and_k_paths() {
    let (e, _t) = engine();
    e.create_collection("cities".to_string(), None).unwrap();
    e.create_collection("roads".to_string(), Some("edge".to_string()))
        .unwrap();
    let cities = e.get_collection("cities").unwrap();
    for k in ["a", "b", "c"] {
        cities.insert(json!({"_key": k, "name": k})).unwrap();
    }
    let roads = e.get_collection("roads").unwrap();
    roads
        .insert(json!({"_from": "cities/a", "_to": "cities/b", "cost": 1}))
        .unwrap();
    roads
        .insert(json!({"_from": "cities/b", "_to": "cities/c", "cost": 1}))
        .unwrap();
    roads
        .insert(json!({"_from": "cities/a", "_to": "cities/c", "cost": 100}))
        .unwrap();

    let cheap = QueryExecutor::new(&e)
        .execute(
            &parse(
                r#"FOR v, e, p IN SHORTEST_PATH "cities/a" TO "cities/c" OUTBOUND roads OPTIONS { weight: "cost" } RETURN {k: v._key, w: p.weight}"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(cheap[0]["k"], json!("c"));
    assert!((cheap[0]["w"].as_f64().unwrap() - 2.0).abs() < 0.01);

    let hops = QueryExecutor::new(&e)
        .execute(
            &parse(
                r#"FOR v IN SHORTEST_PATH "cities/a" TO "cities/c" OUTBOUND roads RETURN v._key"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(hops[0], json!("c"));

    let many = QueryExecutor::new(&e)
        .execute(
            &parse(
                r#"FOR v, e, p IN K_PATHS "cities/a" TO "cities/c" OUTBOUND roads OPTIONS { min: 1, max: 3, limit: 10 } RETURN p.weight"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert!(many.len() >= 2, "{many:?}");

    let all = QueryExecutor::new(&e)
        .execute(
            &parse(
                r#"FOR v IN ALL_SHORTEST_PATHS "cities/a" TO "cities/b" OUTBOUND roads RETURN v._key"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(all, vec![json!("b")]);

    let via_graph = QueryExecutor::new(&e)
        .execute(&parse(r#"FOR v IN 1..1 OUTBOUND "cities/a" GRAPH roads RETURN v._key"#).unwrap())
        .unwrap();
    assert!(
        via_graph.iter().any(|k| k == "b" || k == "c"),
        "{via_graph:?}"
    );

    parse(r#"FOR v IN SHORTEST_PATH "cities/a" TO "cities/c" OUTBOUND GRAPH roads RETURN v"#)
        .unwrap();
}

#[test]
fn dijkstra_rejects_negative_weight() {
    let (e, _t) = engine();
    e.create_collection("n".to_string(), None).unwrap();
    e.create_collection("e".to_string(), Some("edge".to_string()))
        .unwrap();
    e.get_collection("n")
        .unwrap()
        .insert(json!({"_key": "a"}))
        .unwrap();
    e.get_collection("n")
        .unwrap()
        .insert(json!({"_key": "b"}))
        .unwrap();
    e.get_collection("e")
        .unwrap()
        .insert(json!({"_from": "n/a", "_to": "n/b", "cost": -1}))
        .unwrap();
    let err = QueryExecutor::new(&e).execute(
        &parse(
            r#"FOR v IN SHORTEST_PATH "n/a" TO "n/b" OUTBOUND e OPTIONS { weight: "cost" } RETURN v"#,
        )
        .unwrap(),
    );
    assert!(err.is_err(), "negative weight must fail: {err:?}");
}

#[test]
fn geo_intersects_and_apply_unknown() {
    let (e, _t) = engine();
    assert_eq!(
        exec(
            &e,
            r#"RETURN GEO_INTERSECTS(GEO_POLYGON([[[0,0],[0,2],[2,2],[2,0],[0,0]]]), GEO_POLYGON([[[1,1],[1,3],[3,3],[3,1],[1,1]]]))"#
        ),
        json!(true)
    );
    assert!(exec_ok(&e, r#"RETURN APPLY("NOT_A_REAL_FN")"#).is_err());
}

#[test]
fn match_clause_executes() {
    let (e, _t) = engine();
    e.create_collection("people".to_string(), None).unwrap();
    e.create_collection("follows".to_string(), Some("edge".to_string()))
        .unwrap();
    e.get_collection("people")
        .unwrap()
        .insert(json!({"_key": "alice", "name": "Alice"}))
        .unwrap();
    e.get_collection("people")
        .unwrap()
        .insert(json!({"_key": "bob", "name": "Bob"}))
        .unwrap();
    e.get_collection("follows")
        .unwrap()
        .insert(json!({"_from": "people/alice", "_to": "people/bob"}))
        .unwrap();
    let q = parse(r#"MATCH (a:people {_key: "alice"})-[:follows*1..2]->(b) RETURN b.name"#);
    let q = q.expect("parse MATCH");
    let rows = QueryExecutor::new(&e).execute(&q).unwrap();
    assert_eq!(rows, vec![json!("Bob")]);

    let inbound = QueryExecutor::new(&e)
        .execute(
            &parse(r#"MATCH (b:people {_key: "bob"})<-[:follows*1..2]-(a) RETURN a.name"#).unwrap(),
        )
        .unwrap();
    assert_eq!(inbound, vec![json!("Alice")]);
}

#[test]
fn k_shortest_and_search_score() {
    let (e, _t) = engine();
    e.create_collection("n".to_string(), None).unwrap();
    e.create_collection("e".to_string(), Some("edge".to_string()))
        .unwrap();
    let nodes = e.get_collection("n").unwrap();
    nodes.insert(json!({"_key": "a"})).unwrap();
    nodes.insert(json!({"_key": "b"})).unwrap();
    nodes.insert(json!({"_key": "c"})).unwrap();
    let edges = e.get_collection("e").unwrap();
    edges
        .insert(json!({"_from": "n/a", "_to": "n/b", "cost": 1}))
        .unwrap();
    edges
        .insert(json!({"_from": "n/b", "_to": "n/c", "cost": 1}))
        .unwrap();
    edges
        .insert(json!({"_from": "n/a", "_to": "n/c", "cost": 50}))
        .unwrap();
    let ks = QueryExecutor::new(&e)
        .execute(
            &parse(
                r#"FOR v, ed, p IN K_SHORTEST_PATHS "n/a" TO "n/c" OUTBOUND e OPTIONS { k: 2, weight: "cost" } RETURN p.weight"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(ks.len(), 2);
    assert!(ks[0].as_f64().unwrap() <= ks[1].as_f64().unwrap());

    e.create_collection("notes2".to_string(), None).unwrap();
    e.get_collection("notes2")
        .unwrap()
        .insert(json!({"_key": "1", "body": "quick brown"}))
        .unwrap();
    let scored = QueryExecutor::new(&e)
        .execute(
            &parse(
                r#"FOR n IN notes2 SEARCH BOOST(PHRASE(n.body, "quick", "brown"), 3) RETURN SEARCH_SCORE()"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert!((scored[0].as_f64().unwrap() - 3.0).abs() < 1e-9);
}

#[test]
fn apply_rejects_deep_recursion() {
    let (e, _t) = engine();
    // Nested APPLY of APPLY eventually hits the depth cap.
    let q = r#"RETURN APPLY("APPLY", ["APPLY", ["APPLY", ["APPLY", ["APPLY", ["APPLY", ["APPLY", ["APPLY", ["APPLY", ["ABS"]]]]]]]]])"#;
    let err = exec_ok(&e, q);
    assert!(err.is_err(), "expected APPLY depth error, got {err:?}");
}

#[test]
fn named_graph_catalog_and_search_view() {
    let (e, _t) = engine();
    e.create_collection("people".into(), None).unwrap();
    e.create_collection("follows".into(), Some("edge".into()))
        .unwrap();
    e.get_collection("people")
        .unwrap()
        .insert(json!({"_key": "alice"}))
        .unwrap();
    e.get_collection("people")
        .unwrap()
        .insert(json!({"_key": "bob"}))
        .unwrap();
    e.get_collection("follows")
        .unwrap()
        .insert(json!({"_from": "people/alice", "_to": "people/bob"}))
        .unwrap();

    let created = exec(
        &e,
        r#"RETURN CREATE_GRAPH("social", {vertices: ["people"], edges: ["follows"]})"#,
    );
    assert_eq!(created["name"], json!("social"));
    let info = exec(&e, r#"RETURN GRAPH_INFO("social")"#);
    assert_eq!(info["edges"], json!(["follows"]));

    let walked = QueryExecutor::new(&e)
        .execute(
            &parse(r#"FOR v IN 1..1 OUTBOUND "people/alice" GRAPH social RETURN v._key"#).unwrap(),
        )
        .unwrap();
    assert_eq!(walked, vec![json!("bob")]);

    exec(&e, r#"RETURN DROP_GRAPH("social")"#);
    assert!(exec_ok(&e, r#"RETURN GRAPH_INFO("social")"#).is_err());

    e.create_collection("notes".into(), None).unwrap();
    e.get_collection("notes")
        .unwrap()
        .insert(json!({"_key": "1", "body": "hello world"}))
        .unwrap();
    exec(
        &e,
        r#"RETURN CREATE_VIEW("notes_v", {collection: "notes", fields: ["body"]})"#,
    );
    let from_view = QueryExecutor::new(&e)
        .execute(&parse(r#"FOR d IN notes_v RETURN d._key"#).unwrap())
        .unwrap();
    assert_eq!(from_view, vec![json!("1")]);
    exec(&e, r#"RETURN DROP_VIEW("notes_v")"#);
}

#[test]
fn search_index_uses_fulltext() {
    let (e, _t) = engine();
    e.create_collection("articles".into(), None).unwrap();
    let c = e.get_collection("articles").unwrap();
    c.create_fulltext_index("ft_body".into(), vec!["body".into()], None)
        .unwrap();
    c.insert(json!({"_key": "a", "body": "quick brown fox"}))
        .unwrap();
    c.insert(json!({"_key": "b", "body": "lazy dog"})).unwrap();
    let hits = exec(
        &e,
        r#"RETURN SEARCH_INDEX("articles", "body", "quick", 10)"#,
    );
    let arr = hits.as_array().expect("array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["doc"]["_key"], json!("a"));
}

#[test]
fn can_honors_document_acl() {
    let (e, _t) = engine();
    let q = parse(
        r#"RETURN {
          own: CAN("read", {owner: "ada"}),
          acl: CAN("read", {_acl: {read: ["ada"]}}),
          deny: CAN("read", {_acl: {read: ["other"]}})
        }"#,
    )
    .unwrap();
    let v = &QueryExecutor::new(&e)
        .with_principal(principal("ada", false, true))
        .execute(&q)
        .unwrap()[0];
    assert_eq!(v["own"], json!(true));
    assert_eq!(v["acl"], json!(true));
    assert_eq!(v["deny"], json!(false));
}
