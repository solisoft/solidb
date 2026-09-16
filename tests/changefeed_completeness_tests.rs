//! A changefeed subscriber must see every write to its collection.
//!
//! Two paths used to bypass `change_sender` entirely, so a subscriber went
//! quiet while documents were appearing underneath it:
//!
//! - `upsert_batch` never broadcast. That is the path replication applies
//!   through and the one shard replicas receive on, so deletes propagated to
//!   remote subscribers while the inserts that preceded them did not.
//! - `StorageEngine::system_collection` built its own `Collection`, and so its
//!   own broadcast channel, instead of reusing the one `Database` hands out.
//!   Writes arriving through the engine — the transaction endpoints — fired
//!   into a ring nobody was listening to.

use serde_json::json;
use solidb::storage::collection::{ChangeEvent, ChangeType};
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;

fn create_test_engine() -> (Arc<StorageEngine>, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (Arc::new(engine), tmp_dir)
}

fn setup(engine: &Arc<StorageEngine>) {
    engine
        .create_database("app".to_string())
        .expect("create database");
    let db = engine.get_database("app").expect("get database");
    db.create_collection("notes".to_string(), None)
        .expect("create collection");
}

/// Drain whatever the subscriber has buffered without blocking.
fn drain(rx: &mut tokio::sync::broadcast::Receiver<ChangeEvent>) -> Vec<ChangeEvent> {
    let mut out = Vec::new();
    while let Ok(event) = rx.try_recv() {
        out.push(event);
    }
    out
}

#[test]
fn upsert_batch_broadcasts_inserts_and_updates() {
    let (engine, _tmp) = create_test_engine();
    setup(&engine);

    let coll = engine
        .get_database("app")
        .unwrap()
        .get_collection("notes")
        .unwrap();
    let mut rx = coll.change_sender.subscribe();

    coll.upsert_batch(vec![
        ("a".to_string(), json!({"body": "first"})),
        ("b".to_string(), json!({"body": "second"})),
    ])
    .expect("upsert batch");

    let events = drain(&mut rx);
    assert_eq!(events.len(), 2, "one event per upserted document");
    assert!(
        events.iter().all(|e| e.type_ == ChangeType::Insert),
        "documents that did not exist are Inserts, got {:?}",
        events.iter().map(|e| &e.type_).collect::<Vec<_>>()
    );
    let mut keys: Vec<&str> = events.iter().map(|e| e.key.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["a", "b"]);
    assert!(
        events[0].data.is_some(),
        "the event carries the document, which is what a subscriber reads"
    );

    // The same keys again are Updates, not Inserts — the distinction is what
    // lets a subscriber tell a new row from a touched one.
    coll.upsert_batch(vec![("a".to_string(), json!({"body": "edited"}))])
        .expect("upsert batch");

    let events = drain(&mut rx);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].type_, ChangeType::Update);
    assert_eq!(events[0].key, "a");
}

#[test]
fn engine_and_database_collection_handles_share_one_channel() {
    let (engine, _tmp) = create_test_engine();
    setup(&engine);

    // A subscriber attaches the way the changefeed handler does: through the
    // database.
    let via_database = engine
        .get_database("app")
        .unwrap()
        .get_collection("notes")
        .unwrap();
    let mut rx = via_database.change_sender.subscribe();

    // A writer arrives the way the transaction endpoints do: through the
    // engine, by column-family name.
    let via_engine = engine.get_collection("app:notes").expect("engine handle");
    via_engine
        .insert(json!({"_key": "x", "body": "written through the engine"}))
        .expect("insert");

    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "the engine-side handle must publish to the channel the database-side \
         subscriber is listening on"
    );
    assert_eq!(events[0].key, "x");
    assert_eq!(events[0].type_, ChangeType::Insert);
}

#[test]
fn engine_handle_still_resolves_when_the_database_entry_is_gone() {
    // A column family can outlive its `db:` metadata key when a drop is
    // interrupted. The engine path has to keep serving those rather than
    // refusing, which is what it did before it started delegating.
    let (engine, _tmp) = create_test_engine();
    setup(&engine);

    engine
        .get_database("app")
        .unwrap()
        .get_collection("notes")
        .unwrap()
        .insert(json!({"_key": "kept", "body": "still here"}))
        .expect("insert");

    // `delete_database` removes the `db:` key and schedules the CFs for a
    // background drop; a handle taken before that stays usable, and the engine
    // lookup must not panic either way.
    let still_readable = engine.get_collection("app:notes");
    assert!(
        still_readable.is_ok(),
        "engine lookup by column-family name must resolve while the CF exists"
    );
}
