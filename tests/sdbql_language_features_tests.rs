//! SDBQL language features (audit "language" items):
//!
//! - function names normalised to upper case at parse time
//! - REPLACE statement, UPDATE without WITH, INSERT ... IN
//! - OLD / NEW after mutations
//! - OPTIONS on mutations, FOR, traversals
//! - inline array expressions `[* FILTER .. LIMIT .. RETURN ..]`, `[**]`
//! - array comparison operators `ANY ==`, `ALL IN`, `NONE >`, `AT LEAST (n)`
//! - COLLECT `INTO g = expr` and `OPTIONS { method }`
//! - window functions referenced from LET

use serde_json::{json, Value};
use solidb::sdbql::ast::{
    BodyClause, CollectMethod, Expression, OverwriteMode, TraversalOrder, UniqueVertices,
};
use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;

fn engine() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).expect("engine");
    (engine, tmp)
}

fn run(engine: &StorageEngine, q: &str) -> Vec<Value> {
    let query = parse(q).unwrap_or_else(|e| panic!("parse failed for {q}: {e:?}"));
    QueryExecutor::new(engine)
        .execute(&query)
        .unwrap_or_else(|e| panic!("query failed for {q}: {e:?}"))
}

fn run_err(engine: &StorageEngine, q: &str) -> String {
    let query = parse(q).unwrap_or_else(|e| panic!("parse failed for {q}: {e:?}"));
    QueryExecutor::new(engine)
        .execute(&query)
        .expect_err(&format!("expected an error for {q}"))
        .to_string()
}

fn one(engine: &StorageEngine, q: &str) -> Value {
    let mut rows = run(engine, q);
    assert_eq!(rows.len(), 1, "expected one row for {q}, got {rows:?}");
    rows.remove(0)
}

fn users(engine: &StorageEngine) {
    engine.create_collection("users".to_string(), None).unwrap();
    let c = engine.get_collection("users").unwrap();
    c.insert(json!({"_key": "alice", "name": "Alice", "age": 30, "prefs": {"theme": "light", "lang": "fr"}}))
        .unwrap();
    c.insert(json!({"_key": "bob", "name": "Bob", "age": 25, "prefs": {"theme": "dark"}}))
        .unwrap();
}

// ---------------------------------------------------------------------------
// 1. Function names
// ---------------------------------------------------------------------------

#[test]
fn function_names_are_uppercased_at_parse_time() {
    let q = parse("RETURN length([1, 2])").unwrap();
    match &q.return_clause.unwrap().expression {
        Expression::FunctionCall { name, .. } => assert_eq!(name, "LENGTH"),
        other => panic!("expected a function call, got {other:?}"),
    }
    let q = parse("RETURN [3, 1] |> sorted()").unwrap();
    match &q.return_clause.unwrap().expression {
        Expression::Pipeline { right, .. } => {
            assert!(
                matches!(right.as_ref(), Expression::FunctionCall { name, .. } if name == "SORTED")
            )
        }
        other => panic!("expected a pipeline, got {other:?}"),
    }

    let (engine, _tmp) = engine();
    assert_eq!(one(&engine, "RETURN length([1, 2, 3])"), json!(3));
    assert_eq!(one(&engine, "RETURN Upper('a')"), json!("A"));
}

// ---------------------------------------------------------------------------
// 2. REPLACE / UPDATE without WITH / INSERT ... IN
// ---------------------------------------------------------------------------

#[test]
fn replace_statement_with_document_naming_its_key() {
    let (engine, _tmp) = engine();
    users(&engine);
    let new = one(
        &engine,
        r#"REPLACE { _key: "alice", name: "Alicia" } IN users RETURN NEW"#,
    );
    assert_eq!(new["name"], json!("Alicia"));
    assert!(
        new.get("age").is_none(),
        "REPLACE drops absent attributes: {new}"
    );
    let stored = engine
        .get_collection("users")
        .unwrap()
        .get("alice")
        .unwrap();
    assert_eq!(stored.data, json!({"name": "Alicia"}));
}

