//! Startup migration of 1.x collections (one column family each) into the
//! shared keyspace.
//!
//! Each test writes a 1.x layout with `SOLIDB_LEGACY_COLLECTION_CFS=1`, closes
//! the engine, and reopens it without the switch: `initialize` migrates. The
//! switch is process-wide, so the tests in this file run one at a time.

use serde_json::json;
use solidb::storage::cf_ops;
use solidb::storage::{IndexType, StorageEngine};
use std::sync::Mutex;
use tempfile::TempDir;

static SERIAL: Mutex<()> = Mutex::new(());

const LEGACY: &str = "SOLIDB_LEGACY_COLLECTION_CFS";

/// Run `f` against an engine that writes the 1.x layout, then close it.
fn legacy_session(path: &str, f: impl FnOnce(&StorageEngine)) {
    std::env::set_var(LEGACY, "1");
    {
        let engine = StorageEngine::new(path).unwrap();
        engine.initialize().unwrap();
        f(&engine);
        engine.flush_all_stats();
        engine.flush().unwrap();
    }
    std::env::remove_var(LEGACY);
}

fn open(path: &str) -> StorageEngine {
    std::env::remove_var(LEGACY);
    let engine = StorageEngine::new(path).unwrap();
    engine.initialize().unwrap();
    engine
}

/// Column families on disk, minus RocksDB's and SoliDB's own.
fn collection_cfs(path: &str) -> Vec<String> {
    let opts = rust_rocksdb::Options::default();
    let mut cfs: Vec<String> = rust_rocksdb::DB::list_cf(&opts, path)
        .unwrap_or_default()
        .into_iter()
        .filter(|n| n != "default" && n != "_meta" && n != "__keyspaces__")
        .collect();
    cfs.sort();
    cfs
}

fn write_legacy_fixture(path: &str) {
    legacy_session(path, |engine| {
        engine.create_database("shop".into()).unwrap();
        let shop = engine.get_database("shop").unwrap();
        shop.create_collection("users".into(), None).unwrap();
        shop.create_collection("links".into(), Some("edge".into()))
            .unwrap();
        let users = shop.get_collection("users").unwrap();
        for i in 0..250 {
            users
                .insert(json!({"_key": format!("u{:03}", i), "n": i, "bio": format!("likes word{} and cheese", i)}))
                .unwrap();
        }
        users
            .create_index(
                "by_n".into(),
                vec!["n".into()],
                IndexType::Persistent,
                false,
            )
            .unwrap();
        users
            .create_fulltext_index("bio_ft".into(), vec!["bio".into()], None)
            .unwrap();
        shop.get_collection("links")
            .unwrap()
            .insert(json!({"_key": "l1", "_from": "users/u001", "_to": "users/u002"}))
            .unwrap();
        // An engine-level collection with no database.
        engine.create_collection("bare".into(), None).unwrap();
        engine
            .get_collection("bare")
            .unwrap()
            .insert(json!({"_key": "b1"}))
            .unwrap();
    });
}

fn assert_fixture_intact(engine: &StorageEngine) {
    let shop = engine.get_database("shop").unwrap();
    let mut names = shop.list_collections();
    names.sort();
    assert_eq!(names, vec!["links".to_string(), "users".to_string()]);

    let users = shop.get_collection("users").unwrap();
    assert_eq!(users.count(), 250);
    assert_eq!(users.all().len(), 250);
    assert_eq!(users.get("u042").unwrap().to_value()["n"], 42);
    // Index definitions and entries came along.
    assert!(users.list_indexes().iter().any(|i| i.name == "by_n"));
    assert_eq!(
        users
            .index_lookup_gte("n", &json!(200), None)
            .unwrap_or_default()
            .len(),
        50
    );
    assert_eq!(users.fulltext_search("word7", None, 10).unwrap().len(), 1);

    let links = shop.get_collection("links").unwrap();
    assert_eq!(links.get_type(), "edge");
    assert_eq!(links.count(), 1);
    assert_eq!(engine.get_collection("bare").unwrap().count(), 1);
}

