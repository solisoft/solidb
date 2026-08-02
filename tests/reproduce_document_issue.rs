//! `DOCUMENT("db:collection/key")` resolution across database boundaries.
//!
//! This file previously asserted that a qualified id resolved from *any*
//! executor — including one scoped to a different database. That is SEC-178:
//! collections are column families named `"{db}:{collection}"`, so accepting a
//! caller-supplied qualified name let a query opened against one database read
//! every other database on the instance, `_system:_admins` included, while
//! per-database authorization only ever saw the `{db}` in the URL.
//!
//! The expectations below are inverted accordingly: the qualified form is now
//! confined to the executor's own database.

use serde_json::json;
use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (engine, tmp_dir)
}

#[test]
fn test_document_qualified_id_is_confined_to_its_database() {
    let (engine, _tmp) = create_test_engine();

    engine.create_database("db1".to_string()).unwrap();
    let db1 = engine.get_database("db1").unwrap();
    db1.create_collection("col1".to_string(), None).unwrap();
    let col1 = db1.get_collection("col1").unwrap();
    col1.insert(json!({"_key": "k1", "value": "v1"})).unwrap();

    engine.create_database("db2".to_string()).unwrap();

    let query = parse("RETURN DOCUMENT('db1:col1/k1')").unwrap();

    // Inside its own database, the qualified form still resolves — the fix
    // removed the boundary crossing, not the syntax.
    let executor_db1 = QueryExecutor::with_database(&engine, "db1".to_string());
    let result_db1 = executor_db1.execute(&query).unwrap();
    assert_eq!(
        result_db1[0]["value"],
        json!("v1"),
        "a database must still reach its own collections by qualified id"
    );

    // From another database it must not resolve. This is the vulnerability:
    // `db2` has no permission to `db1`, and nothing re-checks it here.
    let executor_db2 = QueryExecutor::with_database(&engine, "db2".to_string());
    let result_db2 = executor_db2.execute(&query);
    assert!(
        result_db2.is_err() || result_db2.as_ref().unwrap()[0] == json!(null),
        "db2 reached into db1: {:?}",
        result_db2
    );
    if let Ok(rows) = &result_db2 {
        assert_ne!(
            rows[0]["value"],
            json!("v1"),
            "db1 document leaked into db2"
        );
    }

    // An executor with no database context has nothing to authorize against,
    // so the qualified form is unusable there too. No server path builds one —
    // every handler uses `with_database` — so this only pins the invariant.
    let executor_global = QueryExecutor::new(&engine);
    let result_global = executor_global.execute(&query);
    assert!(
        result_global.is_err() || result_global.as_ref().unwrap()[0] == json!(null),
        "context-free executor resolved a qualified id: {:?}",
        result_global
    );
}

#[test]
fn test_document_qualified_id_cannot_reach_system_collections() {
    let (engine, _tmp) = create_test_engine();
    engine.initialize().expect("initialize _system");
    engine.create_database("tenant".to_string()).unwrap();

    let executor = QueryExecutor::with_database(&engine, "tenant".to_string());
    for query_str in [
        "RETURN DOCUMENT('_system:_admins/admin')",
        "RETURN DOCUMENT('_system:_api_keys/any')",
    ] {
        let query = parse(query_str).unwrap();
        let result = executor.execute(&query);
        let rendered = format!("{:?}", result);
        assert!(
            !rendered.contains("password_hash") && !rendered.contains("argon2"),
            "{} exposed credentials: {}",
            query_str,
            rendered
        );
    }
}
