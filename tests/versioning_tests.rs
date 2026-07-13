//! Document versioning / time-travel (`AS OF`) tests.

mod common;

use common::{create_test_engine, execute_query};
use serde_json::json;

#[test]
fn test_versioning_disabled_by_default() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();

    assert!(!coll.is_versioned());
    coll.insert(json!({"_key": "a", "v": 1})).unwrap();
    coll.update("a", json!({"_key": "a", "v": 2})).unwrap();

    // No history recorded, and AS OF finds nothing.
    assert!(coll.doc_history("a").is_empty());
    assert_eq!(coll.get_as_of("a", u64::MAX).unwrap(), None);
}

#[test]
fn test_versioning_records_history_and_as_of() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    coll.enable_versioning().unwrap();
    assert!(coll.is_versioned());

    coll.insert(json!({"_key": "a", "v": 1})).unwrap();
    coll.update("a", json!({"_key": "a", "v": 2})).unwrap();
    coll.update("a", json!({"_key": "a", "v": 3})).unwrap();

    let hist = coll.doc_history("a");
    assert_eq!(hist.len(), 3);
    // Newest first.
    assert_eq!(hist[0]["value"]["v"], json!(3));
    assert_eq!(hist[1]["value"]["v"], json!(2));
    assert_eq!(hist[2]["value"]["v"], json!(1));

    let ts_v3 = hist[0]["ts_micros"].as_u64().unwrap();
    let ts_v2 = hist[1]["ts_micros"].as_u64().unwrap();
    let ts_v1 = hist[2]["ts_micros"].as_u64().unwrap();

    // Reading exactly at each version's timestamp returns that version.
    assert_eq!(coll.get_as_of("a", ts_v3).unwrap().unwrap()["v"], json!(3));
    assert_eq!(coll.get_as_of("a", ts_v2).unwrap().unwrap()["v"], json!(2));
    assert_eq!(coll.get_as_of("a", ts_v1).unwrap().unwrap()["v"], json!(1));

    // Just before v2 → v1; just before v1 → nothing (didn't exist yet).
    assert_eq!(
        coll.get_as_of("a", ts_v2 - 1).unwrap().unwrap()["v"],
        json!(1)
    );
    assert_eq!(coll.get_as_of("a", ts_v1 - 1).unwrap(), None);

    // Delete records a tombstone; latest AS OF is gone, history is preserved.
    coll.delete("a").unwrap();
    let hist = coll.doc_history("a");
    assert_eq!(hist.len(), 4);
    assert_eq!(hist[0]["deleted"], json!(true));
    assert_eq!(coll.get_as_of("a", u64::MAX).unwrap(), None);
    assert_eq!(coll.get_as_of("a", ts_v3).unwrap().unwrap()["v"], json!(3));
}

#[test]
fn test_doc_as_of_and_history_via_sdbql() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    coll.enable_versioning().unwrap();
    coll.insert(json!({"_key": "a", "v": 1})).unwrap();
    coll.update("a", json!({"_key": "a", "v": 2})).unwrap();

    let res = execute_query(&engine, r#"RETURN DOC_HISTORY("docs", "a")"#);
    assert_eq!(res[0].as_array().unwrap().len(), 2);

    // Far-future timestamp → latest version.
    let res = execute_query(&engine, r#"RETURN DOC_AS_OF("docs", "a", 99999999999999)"#);
    assert_eq!(res[0]["v"], json!(2));

    // Epoch 0 → before creation → null.
    let res = execute_query(&engine, r#"RETURN DOC_AS_OF("docs", "a", 0)"#);
    assert_eq!(res[0], json!(null));
}

#[test]
fn test_version_retention_cap() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    coll.enable_versioning().unwrap();

    coll.insert(json!({"_key": "a", "n": 0})).unwrap();
    for i in 1..=120 {
        coll.update("a", json!({"_key": "a", "n": i})).unwrap();
    }

    // Default cap is 100 newest versions (SOLIDB_MAX_VERSIONS unset in CI).
    let hist = coll.doc_history("a");
    assert_eq!(hist.len(), 100);
    assert_eq!(hist[0]["value"]["n"], json!(120)); // newest retained
}