#[test]
fn a_legacy_instance_migrates_with_everything_intact() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    write_legacy_fixture(path);
    assert!(collection_cfs(path).contains(&"shop:users".to_string()));

    let engine = open(path);
    assert_fixture_intact(&engine);

    // New collections no longer create column families.
    let before = cf_ops::snapshot();
    let shop = engine.get_database("shop").unwrap();
    shop.create_collection("orders".into(), None).unwrap();
    shop.delete_collection("users").unwrap();
    assert_eq!(before.ops_since(&cf_ops::snapshot()), 0);
    assert!(shop.get_collection("users").is_err());
    assert_eq!(shop.get_collection("links").unwrap().count(), 1);
}

#[test]
fn a_second_start_is_a_no_op_and_data_survives_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    write_legacy_fixture(path);

    {
        let engine = open(path);
        assert_fixture_intact(&engine);
        engine
            .get_database("shop")
            .unwrap()
            .get_collection("users")
            .unwrap()
            .insert(json!({"_key": "after"}))
            .unwrap();
        engine.flush_all_stats();
        engine.flush().unwrap();
    }
    let engine = open(path);
    let users = engine
        .get_database("shop")
        .unwrap()
        .get_collection("users")
        .unwrap();
    assert_eq!(users.count(), 251);
    assert!(users.get("after").is_ok());
}

/// A crash in the middle of a copy leaves `migrating_to` on the entry and
/// some keys in the target range. The next start must wipe them and redo the
/// copy, not add to them.
#[test]
fn an_interrupted_copy_is_redone_from_scratch() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    write_legacy_fixture(path);

    // Simulate the half-done copy directly on disk.
    let target: u64 = (77u64 << 32) | 5;
    {
        let mut opts = rust_rocksdb::Options::default();
        opts.create_missing_column_families(true);
        let mut cfs = rust_rocksdb::DB::list_cf(&opts, path).unwrap();
        if !cfs.contains(&"__keyspaces__".to_string()) {
            cfs.push("__keyspaces__".to_string());
        }
        let db = rust_rocksdb::DB::open_cf(&opts, path, &cfs).unwrap();
        let meta = db.cf_handle("_meta").unwrap();
        db.put_cf(
            &meta,
            b"coll:shop:users",
            serde_json::to_vec(
                &json!({"type_": "document", "created_ms": 0, "migrating_to": target}),
            )
            .unwrap(),
        )
        .unwrap();
        let shared = db.cf_handle("__keyspaces__").unwrap();
        let mut junk = target.to_be_bytes().to_vec();
        junk.extend_from_slice(b"doc:junk");
        db.put_cf(&shared, &junk, b"not a document").unwrap();
    }

    let engine = open(path);
    assert_fixture_intact(&engine);
    let users = engine
        .get_database("shop")
        .unwrap()
        .get_collection("users")
        .unwrap();
    assert!(users.get("junk").is_err());
    assert_eq!(users.recalculate_count(), 250);
}

/// A `db:coll` column family whose database record is gone is the remains of
/// an interrupted 1.x drop: it is neither migrated nor deleted.
#[test]
fn an_orphaned_column_family_is_left_alone() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    write_legacy_fixture(path);
    {
        let mut opts = rust_rocksdb::Options::default();
        opts.create_missing_column_families(true);
        let cfs = rust_rocksdb::DB::list_cf(&opts, path).unwrap();
        let mut db = rust_rocksdb::DB::open_cf(&opts, path, &cfs).unwrap();
        db.create_cf("ghost:things", &rust_rocksdb::Options::default())
            .unwrap();
    }

    let engine = open(path);
    assert_fixture_intact(&engine);
    assert!(collection_cfs(path).contains(&"ghost:things".to_string()));
}
