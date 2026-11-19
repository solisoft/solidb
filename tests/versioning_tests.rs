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

fn now_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64
}

/// Enabling versioning records the collection as it stands (card #997):
/// documents written before, and never since, are found AS OF now.
#[test]
fn test_enable_records_a_baseline_of_existing_documents() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    for i in 0..2_500 {
        coll.insert(json!({"_key": format!("k{i}"), "n": i}))
            .unwrap();
    }
    let before_enable = now_micros();
    // AS OF includes its instant: the baseline must fall after it.
    std::thread::sleep(std::time::Duration::from_millis(2));
    coll.enable_versioning().unwrap();

    let now = coll.scan_as_of(now_micros()).unwrap();
    assert_eq!(now.len(), 2_500, "every existing document, across chunks");
    assert_eq!(
        coll.get_as_of("k7", now_micros()).unwrap().unwrap()["n"],
        json!(7)
    );
    // History starts when versioning is enabled.
    assert!(
        coll.get_as_of("k7", before_enable).unwrap().is_none(),
        "before_enable={before_enable} history={:?}",
        coll.doc_history("k7")
    );

    // Enabling again adds nothing.
    coll.enable_versioning().unwrap();
    assert_eq!(coll.doc_history("k7").len(), 1);

    // A write after the baseline is the newer version.
    coll.update("k7", json!({"_key": "k7", "n": 70})).unwrap();
    assert_eq!(
        coll.get_as_of("k7", now_micros()).unwrap().unwrap()["n"],
        json!(70)
    );
}

/// Writes made while versioning was off are caught up when it is turned
/// back on: an update shows its new value, a delete reads as gone.
#[test]
fn test_reenabling_catches_up_with_writes_made_while_off() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    coll.enable_versioning().unwrap();
    coll.insert(json!({"_key": "a", "n": 1})).unwrap();
    coll.insert(json!({"_key": "b", "n": 1})).unwrap();
    // `a:b` shares `a`'s history prefix; its versions interleave with `a`'s.
    coll.insert(json!({"_key": "a:b", "n": 1})).unwrap();

    coll.disable_versioning().unwrap();
    coll.update("a", json!({"_key": "a", "n": 2})).unwrap();
    coll.delete("b").unwrap();
    coll.enable_versioning().unwrap();

    let now = now_micros();
    assert_eq!(coll.get_as_of("a", now).unwrap().unwrap()["n"], json!(2));
    assert!(
        coll.get_as_of("b", now).unwrap().is_none(),
        "deleted while off"
    );
    assert_eq!(coll.get_as_of("a:b", now).unwrap().unwrap()["n"], json!(1));
    let mut keys: Vec<String> = coll
        .scan_as_of(now)
        .unwrap()
        .iter()
        .map(|d| d["_key"].as_str().unwrap().to_string())
        .collect();
    keys.sort();
    assert_eq!(keys, vec!["a", "a:b"]);
}

/// insert_batch caps the history of a key it writes again after a delete,
/// like a single insert does (card #995).
#[test]
fn test_insert_batch_respects_the_retention_cap() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    coll.enable_versioning().unwrap();

    for i in 0..60 {
        coll.insert_batch(vec![json!({"_key": "a", "n": i})])
            .unwrap();
        coll.delete("a").unwrap();
    }
    coll.insert_batch(vec![json!({"_key": "a", "n": 60})])
        .unwrap();

    // 121 versions written (60 inserts, 60 tombstones, the last insert).
    let hist = coll.doc_history("a");
    assert_eq!(hist.len(), 100);
    assert_eq!(hist[0]["value"]["n"], json!(60)); // newest retained
}

#[test]
fn test_insert_and_upsert_batch_record_history() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    coll.enable_versioning().unwrap();

    coll.insert_batch(vec![
        json!({"_key": "a", "n": 1}),
        json!({"_key": "b", "n": 1}),
    ])
    .unwrap();
    assert_eq!(coll.doc_history("a").len(), 1);
    assert_eq!(coll.doc_history("b").len(), 1);

    coll.upsert_batch(vec![("a".into(), json!({"_key": "a", "n": 2}))])
        .unwrap();
    let hist = coll.doc_history("a");
    assert_eq!(hist.len(), 2);
    assert_eq!(hist[0]["value"]["n"], json!(2));
}

fn keys_of(coll: &solidb::storage::Collection) -> Vec<String> {
    let mut keys: Vec<String> = coll
        .scan_values(None)
        .iter()
        .map(|d| d["_key"].as_str().unwrap().to_string())
        .collect();
    keys.sort();
    keys
}

