//! A statement that writes several documents writes all of them or none
//! (card #846), whatever its size.

use serde_json::{json, Value};
use solidb::sdbql::QueryExecutor;
use solidb::storage::schema::{CollectionSchema, SchemaValidationMode};
use solidb::storage::{IndexType, StorageEngine};
use std::collections::HashMap;
use tempfile::TempDir;

const DB: &str = "app";

fn setup() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database(DB.to_string()).unwrap();
    let db = engine.get_database(DB).unwrap();
    db.create_collection("items".to_string(), None).unwrap();
    let items = db.get_collection("items").unwrap();
    items
        .create_index(
            "by_sku".to_string(),
            vec!["sku".to_string()],
            IndexType::Hash,
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

fn keys(engine: &StorageEngine) -> Vec<Value> {
    run(
        engine,
        "FOR i IN items SORT i._key RETURN i._key",
        json!({}),
    )
    .unwrap()
}

/// Both sizes the executor used to treat differently: up to 100 rows went
/// row by row, more went as a batch.
const SIZES: [usize; 2] = [3, 150];

#[test]
fn an_insert_that_fails_on_one_row_inserts_none() {
    for size in SIZES {
        let (engine, _tmp) = setup();
        run(
            &engine,
            "INSERT {_key: 'taken', sku: 'S-taken'} INTO items",
            json!({}),
        )
        .unwrap();

        // The last row's key exists.
        let mut docs: Vec<Value> = (0..size)
            .map(|i| json!({"_key": format!("k{i}")}))
            .collect();
        docs.push(json!({"_key": "taken"}));
        let failed = run(
            &engine,
            "FOR d IN @docs INSERT d INTO items",
            json!({"docs": docs}),
        );
        assert!(
            failed.unwrap_err().contains("already exists"),
            "size {size}"
        );
        assert_eq!(keys(&engine), [json!("taken")], "size {size}");

        // The last row repeats a unique value.
        let mut docs: Vec<Value> = (0..size).map(|i| json!({"sku": format!("S{i}")})).collect();
        docs.push(json!({"sku": "S-taken"}));
        let failed = run(
            &engine,
            "FOR d IN @docs INSERT d INTO items",
            json!({"docs": docs}),
        );
        assert!(
            failed.unwrap_err().contains("Unique constraint"),
            "size {size}"
        );
        assert_eq!(keys(&engine).len(), 1, "size {size}");

        // And the statement that is valid inserts everything, with NEW.
        let docs: Vec<Value> = (0..size)
            .map(|i| json!({"_key": format!("k{i:03}"), "n": i}))
            .collect();
        let new = run(
            &engine,
            "FOR d IN @docs INSERT d INTO items RETURN NEW.n",
            json!({"docs": docs}),
        )
        .unwrap();
        assert_eq!(
            new,
            (0..size).map(|i| json!(i)).collect::<Vec<_>>(),
            "size {size}"
        );
        assert_eq!(keys(&engine).len(), size + 1);
    }
}

#[test]
fn an_update_that_fails_on_one_row_updates_none() {
    for size in SIZES {
        let (engine, _tmp) = setup();
        let docs: Vec<Value> = (0..size)
            .map(|i| json!({"_key": format!("k{i:03}"), "n": i}))
            .collect();
        run(
            &engine,
            "FOR d IN @docs INSERT d INTO items",
            json!({"docs": docs}),
        )
        .unwrap();
        let items = engine
            .get_database(DB)
            .unwrap()
            .get_collection("items")
            .unwrap();
        items
            .set_json_schema(CollectionSchema::new(
                "default".to_string(),
                json!({"type": "object", "properties": {"n": {"type": "number", "maximum": 1000}}}),
                SchemaValidationMode::Strict,
            ))
            .unwrap();

        // The last document's new value breaks the schema.
        let last = format!("k{:03}", size - 1);
        let failed = run(
            &engine,
            "FOR i IN items UPDATE i WITH {n: i._key == @last ? 5000 : i.n + 1000} IN items",
            json!({"last": last}),
        );
        assert!(
            failed.unwrap_err().contains("Schema validation failed"),
            "size {size}"
        );
        let untouched = run(
            &engine,
            "FOR i IN items FILTER i.n >= 1000 RETURN 1",
            json!({}),
        )
        .unwrap();
        assert!(
            untouched.is_empty(),
            "size {size}: {} rows were updated",
            untouched.len()
        );

        // A unique value two rows would share.
        let failed = run(
            &engine,
            "FOR i IN items UPDATE i WITH {sku: 'same'} IN items",
            json!({}),
        );
        assert!(
            failed.unwrap_err().contains("Unique constraint"),
            "size {size}"
        );
        let with_sku = run(
            &engine,
            "FOR i IN items FILTER i.sku != null RETURN 1",
            json!({}),
        )
        .unwrap();
        assert!(with_sku.is_empty(), "size {size}");

        // A key that names no document.
        let failed = run(
            &engine,
            "FOR k IN @keys UPDATE k WITH {seen: true} IN items",
            json!({"keys": ["k000", "k001", "nope"]}),
        );
        assert!(failed.is_err(), "size {size}");
        let seen = run(
            &engine,
            "FOR i IN items FILTER i.seen == true RETURN 1",
            json!({}),
        )
        .unwrap();
        assert!(seen.is_empty(), "size {size}");

        // The valid statement updates every row.
        run(
            &engine,
            "FOR i IN items UPDATE i WITH {n: i.n + 1} IN items",
            json!({}),
        )
        .unwrap();
        let total = run(&engine, "RETURN SUM(FOR i IN items RETURN i.n)", json!({})).unwrap();
        let expected: usize = (0..size).map(|i| i + 1).sum();
        assert_eq!(total, [json!(expected)], "size {size}");
    }
}

#[test]
fn a_remove_that_names_a_missing_document_removes_none() {
    for size in SIZES {
        let (engine, _tmp) = setup();
        let docs: Vec<Value> = (0..size)
            .map(|i| json!({"_key": format!("k{i:03}")}))
            .collect();
        run(
            &engine,
            "FOR d IN @docs INSERT d INTO items",
            json!({"docs": docs}),
        )
        .unwrap();

        let mut named: Vec<Value> = (0..size).map(|i| json!(format!("k{i:03}"))).collect();
        named.push(json!("nope"));
        let failed = run(
            &engine,
            "FOR k IN @keys REMOVE k IN items",
            json!({"keys": named}),
        );
        assert!(failed.unwrap_err().contains("nope"), "size {size}");
        assert_eq!(keys(&engine).len(), size, "size {size}");

        named.pop();
        run(
            &engine,
            "FOR k IN @keys REMOVE k IN items",
            json!({"keys": named}),
        )
        .unwrap();
        assert!(keys(&engine).is_empty(), "size {size}");
    }
}

/// Several rows that update one document build on each other at any size:
/// past 100 rows they used to all write the value computed from the
/// version the statement first read.
#[test]
fn rows_for_one_document_build_on_each_other_in_a_large_statement() {
    let (engine, _tmp) = setup();
    run(&engine, "INSERT {_key: 'x', n: 0} INTO items", json!({})).unwrap();
    run(
        &engine,
        "FOR i IN 1..150 FOR d IN items FILTER d._key == 'x' UPDATE d WITH {n: d.n + 1} IN items",
        json!({}),
    )
    .unwrap();
    let n = run(&engine, "RETURN DOCUMENT('items/x').n", json!({})).unwrap();
    assert_eq!(n, [json!(150)]);
}

/// `ignoreErrors` still decides row by row: that statement is not one batch.
#[test]
fn ignore_errors_still_skips_the_rows_that_fail() {
    let (engine, _tmp) = setup();
    run(&engine, "INSERT {_key: 'taken'} INTO items", json!({})).unwrap();
    run(
        &engine,
        "FOR k IN ['a', 'taken', 'b'] INSERT {_key: k} INTO items OPTIONS {ignoreErrors: true}",
        json!({}),
    )
    .unwrap();
    assert_eq!(keys(&engine), [json!("a"), json!("b"), json!("taken")]);
}
