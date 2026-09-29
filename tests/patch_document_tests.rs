//! `Collection::patch_document`: RFC 6902 patches applied to a stored document
//! (the server side of delta sync).

mod common;

use common::create_test_engine;
use serde_json::json;
use solidb::sync::delta::JsonPatch;

fn patch(v: serde_json::Value) -> JsonPatch {
    serde_json::from_value(v).expect("valid patch")
}

#[test]
fn patch_replaces_and_removes_fields() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    let before = coll
        .insert(json!({"_key": "a", "name": "x", "tmp": 1, "n": 1}))
        .unwrap();

    let after = coll
        .patch_document(
            "a",
            &patch(json!([
                {"op": "replace", "path": "/name", "value": "y"},
                {"op": "remove", "path": "/tmp"},
                {"op": "add", "path": "/extra", "value": true}
            ])),
        )
        .unwrap();

    let v = after.to_value();
    assert_eq!(v["name"], "y");
    assert_eq!(v["n"], 1);
    assert_eq!(v["extra"], true);
    assert!(v.get("tmp").is_none(), "a patch can remove a field: {}", v);
    assert_eq!(v["_key"], "a");
    assert_ne!(after.revision(), before.revision());
    assert_eq!(after.created_at, before.created_at);
    // What was stored matches what was returned.
    assert_eq!(coll.get("a").unwrap().to_value(), v);
}

#[test]
fn failed_patch_changes_nothing() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    coll.insert(json!({"_key": "a", "n": 1})).unwrap();

    let bad = patch(json!([
        {"op": "replace", "path": "/n", "value": 2},
        {"op": "remove", "path": "/does/not/exist"}
    ]));
    assert!(coll.patch_document("a", &bad).is_err());
    assert_eq!(coll.get("a").unwrap().to_value()["n"], 1);
}

#[test]
fn patch_without_a_base_document_is_not_found() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    let p = patch(json!([{"op": "add", "path": "/n", "value": 1}]));
    assert!(matches!(
        coll.patch_document("missing", &p),
        Err(solidb::DbError::DocumentNotFound(_))
    ));
}

#[test]
fn a_patch_cannot_rename_the_document_or_forge_server_fields() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    let before = coll.insert(json!({"_key": "a", "n": 1})).unwrap();

    let after = coll
        .patch_document(
            "a",
            &patch(json!([
                {"op": "replace", "path": "/_key", "value": "hijacked"},
                {"op": "replace", "path": "/_id", "value": "other/a"},
                {"op": "replace", "path": "/_created_at", "value": "1999-01-01T00:00:00Z"},
                {"op": "add", "path": "/n2", "value": 2}
            ])),
        )
        .unwrap();

    let v = after.to_value();
    assert_eq!(v["_key"], "a", "the key is the document's identity");
    assert_ne!(v["_id"], "other/a");
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(v["n2"], 2, "the legitimate part of the patch still applies");
    assert!(coll.get("hijacked").is_err());
}

#[test]
fn concurrent_patches_of_one_document_lose_no_update() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    coll.insert(json!({"_key": "a"})).unwrap();

    let threads = 8;
    std::thread::scope(|scope| {
        for t in 0..threads {
            let coll = coll.clone();
            scope.spawn(move || {
                let p = patch(json!([{"op": "add", "path": format!("/f{}", t), "value": t}]));
                coll.patch_document("a", &p).unwrap();
            });
        }
    });

    let v = coll.get("a").unwrap().to_value();
    for t in 0..threads {
        assert_eq!(v[format!("f{}", t)], t, "patch {} was lost: {}", t, v);
    }
}
