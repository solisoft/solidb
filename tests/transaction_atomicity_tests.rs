//! Transaction commit atomicity and conflict detection (audit D2), plus the
//! engine/database handle-cache eviction on collection recreate (audit D8).

use serde_json::json;
use solidb::error::DbError;
use solidb::storage::StorageEngine;
use solidb::transaction::IsolationLevel;
use std::sync::Arc;
use tempfile::TempDir;

fn setup() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    db.create_collection("a".to_string(), None).unwrap();
    db.create_collection("b".to_string(), None).unwrap();
    engine.initialize_transactions().unwrap();
    (engine, tmp)
}

#[test]
fn failed_commit_writes_nothing_in_any_collection_and_frees_locks() {
    let (engine, _tmp) = setup();
    let manager = engine.transaction_manager().unwrap();
    let a = engine.get_collection("app:a").unwrap();
    let b = engine.get_collection("app:b").unwrap();

    let tx_id = manager.begin(IsolationLevel::ReadCommitted).unwrap();
    {
        let tx_arc = manager.get(tx_id).unwrap();
        let mut tx = tx_arc.write().unwrap();
        let wal = manager.wal().clone();
        let locks = manager.lock_manager().clone();
        a.insert_tx(&mut tx, &wal, &locks, json!({"_key": "a1", "v": 1}))
            .unwrap();
        b.insert_tx(&mut tx, &wal, &locks, json!({"_key": "b1", "v": 1}))
            .unwrap();
    }

    // A non-transactional writer takes `b1` before the commit.
    b.insert(json!({"_key": "b1", "v": "outside"})).unwrap();

    let res = engine.commit_transaction(tx_id);
    assert!(
        matches!(res, Err(DbError::TransactionConflict(_))),
        "{:?}",
        res
    );

    // Nothing from the transaction landed — not even in the collection
    // staged before the one that conflicted.
    assert!(matches!(a.get("a1"), Err(DbError::DocumentNotFound(_))));
    assert_eq!(b.get("b1").unwrap().data["v"], json!("outside"));

    // The transaction is gone and its locks are free.
    assert!(!manager.is_active(tx_id));
    let next = manager.begin(IsolationLevel::ReadCommitted).unwrap();
    assert!(manager
        .lock_manager()
        .acquire_exclusive(next, "app", "a", "a1")
        .is_ok());
}

#[test]
fn update_based_on_a_stale_revision_conflicts() {
    let (engine, _tmp) = setup();
    let manager = engine.transaction_manager().unwrap();
    let a = engine.get_collection("app:a").unwrap();
    a.insert(json!({"_key": "d", "x": 0})).unwrap();

    let tx_id = manager.begin(IsolationLevel::ReadCommitted).unwrap();
    {
        let tx_arc = manager.get(tx_id).unwrap();
        let mut tx = tx_arc.write().unwrap();
        a.update_tx(
            &mut tx,
            &manager.wal().clone(),
            &manager.lock_manager().clone(),
            "d",
            json!({"x": 1}),
        )
        .unwrap();
    }

    // Written in between by someone who takes no transaction lock.
    a.update("d", json!({"y": 2})).unwrap();

    let res = engine.commit_transaction(tx_id);
    assert!(
        matches!(res, Err(DbError::TransactionConflict(_))),
        "{:?}",
        res
    );
    let doc = a.get("d").unwrap();
    assert_eq!(doc.data["y"], json!(2), "the intervening write survives");
    assert_eq!(doc.data["x"], json!(0), "the stale update was not applied");
    assert!(!manager.is_active(tx_id));
}

#[test]
fn commit_spans_collections_and_sees_its_own_writes() {
    let (engine, _tmp) = setup();
    let manager = engine.transaction_manager().unwrap();
    let a = engine.get_collection("app:a").unwrap();
    let b = engine.get_collection("app:b").unwrap();
    a.insert(json!({"_key": "d", "x": 0})).unwrap();

    let tx_id = manager.begin(IsolationLevel::Serializable).unwrap();
    let returned_rev = {
        let tx_arc = manager.get(tx_id).unwrap();
        let mut tx = tx_arc.write().unwrap();
        let wal = manager.wal().clone();
        let locks = manager.lock_manager().clone();
        a.update_tx(&mut tx, &wal, &locks, "d", json!({"x": 1}))
            .unwrap();
        // Based on the first update, not on disk.
        let second = a
            .update_tx(&mut tx, &wal, &locks, "d", json!({"z": 3}))
            .unwrap();
        b.insert_tx(&mut tx, &wal, &locks, json!({"_key": "n", "v": true}))
            .unwrap();
        second.rev
    };

    engine.commit_transaction(tx_id).unwrap();

    let doc = a.get("d").unwrap();
    assert_eq!(doc.data["x"], json!(1));
    assert_eq!(doc.data["z"], json!(3));
    assert_eq!(
        doc.rev, returned_rev,
        "the committed _rev is the one returned"
    );
    assert_eq!(b.get("n").unwrap().data["v"], json!(true));
    assert!(!manager.is_active(tx_id));
}

#[test]
fn recreated_collection_is_not_served_its_predecessors_handle() {
    let (engine, _tmp) = setup();
    let db = engine.get_database("app").unwrap();

    let old = engine.get_collection("app:a").unwrap();
    db.delete_collection("a").unwrap();
    db.create_collection("a".to_string(), None).unwrap();

    let fresh = engine.get_collection("app:a").unwrap();
    assert!(
        !Arc::ptr_eq(&old.change_sender, &fresh.change_sender),
        "the engine cache kept the deleted collection's handle"
    );
    let via_db = db.get_collection("a").unwrap();
    assert!(Arc::ptr_eq(&fresh.change_sender, &via_db.change_sender));
}
