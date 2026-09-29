//! `FILTER doc._id == ...` is answered as a primary-key lookup; results must
//! match what a scan would return.

mod common;

use common::{create_test_engine, execute_query};
use serde_json::json;

fn keys(engine: &solidb::StorageEngine, q: &str) -> Vec<String> {
    let mut k: Vec<String> = execute_query(engine, q)
        .into_iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    k.sort();
    k
}

#[test]
fn id_equality_and_in_match_a_scan() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("c".to_string(), None).unwrap();
    engine.create_collection("other".to_string(), None).unwrap();
    let c = engine.get_collection("c").unwrap();
    for k in ["a", "b", "c1"] {
        c.insert(json!({"_key": k})).unwrap();
    }
    engine
        .get_collection("other")
        .unwrap()
        .insert(json!({"_key": "a"}))
        .unwrap();

    assert_eq!(
        keys(&engine, r#"FOR d IN c FILTER d._id == "c/a" RETURN d._key"#),
        ["a"]
    );
    assert_eq!(
        keys(
            &engine,
            r#"FOR d IN c FILTER d._id IN ["c/a", "c/c1", "c/a", "c/zzz"] RETURN d._key"#
        ),
        ["a", "c1"]
    );
    // Another collection's _id, a missing key and a non-string match nothing.
    assert!(keys(
        &engine,
        r#"FOR d IN c FILTER d._id == "other/a" RETURN d._key"#
    )
    .is_empty());
    assert!(keys(
        &engine,
        r#"FOR d IN c FILTER d._id == "c/nope" RETURN d._key"#
    )
    .is_empty());
    assert!(keys(&engine, "FOR d IN c FILTER d._id == 5 RETURN d._key").is_empty());
    // Other operators still scan.
    assert_eq!(
        keys(&engine, r#"FOR d IN c FILTER d._id != "c/a" RETURN d._key"#),
        ["b", "c1"]
    );
}