/// Restore to an instant (card #1001): changed documents go back, deleted
/// ones return, later ones go; the restore is itself history, so restoring
/// to just before it undoes it. With `keys`, only those are touched.
#[test]
fn test_restore_as_of_and_undo() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    coll.enable_versioning().unwrap();
    coll.insert(json!({"_key": "a", "n": 1})).unwrap();
    coll.insert(json!({"_key": "b", "n": 1})).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let t1 = now_micros();
    std::thread::sleep(std::time::Duration::from_millis(2));
    coll.update("a", json!({"_key": "a", "n": 2})).unwrap();
    coll.delete("b").unwrap();
    coll.insert(json!({"_key": "c", "n": 1})).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let before_restore = now_micros();
    std::thread::sleep(std::time::Duration::from_millis(2));

    // Only `a`.
    let partial = coll.restore_as_of(t1, Some(&["a".to_string()])).unwrap();
    assert_eq!(partial.written.len(), 1);
    assert_eq!(coll.get("a").unwrap().to_value()["n"], json!(1));
    assert_eq!(keys_of(&coll), vec!["a", "c"]);

    // Everything.
    let all = coll.restore_as_of(t1, None).unwrap();
    assert_eq!(all.written.len(), 1, "b comes back, a already matches");
    assert_eq!(all.removed, vec!["c".to_string()]);
    assert_eq!(keys_of(&coll), vec!["a", "b"]);
    assert_eq!(coll.get("b").unwrap().to_value()["n"], json!(1));

    // Undone: back to the instant before the restores.
    coll.restore_as_of(before_restore, None).unwrap();
    assert_eq!(keys_of(&coll), vec!["a", "c"]);
    assert_eq!(coll.get("a").unwrap().to_value()["n"], json!(2));

    // Nothing to restore from without versioning.
    engine.create_collection("plain".to_string(), None).unwrap();
    let plain = engine.get_collection("plain").unwrap();
    assert!(plain.restore_as_of(t1, None).is_err());
}

/// A named instant reads to the microsecond through VERSION_TAG, and
/// keeps the version it needs past the retention cap.
#[test]
fn test_version_tags_read_and_survive_pruning() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    coll.enable_versioning().unwrap();
    coll.insert(json!({"_key": "a", "n": 0})).unwrap();
    let tagged = coll.create_version_tag("v0", None).unwrap();
    // Written in the same millisecond as the tag, possibly: the tag still
    // reads the version before it.
    coll.update("a", json!({"_key": "a", "n": 1})).unwrap();
    assert_eq!(coll.version_tags(), vec![("v0".to_string(), tagged)]);

    let q = r#"FOR d IN docs SYSTEM_TIME AS OF VERSION_TAG("docs", "v0") RETURN d.n"#;
    assert_eq!(execute_query(&engine, q), vec![json!(0)]);
    let q = r#"RETURN DOC_AS_OF("docs", "a", VERSION_TAG("docs", "v0")).n"#;
    assert_eq!(execute_query(&engine, q), vec![json!(0)]);

    for i in 2..=120 {
        coll.update("a", json!({"_key": "a", "n": i})).unwrap();
    }
    let hist = coll.doc_history("a");
    assert_eq!(hist.len(), 101, "the cap, and the tagged version");
    assert_eq!(coll.get_as_of("a", tagged).unwrap().unwrap()["n"], json!(0));

    // Without the tag the cap applies again at the next write.
    assert!(coll.delete_version_tag("v0").unwrap());
    assert!(!coll.delete_version_tag("v0").unwrap());
    coll.update("a", json!({"_key": "a", "n": 121})).unwrap();
    assert_eq!(coll.doc_history("a").len(), 100);

    assert!(coll.create_version_tag("  ", None).is_err());
}

/// Truncating a versioned collection keeps its history (card #1001): the
/// documents are gone now, still there AS OF before, and a restore brings
/// them back.
#[test]
fn test_truncate_keeps_history_of_a_versioned_collection() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let coll = engine.get_collection("docs").unwrap();
    coll.enable_versioning().unwrap();
    for i in 0..3 {
        coll.insert(json!({"_key": format!("k{i}"), "n": i}))
            .unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(2));
    let before = now_micros();
    std::thread::sleep(std::time::Duration::from_millis(2));
    coll.truncate().unwrap();
    assert_eq!(coll.count(), 0);
    assert!(coll.scan_as_of(now_micros()).unwrap().is_empty());
    assert_eq!(coll.scan_as_of(before).unwrap().len(), 3);
    coll.restore_as_of(before, None).unwrap();
    assert_eq!(coll.count(), 3);
}
