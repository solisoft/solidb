//! UPSERT by the value of a unique index, and by key, under concurrent
//! writers: one document, and no update computed from `OLD` lost.

use serde_json::{json, Value};
use solidb::sdbql::QueryExecutor;
use solidb::storage::{IndexType, StorageEngine};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const DB: &str = "app";

fn setup() -> (Arc<StorageEngine>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
    engine.initialize().unwrap();
    engine.create_database(DB.to_string()).unwrap();
    let db = engine.get_database(DB).unwrap();
    db.create_collection("users".to_string(), None).unwrap();
    let users = db.get_collection("users").unwrap();
    users
        .create_index(
            "by_email".to_string(),
            vec!["email".to_string()],
            IndexType::Hash,
            true,
        )
        .unwrap();
    users
        .create_index(
            "by_tenant_login".to_string(),
            vec!["tenant".to_string(), "login".to_string()],
            IndexType::Persistent,
            true,
        )
        .unwrap();
    (engine, tmp)
}

fn run(engine: &StorageEngine, query: &str, vars: Value) -> Result<Vec<Value>, String> {
    let vars: HashMap<String, Value> = vars
        .as_object()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let ast = solidb::sdbql::parse(query).map_err(|e| e.to_string())?;
    QueryExecutor::with_database_and_bind_vars(engine, DB.to_string(), vars)
        .execute(&ast)
        .map_err(|e| e.to_string())
}

const BY_EMAIL: &str = "UPSERT {email: @e} INSERT {email: @e, n: 1} \
     UPDATE {n: OLD.n + 1} IN users RETURN {n: NEW.n, was: OLD.n, key: NEW._key}";

#[test]
fn a_search_on_a_unique_index_finds_its_document() {
    let (engine, _tmp) = setup();
    let vars = json!({"e": "ann@example.com"});

    let first = run(&engine, BY_EMAIL, vars.clone()).unwrap();
    assert_eq!(first[0]["n"], json!(1));
    assert_eq!(first[0]["was"], json!(null));
    let key = first[0]["key"].clone();

    // The second run updates that document, from its `OLD`.
    let second = run(&engine, BY_EMAIL, vars.clone()).unwrap();
    assert_eq!(second[0], json!({"n": 2, "was": 1, "key": key}));
    let third = run(&engine, BY_EMAIL, vars).unwrap();
    assert_eq!(third[0]["n"], json!(3));

    // Another value is another document.
    run(&engine, BY_EMAIL, json!({"e": "bob@example.com"})).unwrap();
    let all = run(
        &engine,
        "FOR u IN users SORT u.email RETURN [u.email, u.n]",
        json!({}),
    )
    .unwrap();
    assert_eq!(
        all,
        [json!(["ann@example.com", 3]), json!(["bob@example.com", 1])]
    );
}

#[test]
fn a_unique_index_over_two_fields() {
    let (engine, _tmp) = setup();
    let q = "UPSERT {login: @l, tenant: @t} INSERT {tenant: @t, login: @l, seen: 1} \
             UPDATE {seen: OLD.seen + 1} IN users RETURN NEW.seen";
    for expected in 1..=3 {
        let seen = run(&engine, q, json!({"t": "acme", "l": "ann"})).unwrap();
        assert_eq!(seen, [json!(expected)]);
    }
    // The same login under another tenant is another document.
    assert_eq!(
        run(&engine, q, json!({"t": "globex", "l": "ann"})).unwrap(),
        [json!(1)]
    );
    let count = run(&engine, "RETURN LENGTH(FOR u IN users RETURN 1)", json!({})).unwrap();
    assert_eq!(count, [json!(2)]);
}

#[test]
fn a_search_no_unique_index_serves_inserts_as_before() {
    let (engine, _tmp) = setup();
    // `city` has no unique index: nothing says which document is meant, and
    // an UPSERT never scans for one.
    let q = "UPSERT {city: 'Lyon'} INSERT {city: 'Lyon'} UPDATE {seen: true} IN users";
    run(&engine, q, json!({})).unwrap();
    run(&engine, q, json!({})).unwrap();
    let count = run(&engine, "RETURN LENGTH(FOR u IN users RETURN 1)", json!({})).unwrap();
    assert_eq!(count, [json!(2)]);
    // Part of a composite index is not that index.
    let partial = "UPSERT {tenant: 'acme'} INSERT {tenant: 'acme', login: 'x'} \
                   UPDATE {seen: true} IN users RETURN NEW.seen";
    assert_eq!(run(&engine, partial, json!({})).unwrap(), [json!(null)]);
}

#[test]
fn old_is_the_document_found_by_key_too() {
    let (engine, _tmp) = setup();
    let q =
        "UPSERT {_key: 'c'} INSERT {_key: 'c', n: 1} UPDATE {n: OLD.n + 1} IN users RETURN NEW.n";
    assert_eq!(run(&engine, q, json!({})).unwrap(), [json!(1)]);
    assert_eq!(run(&engine, q, json!({})).unwrap(), [json!(2)]);
}

/// Several threads run the same UPSERT. Each run either creates the
/// document or adds one to its counter: one document at the end, and a
/// counter equal to the number of runs.
fn hammer(query: &'static str, vars: Value, find: &str) {
    const THREADS: usize = 8;
    const RUNS: usize = 60;
    let (engine, _tmp) = setup();
    let workers: Vec<_> = (0..THREADS)
        .map(|_| {
            let (engine, vars) = (engine.clone(), vars.clone());
            std::thread::spawn(move || {
                for _ in 0..RUNS {
                    run(&engine, query, vars.clone())
                        .expect("an UPSERT lost a race it should retry");
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let rows = run(&engine, find, json!({})).unwrap();
    assert_eq!(
        rows,
        [json!(THREADS * RUNS)],
        "one document, no update lost"
    );
}

#[test]
fn concurrent_upserts_by_unique_value_lose_nothing() {
    hammer(
        BY_EMAIL,
        json!({"e": "ann@example.com"}),
        "FOR u IN users RETURN u.n",
    );
}

#[test]
fn concurrent_upserts_by_key_lose_nothing() {
    hammer(
        "UPSERT {_key: 'c'} INSERT {_key: 'c', n: 1} UPDATE {n: OLD.n + 1} IN users",
        json!({}),
        "FOR u IN users RETURN u.n",
    );
}
