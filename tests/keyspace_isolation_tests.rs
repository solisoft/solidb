//! Shared-layout collections live side by side in one column family, each
//! under an eight-byte key prefix. These tests put collections and databases
//! next to each other and check that nothing — full scans, index ranges in
//! both directions, fulltext, truncate, drops — reaches across a boundary,
//! and that creating or dropping never touches the column-family map.

use serde_json::json;
use solidb::storage::cf_ops;
use solidb::storage::{IndexType, StorageEngine};
use tempfile::TempDir;

fn engine() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    (engine, tmp)
}

fn fill(engine: &StorageEngine, db: &str, coll: &str, n: usize, tag: &str) {
    let database = engine.get_database(db).unwrap();
    database.create_collection(coll.to_string(), None).unwrap();
    let c = database.get_collection(coll).unwrap();
    for i in 0..n {
        c.insert(json!({"_key": format!("k{:03}", i), "tag": tag, "n": i, "text": format!("word{} {}", i, tag)}))
            .unwrap();
    }
}

#[test]
fn neighbouring_collections_never_see_each_other() {
    let (engine, _tmp) = engine();
    engine.create_database("shop".into()).unwrap();
    fill(&engine, "shop", "a", 20, "alpha");
    fill(&engine, "shop", "b", 7, "beta");
    fill(&engine, "shop", "c", 13, "gamma");

    let db = engine.get_database("shop").unwrap();
    let a = db.get_collection("a").unwrap();
    let b = db.get_collection("b").unwrap();
    let c = db.get_collection("c").unwrap();

    assert_eq!(a.all().len(), 20);
    assert_eq!(b.all().len(), 7);
    assert_eq!(c.all().len(), 13);
    assert!(b.all().iter().all(|d| d.to_value()["tag"] == "beta"));
    assert_eq!(b.count(), 7);
    assert_eq!(b.recalculate_count(), 7);

    // Index ranges, both directions, only ever return this collection's rows.
    b.create_index(
        "by_n".into(),
        vec!["n".into()],
        IndexType::Persistent,
        false,
    )
    .unwrap();
    let up = b.index_lookup_gte("n", &json!(0), None).unwrap_or_default();
    assert_eq!(up.len(), 7);
    let down = b
        .index_lookup_lte("n", &json!(1000), None)
        .unwrap_or_default();
    assert_eq!(down.len(), 7);
    let sorted = b.index_sorted("n", false, Some(100)).unwrap_or_default();
    assert_eq!(sorted.len(), 7);
    assert!(sorted.iter().all(|d| d.to_value()["tag"] == "beta"));

    // Fulltext over the middle collection finds only its own words.
    b.create_fulltext_index("ft".into(), vec!["text".into()], Some(3))
        .unwrap();
    assert_eq!(b.fulltext_search("beta", None, 100).unwrap().len(), 7);
    assert!(b.fulltext_search("alpha", None, 100).unwrap().is_empty());
}

#[test]
fn truncate_and_drop_leave_neighbours_intact() {
    let (engine, _tmp) = engine();
    engine.create_database("shop".into()).unwrap();
    fill(&engine, "shop", "a", 5, "alpha");
    fill(&engine, "shop", "b", 5, "beta");
    fill(&engine, "shop", "c", 5, "gamma");
    let db = engine.get_database("shop").unwrap();
    let b = db.get_collection("b").unwrap();
    b.create_index(
        "by_n".into(),
        vec!["n".into()],
        IndexType::Persistent,
        false,
    )
    .unwrap();

    b.truncate().unwrap();
    assert!(b.all().is_empty());
    // Its own definitions survive a truncate.
    assert!(b.list_indexes().iter().any(|i| i.name == "by_n"));
    assert_eq!(db.get_collection("a").unwrap().all().len(), 5);
    assert_eq!(db.get_collection("c").unwrap().all().len(), 5);

    db.delete_collection("b").unwrap();
    assert_eq!(db.get_collection("a").unwrap().all().len(), 5);
    assert_eq!(db.get_collection("c").unwrap().all().len(), 5);
    assert!(db.get_collection("b").is_err());
}

#[test]
fn recreate_gets_a_fresh_empty_keyspace_and_stale_handles_die() {
    let (engine, _tmp) = engine();
    engine.create_database("shop".into()).unwrap();
    fill(&engine, "shop", "orders", 3, "old");
    let db = engine.get_database("shop").unwrap();
    let old = db.get_collection("orders").unwrap();
    old.create_index("by_n".into(), vec!["n".into()], IndexType::Hash, false)
        .unwrap();

    db.delete_collection("orders").unwrap();
    db.create_collection("orders".into(), Some("edge".into()))
        .unwrap();
    let new = db.get_collection("orders").unwrap();

    assert!(new.all().is_empty());
    assert_eq!(new.count(), 0);
    assert!(!new.list_indexes().iter().any(|i| i.name == "by_n"));
    assert_eq!(new.get_type(), "edge");

    // The old handle does not read or write into the new incarnation.
    assert!(old.get("k000").is_err());
    assert!(old.insert(json!({"_key": "ghost"})).is_err());
    assert!(new.get("ghost").is_err());
}

#[test]
fn dropping_a_database_is_one_range_and_spares_the_others() {
    let (engine, _tmp) = engine();
    engine.create_database("app".into()).unwrap();
    engine.create_database("app_test".into()).unwrap();
    fill(&engine, "app", "users", 4, "keep");
    fill(&engine, "app_test", "users", 4, "drop");
    fill(&engine, "app_test", "orders", 4, "drop");

    let before = cf_ops::snapshot();
    engine.delete_database("app_test").unwrap();
    assert_eq!(before.ops_since(&cf_ops::snapshot()), 0);

    assert!(engine.get_database("app_test").is_err());
    let users = engine
        .get_database("app")
        .unwrap()
        .get_collection("users")
        .unwrap();
    assert_eq!(users.all().len(), 4);
    assert!(users.all().iter().all(|d| d.to_value()["tag"] == "keep"));

    // Recreated database starts empty, in a new id range.
    engine.create_database("app_test".into()).unwrap();
    let db = engine.get_database("app_test").unwrap();
    assert!(db.list_collections().is_empty());
    db.create_collection("users".into(), None).unwrap();
    assert!(db.get_collection("users").unwrap().all().is_empty());
}

#[test]
fn many_creates_and_drops_touch_no_column_family() {
    let (engine, _tmp) = engine();
    engine.create_database("churn".into()).unwrap();
    let db = engine.get_database("churn").unwrap();
    let before = cf_ops::snapshot();
    for round in 0..3 {
        for i in 0..20 {
            db.create_collection(format!("c{}", i), None).unwrap();
            db.get_collection(&format!("c{}", i))
                .unwrap()
                .insert(json!({"_key": "x", "round": round}))
                .unwrap();
        }
        for i in 0..20 {
            db.delete_collection(&format!("c{}", i)).unwrap();
        }
    }
    assert_eq!(before.ops_since(&cf_ops::snapshot()), 0);
    assert!(db.list_collections().is_empty());
}