#[test]
fn replace_key_with_document() {
    let (engine, _tmp) = engine();
    users(&engine);
    let rows = run(
        &engine,
        r#"FOR u IN users FILTER u._key == "bob"
           REPLACE u WITH { name: u.name, replaced: true } IN users
           RETURN { old: OLD.age, new: NEW }"#,
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["old"], json!(25));
    assert_eq!(rows[0]["new"]["replaced"], json!(true));
    assert!(rows[0]["new"].get("age").is_none());
    assert_eq!(rows[0]["new"]["_key"], json!("bob"));

    // By key string.
    run(&engine, r#"REPLACE "bob" WITH { v: 1 } IN users"#);
    let stored = engine.get_collection("users").unwrap().get("bob").unwrap();
    assert_eq!(stored.data, json!({"v": 1}));
}

#[test]
fn replace_missing_document_fails_unless_ignored() {
    let (engine, _tmp) = engine();
    users(&engine);
    let err = run_err(&engine, r#"REPLACE { _key: "nobody", x: 1 } IN users"#);
    assert!(err.to_lowercase().contains("not found"), "{err}");
    let rows = run(
        &engine,
        r#"REPLACE { _key: "nobody", x: 1 } IN users OPTIONS { ignoreErrors: true } RETURN NEW"#,
    );
    assert!(
        rows.is_empty(),
        "an ignored row produces no output: {rows:?}"
    );
    assert!(engine
        .get_collection("users")
        .unwrap()
        .get("nobody")
        .is_err());
}

#[test]
fn upsert_replace_branch_replaces() {
    let (engine, _tmp) = engine();
    users(&engine);
    let row = one(
        &engine,
        r#"UPSERT { _key: "alice" } INSERT { _key: "alice", fresh: true }
           REPLACE { name: "Replaced" } IN users RETURN { old: OLD.name, new: NEW }"#,
    );
    assert_eq!(row["old"], json!("Alice"));
    assert_eq!(row["new"]["name"], json!("Replaced"));
    assert!(row["new"].get("age").is_none(), "{row}");

    let row = one(
        &engine,
        r#"UPSERT { _key: "carol" } INSERT { _key: "carol", n: 1 }
           UPDATE { n: 2 } IN users RETURN { old: OLD, new: NEW.n }"#,
    );
    assert_eq!(row, json!({"old": null, "new": 1}));
}

#[test]
fn update_without_with_uses_the_document_as_patch() {
    let (engine, _tmp) = engine();
    users(&engine);
    let new = one(
        &engine,
        r#"UPDATE { _key: "alice", age: 31 } IN users RETURN NEW"#,
    );
    assert_eq!(new["age"], json!(31));
    assert_eq!(new["name"], json!("Alice"), "UPDATE keeps other attributes");

    run(
        &engine,
        r#"FOR u IN users FILTER u._key == "bob" UPDATE MERGE(u, { age: 26 }) IN users"#,
    );
    let bob = engine.get_collection("users").unwrap().get("bob").unwrap();
    assert_eq!(bob.data["age"], json!(26));
}

#[test]
fn insert_accepts_in_as_well_as_into() {
    let (engine, _tmp) = engine();
    users(&engine);
    let new = one(
        &engine,
        r#"INSERT { _key: "dan", n: 1 } IN users RETURN NEW"#,
    );
    assert_eq!(new["_key"], json!("dan"));
    let rows = run(
        &engine,
        r#"FOR i IN [1, 2] INSERT { i, ok: i IN [1, 2] } IN users RETURN NEW.ok"#,
    );
    assert_eq!(rows, vec![json!(true), json!(true)]);
}

#[test]
fn in_operator_inside_update_with_object() {
    let (engine, _tmp) = engine();
    users(&engine);
    // SORT before a mutation is not supported by the parser (SORT closes the
    // body), so compare the results as a set.
    let mut rows = run(
        &engine,
        r#"FOR u IN users
           UPDATE u WITH { young: u.age IN [25, 26] } IN users
           RETURN NEW.young"#,
    );
    rows.sort_by_key(|v| v.as_bool());
    assert_eq!(rows, vec![json!(false), json!(true)]);
}

// ---------------------------------------------------------------------------
// 3. OLD / NEW
// ---------------------------------------------------------------------------

#[test]
fn old_after_update_and_remove() {
    let (engine, _tmp) = engine();
    users(&engine);
    let row = one(
        &engine,
        r#"FOR u IN users FILTER u._key == "alice"
           UPDATE u WITH { age: 40 } IN users
           RETURN { before: OLD.age, after: NEW.age }"#,
    );
    assert_eq!(row, json!({"before": 30, "after": 40}));

    let row = one(
        &engine,
        r#"FOR u IN users FILTER u._key == "bob" REMOVE u IN users RETURN OLD.name"#,
    );
    assert_eq!(row, json!("Bob"));
    assert!(engine.get_collection("users").unwrap().get("bob").is_err());
}

#[test]
fn new_after_insert_small_and_bulk() {
    let (engine, _tmp) = engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let rows = run(
        &engine,
        "FOR i IN 1..3 INSERT { i } INTO items RETURN NEW.i",
    );
    assert_eq!(rows, vec![json!(1), json!(2), json!(3)]);

    // More than 100 rows takes the batch path; NEW still lines up.
    let rows = run(
        &engine,
        "FOR i IN 1..150 INSERT { i, tag: 'bulk' } INTO items RETURN NEW.i",
    );
    assert_eq!(rows.len(), 150);
    assert_eq!(rows[0], json!(1));
    assert_eq!(rows[149], json!(150));

    // Old is null for an insert.
    let row = one(&engine, "INSERT { x: 1 } INTO items RETURN OLD");
    assert_eq!(row, Value::Null);
}

#[test]
fn new_after_bulk_update() {
    let (engine, _tmp) = engine();
    engine.create_collection("items".to_string(), None).unwrap();
    run(&engine, "FOR i IN 1..150 INSERT { i } INTO items");
    let rows = run(
        &engine,
        "FOR d IN items UPDATE d WITH { doubled: d.i * 2 } IN items RETURN NEW.doubled - OLD.i",
    );
    assert_eq!(rows.len(), 150);
    let mut values: Vec<i64> = rows.iter().map(|v| v.as_i64().unwrap()).collect();
    values.sort();
    assert_eq!(values, (1..=150).collect::<Vec<_>>());
}

// ---------------------------------------------------------------------------
// 4. OPTIONS
// ---------------------------------------------------------------------------

#[test]
fn insert_overwrite_modes() {
    let (engine, _tmp) = engine();
    users(&engine);

    let err = run_err(&engine, r#"INSERT { _key: "alice" } INTO users"#);
    assert!(err.contains("already exists"), "{err}");

    let row = one(
        &engine,
        r#"INSERT { _key: "alice", name: "X" } INTO users OPTIONS { overwriteMode: "ignore" }
           RETURN { new: NEW, old: OLD.name }"#,
    );
    assert_eq!(row, json!({"new": null, "old": "Alice"}));
    let alice = engine
        .get_collection("users")
        .unwrap()
        .get("alice")
        .unwrap();
    assert_eq!(alice.data["name"], json!("Alice"));

    let row = one(
        &engine,
        r#"INSERT { _key: "alice", nick: "Al" } INTO users OPTIONS { overwriteMode: "update" }
           RETURN NEW"#,
    );
    assert_eq!(row["nick"], json!("Al"));
    assert_eq!(row["name"], json!("Alice"), "update merges");

    let row = one(
        &engine,
        r#"INSERT { _key: "alice", only: 1 } INTO users OPTIONS { overwriteMode: "replace" }
           RETURN NEW"#,
    );
    assert_eq!(row["only"], json!(1));
    assert!(row.get("name").is_none(), "replace drops attributes: {row}");

    // A key that does not exist is a plain insert under every mode.
    let row = one(
        &engine,
        r#"INSERT { _key: "zed", z: 1 } INTO users OPTIONS { overwriteMode: "replace" } RETURN NEW.z"#,
    );
    assert_eq!(row, json!(1));
}

#[test]
fn ignore_errors_skips_failed_rows() {
    let (engine, _tmp) = engine();
    users(&engine);
    let rows = run(
        &engine,
        r#"FOR k IN ["alice", "new1", "bob", "new2"]
           INSERT { _key: k } INTO users OPTIONS { ignoreErrors: true }
           RETURN NEW._key"#,
    );
    assert_eq!(rows, vec![json!("new1"), json!("new2")]);

    let rows = run(
        &engine,
        r#"FOR k IN ["alice", "ghost"] REMOVE k IN users OPTIONS { ignoreErrors: true } RETURN k"#,
    );
    assert_eq!(rows, vec![json!("alice")]);

    let rows = run(
        &engine,
        r#"FOR k IN ["bob", "ghost"] UPDATE k WITH { seen: true } IN users
           OPTIONS { ignoreErrors: true } RETURN NEW._key"#,
    );
    assert_eq!(rows, vec![json!("bob")]);

    let err = run_err(&engine, r#"REMOVE "ghost" IN users"#);
    assert!(err.to_lowercase().contains("not found"), "{err}");
}

#[test]
fn update_keep_null_and_merge_objects() {
    let (engine, _tmp) = engine();
    users(&engine);

    // Default: shallow merge, null stored.
    let new = one(
        &engine,
        r#"UPDATE "alice" WITH { age: null, prefs: { theme: "dark" } } IN users RETURN NEW"#,
    );
    assert_eq!(new["age"], Value::Null);
    assert!(new.as_object().unwrap().contains_key("age"));
    assert_eq!(new["prefs"], json!({"theme": "dark"}));

    // keepNull false removes; mergeObjects true merges nested objects.
    let new = one(
        &engine,
        r#"UPDATE "bob" WITH { age: null, prefs: { lang: "en", theme: null } } IN users
           OPTIONS { keepNull: false, mergeObjects: true } RETURN NEW"#,
    );
    assert!(!new.as_object().unwrap().contains_key("age"), "{new}");
    assert_eq!(new["prefs"], json!({"lang": "en"}));
    assert_eq!(new["name"], json!("Bob"));
}

#[test]
fn unknown_options_are_parse_errors() {
    assert!(parse("INSERT {} INTO c OPTIONS { overwriteMod: 'x' }").is_err());
    assert!(parse("INSERT {} INTO c OPTIONS { overwriteMode: 'sometimes' }").is_err());
    assert!(parse("REMOVE 'k' IN c OPTIONS { overwriteMode: 'replace' }").is_err());
    assert!(parse("INSERT {} INTO c OPTIONS { ignoreErrors: @flag }").is_err());
    // AQL knobs with no meaning here are accepted.
    assert!(parse("INSERT {} INTO c OPTIONS { waitForSync: true, exclusive: true }").is_ok());
}

#[test]
fn mutation_options_reach_the_ast() {
    let q = parse(
        r#"INSERT {} INTO c OPTIONS { overwriteMode: "update", keepNull: false, ignoreErrors: true }"#,
    )
    .unwrap();
    let BodyClause::Insert(ins) = &q.body_clauses[0] else {
        panic!("expected INSERT");
    };
    assert_eq!(ins.options.overwrite_mode, Some(OverwriteMode::Update));
    assert_eq!(ins.options.keep_null, Some(false));
    assert!(ins.options.ignore_errors);
}

#[test]
fn for_options_index_hint_is_parsed() {
    let q = parse(
        r#"FOR d IN docs OPTIONS { indexHint: ["a", "b"], forceIndexHint: true } FILTER d.x == 1 RETURN d"#,
    )
    .unwrap();
    let opts = q.for_clauses[0].options.clone().expect("options");
    assert_eq!(opts.index_hint, vec!["a".to_string(), "b".to_string()]);
    assert!(opts.force_index_hint);

    let q = parse(r#"FOR d IN docs OPTIONS { indexHint: "a" } RETURN d"#).unwrap();
    assert_eq!(
        q.for_clauses[0].options.as_ref().unwrap().index_hint,
        vec!["a"]
    );
    assert!(parse(r#"FOR d IN docs OPTIONS { hint: "a" } RETURN d"#).is_err());

    // The hint does not change results.
    let (engine, _tmp) = engine();
    users(&engine);
    let rows = run(
        &engine,
        r#"FOR u IN users OPTIONS { indexHint: "nope" } FILTER u.age > 26 RETURN u._key"#,
    );
    assert_eq!(rows, vec![json!("alice")]);
}

fn diamond(engine: &StorageEngine) {
    engine
        .create_collection("cities".to_string(), None)
        .unwrap();
    engine
        .create_collection("roads".to_string(), Some("edge".to_string()))
        .unwrap();
    let cities = engine.get_collection("cities").unwrap();
    for k in ["a", "b", "c", "d"] {
        cities.insert(json!({"_key": k, "name": k})).unwrap();
    }
    let roads = engine.get_collection("roads").unwrap();
    for (from, to) in [("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")] {
        roads
            .insert(json!({"_from": format!("cities/{from}"), "_to": format!("cities/{to}")}))
            .unwrap();
    }
}

#[test]
fn traversal_options_uniqueness_and_order() {
    let (engine, _tmp) = engine();
    diamond(&engine);

    // Default: BFS, each vertex once.
    let mut rows = run(
        &engine,
        r#"FOR v IN 1..2 OUTBOUND "cities/a" roads RETURN v.name"#,
    );
    rows.sort_by_key(|v| v.to_string());
    assert_eq!(rows, vec![json!("b"), json!("c"), json!("d")]);

    // Path uniqueness: d is reached by two distinct paths.
    let rows = run(
        &engine,
        r#"FOR v IN 1..2 OUTBOUND "cities/a" roads OPTIONS { uniqueVertices: "path" } RETURN v.name"#,
    );
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert_eq!(rows.iter().filter(|v| **v == json!("d")).count(), 2);

    // BFS emits both depth-1 vertices first; DFS goes down a branch first.
    let bfs = run(
        &engine,
        r#"FOR v, e, p IN 1..2 OUTBOUND "cities/a" roads OPTIONS { uniqueVertices: "path" }
           RETURN LENGTH(p.edges)"#,
    );
    assert_eq!(bfs, vec![json!(1), json!(1), json!(2), json!(2)]);
    let dfs = run(
        &engine,
        r#"FOR v, e, p IN 1..2 OUTBOUND "cities/a" roads OPTIONS { order: "dfs" }
           RETURN LENGTH(p.edges)"#,
    );
    assert_eq!(dfs, vec![json!(1), json!(2), json!(1), json!(2)]);

    let q = parse(
        r#"FOR v IN 1..2 OUTBOUND "cities/a" roads PRUNE v.name == "x" OPTIONS { order: "dfs" } RETURN v"#,
    )
    .unwrap();
    let BodyClause::GraphTraversal(gt) = &q.body_clauses[0] else {
        panic!("expected a traversal");
    };
    assert_eq!(gt.options.order, TraversalOrder::Dfs);
    assert_eq!(gt.options.unique_vertices, UniqueVertices::None);
    assert!(gt.prune.is_some());

    assert!(parse(
        r#"FOR v IN 1..2 OUTBOUND "cities/a" roads OPTIONS { uniqueVertices: "global", order: "dfs" } RETURN v"#
    )
    .is_err());
}

// ---------------------------------------------------------------------------
// 5. Inline array expressions
// ---------------------------------------------------------------------------

#[test]
fn inline_filter_limit_return() {
    let (engine, _tmp) = engine();
    let people = r#"[{n: "a", age: 10}, {n: "b", age: 20}, {n: "c", age: 30}, {n: "d", age: 40}]"#;
    let q = |body: &str| format!("LET p = {people} RETURN {body}");

    assert_eq!(
        one(&engine, &q("p[* FILTER CURRENT.age > 15].n")),
        json!(["b", "c", "d"])
    );
    assert_eq!(one(&engine, &q("p[* LIMIT 2].n")), json!(["a", "b"]));
    assert_eq!(one(&engine, &q("p[* LIMIT 1, 2].n")), json!(["b", "c"]));
    assert_eq!(
        one(&engine, &q("p[* RETURN CURRENT.age * 2]")),
        json!([20, 40, 60, 80])
    );
    assert_eq!(
        one(
            &engine,
            &q("p[* FILTER CURRENT.age >= 20 LIMIT 1, 5 RETURN UPPER(CURRENT.n)]")
        ),
        json!(["C", "D"])
    );
    // Plain [*] and [*].path keep working.
    assert_eq!(one(&engine, &q("p[*].n")), json!(["a", "b", "c", "d"]));
    // Non-array operand.
    assert_eq!(one(&engine, "RETURN null[* FILTER CURRENT > 1]"), json!([]));
    // Outer variables are visible inside.
    assert_eq!(
        one(
            &engine,
            "LET min = 2 RETURN [1, 2, 3][* FILTER CURRENT >= min]"
        ),
        json!([2, 3])
    );
    // Nested inline expressions shadow CURRENT.
    assert_eq!(
        one(
            &engine,
            "RETURN [[1, 2], [3, 4]][* RETURN CURRENT[* FILTER CURRENT > 1]]"
        ),
        json!([[2], [3, 4]])
    );
}

#[test]
fn double_star_flattens() {
    let (engine, _tmp) = engine();
    assert_eq!(
        one(&engine, "RETURN [[1, 2], [3], 4][**]"),
        json!([1, 2, 3, 4])
    );
    assert_eq!(one(&engine, "RETURN [[[1]], [2]][**]"), json!([[1], 2]));
    assert_eq!(one(&engine, "RETURN [[[1]], [2]][***]"), json!([1, 2]));
    assert_eq!(
        one(
            &engine,
            "LET posts = [{tags: ['a', 'b']}, {tags: ['c']}] RETURN posts[*].tags[**]"
        ),
        json!(["a", "b", "c"])
    );
    assert_eq!(
        one(&engine, "RETURN [[1, 2], [3]][** FILTER CURRENT != 2]"),
        json!([1, 3])
    );
}

// ---------------------------------------------------------------------------
// 6. Array comparison operators
// ---------------------------------------------------------------------------

#[test]
fn array_comparison_operators() {
    let (engine, _tmp) = engine();
    let cases = [
        ("[1, 2, 3] ANY == 2", true),
        ("[1, 2, 3] ANY == 5", false),
        ("[1, 2, 3] ALL > 0", true),
        ("[1, 2, 3] ALL > 1", false),
        ("[1, 2, 3] NONE > 5", true),
        ("[1, 2, 3] NONE >= 3", false),
        ("[1, 2, 3] ALL IN [1, 2, 3, 4]", true),
        ("[1, 2, 9] ALL IN [1, 2, 3, 4]", false),
        ("[1, 9] ANY NOT IN [1, 2]", true),
        ("[1, 2] ANY != 1", true),
        ("[1, 2] ALL <= 2", true),
        ("[1, 2] ANY < 1", false),
        ("[1, 2, 3] AT LEAST (2) >= 2", true),
        ("[1, 2, 3] AT LEAST (3) >= 2", false),
        ("[] ALL == 1", true),
        ("[] ANY == 1", false),
        ("[] NONE == 1", true),
        ("[] AT LEAST (0) == 1", true),
        ("'x' ANY == 'x'", false),
        ("[1] ANY IN 1", false),
    ];
    for (expr, expected) in cases {
        assert_eq!(
            one(&engine, &format!("RETURN {expr}")),
            json!(expected),
            "{expr}"
        );
    }
}

#[test]
fn array_comparison_in_filters_and_with_other_operators() {
    let (engine, _tmp) = engine();
    let rows = run(
        &engine,
        "FOR p IN [{t: ['a', 'b']}, {t: ['c']}, {t: []}] FILTER p.t ANY IN ['b', 'c'] RETURN p.t",
    );
    assert_eq!(rows, vec![json!(["a", "b"]), json!(["c"])]);
    // Binds tighter than AND.
    assert_eq!(
        one(&engine, "RETURN [1, 2] ALL > 0 AND [3] ANY == 3"),
        json!(true)
    );
    // The prefix quantifier still parses.
    assert_eq!(
        one(&engine, "RETURN ANY x IN [1, 2] SATISFIES x > 1"),
        json!(true)
    );
    // Words used elsewhere are untouched: a shortest path's direction.
    assert!(parse(r#"FOR v IN SHORTEST_PATH "a/1" TO "a/2" ANY edges RETURN v"#).is_ok());
}

// ---------------------------------------------------------------------------
// 7. COLLECT extensions
// ---------------------------------------------------------------------------

#[test]
fn collect_into_expression_and_method_parse() {
    let q = parse(
        "FOR u IN users COLLECT c = u.city INTO names = u.name OPTIONS { method: 'hash' } RETURN names",
    )
    .unwrap();
    let collect = q
        .body_clauses
        .iter()
        .find_map(|c| match c {
            BodyClause::Collect(c) => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(collect.into_var.as_deref(), Some("names"));
    assert!(collect.into_expr.is_some());
    assert_eq!(collect.method, Some(CollectMethod::Hash));

    // AGGREGATE before INTO (AQL order).
    assert!(parse(
        "FOR u IN users COLLECT c = u.city AGGREGATE n = COUNT() INTO g RETURN { c, n, g }"
    )
    .is_ok());
    assert!(parse("FOR u IN users COLLECT c = u.city INTO g = u KEEP u RETURN g").is_err());
    assert!(
        parse("FOR u IN users COLLECT c = u.city OPTIONS { method: 'fast' } RETURN c").is_err()
    );
}

#[test]
fn collect_into_expression_and_sorted_output() {
    let (engine, _tmp) = engine();
    let rows = run(
        &engine,
        "FOR u IN [{c: 'b', n: 1}, {c: 'a', n: 2}, {c: 'b', n: 3}, {c: 'c', n: 4}]
           COLLECT city = u.c INTO ns = u.n
           RETURN { city, ns }",
    );
    // Sorted by group value by default (AQL); INTO projects u.n.
    assert_eq!(
        rows,
        vec![
            json!({"city": "a", "ns": [2]}),
            json!({"city": "b", "ns": [1, 3]}),
            json!({"city": "c", "ns": [4]}),
        ]
    );
}

// ---------------------------------------------------------------------------
// 8. Window functions in LET
// ---------------------------------------------------------------------------

#[test]
fn window_function_in_let_after_sort() {
    let (engine, _tmp) = engine();
    let rows = run(
        &engine,
        "FOR d IN [{day: 2, r: 20}, {day: 1, r: 10}, {day: 3, r: 50}]
           SORT d.day
           LET prev = LAG(d.r) OVER (ORDER BY d.day)
           RETURN { day: d.day, change: prev != null ? d.r - prev : null }",
    );
    assert_eq!(
        rows,
        vec![
            json!({"day": 1, "change": null}),
            json!({"day": 2, "change": 10}),
            json!({"day": 3, "change": 30}),
        ]
    );

    // Windows see every sorted row even when LIMIT follows.
    let rows = run(
        &engine,
        "FOR i IN [3, 1, 2] SORT i LIMIT 1, 1
           LET rn = ROW_NUMBER() OVER (ORDER BY i)
           RETURN { i, rn }",
    );
    assert_eq!(rows, vec![json!({"i": 2, "rn": 2})]);
}

#[test]
fn window_function_in_body_let_feeds_filter() {
    let (engine, _tmp) = engine();
    let mut rows = run(
        &engine,
        "FOR o IN [{c: 'x', t: 5}, {c: 'x', t: 9}, {c: 'y', t: 1}, {c: 'x', t: 7}]
           LET rn = ROW_NUMBER() OVER (PARTITION BY o.c ORDER BY o.t DESC)
           FILTER rn <= 2
           RETURN [o.c, o.t]",
    );
    rows.sort_by_key(|v| v.to_string());
    assert_eq!(
        rows,
        vec![json!(["x", 7]), json!(["x", 9]), json!(["y", 1])]
    );
}

#[test]
fn new_window_function_names_parse() {
    for f in [
        "NTILE(4)",
        "PERCENT_RANK()",
        "CUME_DIST()",
        "NTH_VALUE(d.x, 2)",
    ] {
        let q = parse(&format!("FOR d IN docs RETURN {f} OVER (ORDER BY d.x)"))
            .unwrap_or_else(|e| panic!("{f}: {e:?}"));
        assert!(matches!(
            q.return_clause.unwrap().expression,
            Expression::WindowFunctionCall { .. }
        ));
    }
}

// ---------------------------------------------------------------------------
// Authorization classification
// ---------------------------------------------------------------------------

#[test]
fn mutation_in_post_limit_let_counts_as_write() {
    let q = parse("FOR u IN users LIMIT 1 LET x = (FOR d IN users REMOVE d IN users) RETURN x")
        .unwrap();
    assert!(q.has_mutations());
    let q = parse(r#"REPLACE { _key: "a" } IN users"#).unwrap();
    assert!(q.has_mutations());
}

// ---------------------------------------------------------------- mistakes

/// A FILTER that names a function that does not exist, or a bind variable
/// that was not sent, is a mistake in the query: it fails, where it used to
/// drop every row and answer `[]` (card #794). An error that is a fact about
/// one row still only drops that row.
#[test]
fn a_mistake_in_a_filter_fails_the_query() {
    let (engine, _tmp) = engine();
    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();
    for (k, n) in [("a", 1), ("b", 2)] {
        users.insert(json!({"_key": k, "n": n})).unwrap();
    }

    let unknown = run_err(&engine, "FOR u IN users FILTER NOPE(u.n) RETURN u._key");
    assert!(unknown.contains("Unknown function: NOPE"), "{unknown}");
    let unbound = run_err(&engine, "FOR u IN users FILTER u.n == @n RETURN u._key");
    assert!(unbound.contains("Bind variable '@n'"), "{unbound}");
    // In a subquery, and behind an index-free second FILTER, too.
    let nested = run_err(
        &engine,
        "FOR u IN users RETURN LENGTH(FOR x IN users FILTER NOPE(x.n) RETURN 1)",
    );
    assert!(nested.contains("Unknown function: NOPE"), "{nested}");
    let second = run_err(
        &engine,
        "FOR u IN users FILTER u.n > 0 FILTER u.n == @missing RETURN u._key",
    );
    assert!(second.contains("Bind variable '@missing'"), "{second}");

    // The same filters, written right, still answer.
    let mut rows = run(&engine, "FOR u IN users FILTER ABS(u.n) > 1 RETURN u._key");
    rows.sort_by_key(|v| v.to_string());
    assert_eq!(rows, vec![json!("b")]);
    // And a query that writes nothing because of the mistake wrote nothing.
    let refused = run_err(
        &engine,
        "FOR u IN users FILTER NOPE(u.n) UPDATE u WITH {n: 99} IN users",
    );
    assert!(refused.contains("Unknown function"), "{refused}");
    assert_eq!(
        run(&engine, "FOR u IN users SORT u._key RETURN u.n"),
        vec![json!(1), json!(2)]
    );
}

// ---------------------------------------------------------------- keywords

/// A keyword is a field name as it was written, in an object and after a
/// dot. Only some were accepted in an object, and each as one fixed
/// spelling: `{on: 1}` did not parse, and `{sort: 1}` made a field `order`.
#[test]
fn a_keyword_is_a_field_name_as_written() {
    let (engine, _tmp) = engine();
    assert_eq!(
        one(
            &engine,
            "RETURN {on: 1, to: 2, sort: 3, order: 4, Filter: 5, left: 6}"
        ),
        json!({"on": 1, "to": 2, "sort": 3, "order": 4, "Filter": 5, "left": 6})
    );
    assert_eq!(
        one(
            &engine,
            r#"LET d = {"order": 1, "to": 2, "sort": 3, "on": 4} RETURN [d.order, d.to, d.sort, d.on]"#
        ),
        json!([1, 2, 3, 4])
    );
    // Keywords are still keywords where a clause begins.
    assert_eq!(
        run(&engine, "FOR x IN [3, 1, 2] SORT x RETURN {sort: x}"),
        vec![json!({"sort": 1}), json!({"sort": 2}), json!({"sort": 3})]
    );
}

// --------------------------------------------------------------------- SUM

/// The sum of integers is an integer, as in SQL: `SUM([5, 10])` is `15`,
/// not `15.0`, in the function, in COLLECT and over a window. One float
/// among the values makes the sum a float.
#[test]
fn the_sum_of_integers_is_an_integer() {
    let (engine, _tmp) = engine();
    let is_int = |v: &Value| v.is_i64() || v.is_u64();

    let sum = one(&engine, "RETURN SUM([5, 10])");
    assert!(is_int(&sum) && sum == json!(15), "{sum:?}");
    let none = one(&engine, "RETURN SUM([])");
    assert!(is_int(&none) && none == json!(0), "{none:?}");
    let mixed = one(&engine, "RETURN SUM([1, 2.5])");
    assert!(mixed.is_f64() && mixed == json!(3.5), "{mixed:?}");
    let floats = one(&engine, "RETURN SUM([1.0, 2.0])");
    assert!(floats.is_f64() && floats == json!(3.0), "{floats:?}");
    // Past an i64 the integer does not fit a JSON number: a float.
    let wide = one(
        &engine,
        "RETURN SUM([9223372036854775807, 9223372036854775807, 9223372036854775807])",
    );
    assert!(wide.is_f64(), "{wide:?}");

    engine.create_collection("sales".to_string(), None).unwrap();
    let sales = engine.get_collection("sales").unwrap();
    for (k, shop, qty, price) in [("1", "a", 2, 1.5), ("2", "a", 3, 2.0), ("3", "b", 4, 0.25)] {
        sales
            .insert(json!({"_key": k, "shop": shop, "qty": qty, "price": price}))
            .unwrap();
    }
    let by_shop = run(
        &engine,
        "FOR s IN sales COLLECT shop = s.shop AGGREGATE qty = SUM(s.qty), paid = SUM(s.qty * s.price) \
         SORT shop RETURN [shop, qty, paid]",
    );
    assert_eq!(by_shop, vec![json!(["a", 5, 9.0]), json!(["b", 4, 1.0])]);
    assert!(
        is_int(&by_shop[0][1]) && by_shop[0][2].is_f64(),
        "{by_shop:?}"
    );
    let total = one(
        &engine,
        "FOR s IN sales COLLECT AGGREGATE qty = SUM(s.qty) RETURN qty",
    );
    assert!(is_int(&total) && total == json!(9), "{total:?}");
    let running = run(
        &engine,
        "FOR s IN sales SORT s._key RETURN SUM(s.qty) OVER (ORDER BY s._key)",
    );
    assert_eq!(running, vec![json!(2), json!(5), json!(9)]);
    assert!(running.iter().all(is_int), "{running:?}");
}

// ------------------------------------------------------------ vertex ids

/// A traversal takes its start as edges name it (`"people/ann"`), as the
/// document's `_id` reads (`"shop:people/ann"`) or as the document itself,
/// and finds edges written either way.
#[test]
fn a_traversal_starts_from_an_id_or_a_document() {
    let (engine, _tmp) = engine();
    engine.create_database("shop".to_string()).unwrap();
    let db = engine.get_database("shop").unwrap();
    db.create_collection("people".to_string(), None).unwrap();
    db.create_collection("follows".to_string(), Some("edge".to_string()))
        .unwrap();
    let in_shop = |q: &str| {
        let query = parse(q).unwrap_or_else(|e| panic!("parse failed for {q}: {e:?}"));
        QueryExecutor::with_database(&engine, "shop".to_string())
            .execute(&query)
            .unwrap_or_else(|e| panic!("query failed for {q}: {e:?}"))
    };
    in_shop(r#"FOR k IN ["ann", "bob", "cid", "dee"] INSERT {_key: k} INTO people"#);
    in_shop(
        r#"FOR e IN [["ann", "bob"], ["bob", "cid"]]
             INSERT {_from: CONCAT("people/", e[0]), _to: CONCAT("people/", e[1])} INTO follows"#,
    );
    // One edge written with the documents' own ids.
    in_shop(
        r#"FOR a IN people FILTER a._key == "cid" FOR b IN people FILTER b._key == "dee"
             INSERT {_from: a._id, _to: b._id} INTO follows"#,
    );
    assert_eq!(
        in_shop(r#"FOR p IN people FILTER p._key == "ann" RETURN p._id"#),
        vec![json!("shop:people/ann")]
    );

    let reached = vec![json!("bob"), json!("cid"), json!("dee")];
    for start in [
        r#""people/ann""#,
        r#""shop:people/ann""#,
        "p._id",
        "p",
        r#"CONCAT("people/", p._key)"#,
    ] {
        let q = format!(
            r#"FOR p IN people FILTER p._key == "ann"
                 FOR v IN 1..3 OUTBOUND {start} follows SORT v._key RETURN v._key"#
        );
        assert_eq!(in_shop(&q), reached, "start {start}");
    }
    assert_eq!(
        in_shop(r#"FOR v IN 1..3 INBOUND "shop:people/dee" follows SORT v._key RETURN v._key"#),
        vec![json!("ann"), json!("bob"), json!("cid")]
    );
    assert_eq!(
        in_shop(r#"FOR v IN 1..1 ANY "people/cid" follows SORT v._key RETURN v._key"#),
        vec![json!("bob"), json!("dee")]
    );
    let not_a_vertex = {
        let query = parse("FOR v IN 1..1 OUTBOUND 42 follows RETURN v").unwrap();
        QueryExecutor::with_database(&engine, "shop".to_string())
            .execute(&query)
            .expect_err("a number is no vertex")
            .to_string()
    };
    assert!(
        not_a_vertex.contains("Start vertex must be"),
        "{not_a_vertex}"
    );
}
