//! The document write paths the driver takes (card #774): single inserts,
//! merge updates and bulk inserts, as the driver handlers run them.
//!
//! These pin down what clients see — the returned document, the change
//! events, the index entries, the errors — across the restructured paths:
//! the merge update that reads the document once under its key's lock, the
//! insert that returns its value without copying it, the bulk insert that
//! prepares its batch before taking the keys' locks, the per-key locks
//! themselves, and the cached schema.

use serde_json::{json, Value};
use solidb::driver::handlers::{document, DriverHandler};
use solidb::driver::protocol::Response;
use solidb::storage::collection::{ChangeEvent, ChangeType};
use solidb::storage::schema::{CollectionSchema, SchemaValidationMode};
use solidb::storage::{Collection, IndexType, StorageEngine};
use std::sync::Arc;
use tempfile::TempDir;

const DB: &str = "app";
const COLL: &str = "orders";

fn setup() -> (Arc<StorageEngine>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
    engine.create_database(DB.to_string()).unwrap();
    let db = engine.get_database(DB).unwrap();
    db.create_collection(COLL.to_string(), None).unwrap();
    let coll = db.get_collection(COLL).unwrap();
    for field in ["status", "customer_id"] {
        coll.create_index(
            format!("idx_{field}"),
            vec![field.to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();
    }
    (engine, tmp)
}

fn coll(engine: &StorageEngine) -> Collection {
    engine
        .get_database(DB)
        .unwrap()
        .get_collection(COLL)
        .unwrap()
}

fn ok_value(resp: Response) -> Value {
    match resp {
        Response::Ok { data: Some(v), .. } => v,
        other => panic!("expected a value, got {other:?}"),
    }
}

fn error_text(resp: Response) -> String {
    match resp {
        Response::Error { error } => error.to_string(),
        other => panic!("expected an error, got {other:?}"),
    }
}

fn drain(rx: &mut tokio::sync::broadcast::Receiver<ChangeEvent>) -> Vec<ChangeEvent> {
    let mut out = Vec::new();
    while let Ok(e) = rx.try_recv() {
        out.push(e);
    }
    out
}

/// Keys the index on `field` returns for `value`, sorted.
fn indexed(coll: &Collection, field: &str, value: &str) -> Vec<String> {
    let mut keys: Vec<String> = coll
        .index_lookup_eq(field, &json!(value))
        .expect("an index serves the field")
        .into_iter()
        .map(|d| d.key)
        .collect();
    keys.sort();
    keys
}

#[test]
fn insert_returns_the_stored_document_and_announces_it() {
    let (engine, _tmp) = setup();
    let handler = DriverHandler::new(engine.clone(), None);
    let c = coll(&engine);
    let mut rx = c.change_sender.subscribe();

    let value = ok_value(document::handle_insert(
        &handler,
        DB.into(),
        COLL.into(),
        Some("o1".into()),
        json!({"status": "pending", "customer_id": "c1", "_rev": "client", "n": [1, {"x": 2}]}),
    ));
    let stored = c.get("o1").unwrap().to_value();
    assert_eq!(value, stored);
    assert_eq!(value["_key"], "o1");
    assert_ne!(value["_rev"], "client");

    let events = drain(&mut rx);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].type_, ChangeType::Insert);
    assert_eq!(events[0].data.as_ref(), Some(&stored));
    assert_eq!(indexed(&c, "status", "pending"), vec!["o1"]);

    // A key taken: the conflict, and nothing changed.
    let err = error_text(document::handle_insert(
        &handler,
        DB.into(),
        COLL.into(),
        Some("o1".into()),
        json!({"status": "shipped"}),
    ));
    assert!(
        err.contains("Document with _key 'o1' already exists"),
        "{err}"
    );
    assert_eq!(c.get("o1").unwrap().to_value(), stored);
    assert!(drain(&mut rx).is_empty());

    // No key: one is generated, and returned.
    let value = ok_value(document::handle_insert(
        &handler,
        DB.into(),
        COLL.into(),
        None,
        json!({"status": "pending"}),
    ));
    let key = value["_key"].as_str().unwrap();
    assert_eq!(c.get(key).unwrap().to_value(), value);
}

#[test]
fn merge_update_merges_top_level_fields_once() {
    let (engine, _tmp) = setup();
    let handler = DriverHandler::new(engine.clone(), None);
    let c = coll(&engine);
    c.insert(
        json!({"_key": "o1", "status": "pending", "customer_id": "c1",
                    "address": {"city": "Lyon", "zip": "69001"}, "tags": ["a"]}),
    )
    .unwrap();
    let before = c.get("o1").unwrap().to_value();
    let mut rx = c.change_sender.subscribe();

    let value = ok_value(document::handle_update(
        &handler,
        DB.into(),
        COLL.into(),
        "o1".into(),
        json!({"status": "shipped", "address": {"city": "Paris"}, "new": 1,
               "_rev": "forged", "_created_at": "1999-01-01T00:00:00Z"}),
        true,
    ));
    let stored = c.get("o1").unwrap().to_value();
    assert_eq!(value, stored);
    // Top-level fields replace, nested objects are not merged, system
    // fields are the server's.
    assert_eq!(stored["status"], "shipped");
    assert_eq!(stored["address"], json!({"city": "Paris"}));
    assert_eq!(stored["new"], 1);
    assert_eq!(stored["tags"], json!(["a"]));
    assert_eq!(stored["customer_id"], "c1");
    assert_ne!(stored["_rev"], before["_rev"]);
    assert_ne!(stored["_rev"], "forged");
    assert_eq!(stored["_created_at"], before["_created_at"]);

    // The event carries both versions whole.
    let events = drain(&mut rx);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].type_, ChangeType::Update);
    assert_eq!(events[0].data.as_ref(), Some(&stored));
    assert_eq!(events[0].old_data.as_ref(), Some(&before));

    // The index moved with the value, and kept the untouched one.
    assert!(indexed(&c, "status", "pending").is_empty());
    assert_eq!(indexed(&c, "status", "shipped"), vec!["o1"]);
    assert_eq!(indexed(&c, "customer_id", "c1"), vec!["o1"]);

    // Changes that are not an object leave the fields as they are.
    let before = stored;
    let value = ok_value(document::handle_update(
        &handler,
        DB.into(),
        COLL.into(),
        "o1".into(),
        json!("not an object"),
        true,
    ));
    for (k, v) in before.as_object().unwrap() {
        if k != "_rev" && k != "_updated_at" {
            assert_eq!(&value[k], v, "{k}");
        }
    }
    assert_ne!(value["_rev"], before["_rev"]);
    let events = drain(&mut rx);
    assert_eq!(events[0].old_data.as_ref(), Some(&before));

    // A missing document: the error a read gives.
    let missing = error_text(document::handle_update(
        &handler,
        DB.into(),
        COLL.into(),
        "nope".into(),
        json!({"a": 1}),
        true,
    ));
    let read = c.get("nope").unwrap_err().to_string();
    assert!(missing.contains(&read), "{missing} / {read}");
}

/// Concurrent merges of different fields of one document all land: each is
/// read and written under the key's lock.
#[test]
fn concurrent_merges_of_different_fields_all_land() {
    let (engine, _tmp) = setup();
    let c = coll(&engine);
    c.insert(json!({"_key": "hot", "status": "pending"}))
        .unwrap();

    let threads: Vec<_> = (0..8)
        .map(|t| {
            let engine = engine.clone();
            std::thread::spawn(move || {
                let handler = DriverHandler::new(engine, None);
                for i in 0..50 {
                    let resp = document::handle_update(
                        &handler,
                        DB.into(),
                        COLL.into(),
                        "hot".into(),
                        json!({ format!("f{t}"): i, "status": format!("s{}", i % 3) }),
                        true,
                    );
                    ok_value(resp);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let doc = c.get("hot").unwrap().to_value();
    for t in 0..8 {
        assert_eq!(doc[format!("f{t}")], 49, "field f{t} of {doc}");
    }
    // Exactly one status entry for the document, the current one.
    let status = doc["status"].as_str().unwrap().to_string();
    for s in ["pending", "s0", "s1", "s2"] {
        let expected: Vec<String> = if s == status {
            vec!["hot".into()]
        } else {
            vec![]
        };
        assert_eq!(indexed(&c, "status", s), expected, "status {s}");
    }
}

#[test]
fn bulk_insert_reports_the_first_conflict_and_writes_nothing() {
    let (engine, _tmp) = setup();
    let handler = DriverHandler::new(engine.clone(), None);
    let c = coll(&engine);
    c.insert(json!({"_key": "b", "status": "x"})).unwrap();
    c.insert(json!({"_key": "d", "status": "x"})).unwrap();

    let docs: Vec<Value> = ["a", "b", "c", "d"]
        .iter()
        .map(|k| json!({"_key": k, "status": "new"}))
        .collect();
    let err = error_text(document::handle_bulk_insert(
        &handler,
        DB.into(),
        COLL.into(),
        docs,
    ));
    assert!(
        err.contains("Document with _key 'b' already exists"),
        "{err}"
    );
    assert_eq!(c.count(), 2);
    assert!(c.get("a").is_err() && c.get("c").is_err());
    assert!(indexed(&c, "status", "new").is_empty());

    let err = error_text(document::handle_bulk_insert(
        &handler,
        DB.into(),
        COLL.into(),
        vec![json!({"_key": "e"}), json!({"_key": "e"})],
    ));
    assert!(err.contains("Duplicate _key 'e' within batch"), "{err}");
    assert_eq!(c.count(), 2);
}

#[test]
fn bulk_insert_announces_every_document_whole() {
    let (engine, _tmp) = setup();
    let handler = DriverHandler::new(engine.clone(), None);
    let c = coll(&engine);
    let mut rx = c.change_sender.subscribe();

    let docs: Vec<Value> = (0..20)
        .map(|i| json!({"_key": format!("k{i}"), "status": "new", "items": [{"sku": i}]}))
        .collect();
    match document::handle_bulk_insert(&handler, DB.into(), COLL.into(), docs) {
        Response::Ok { count, .. } => assert_eq!(count, Some(20)),
        other => panic!("{other:?}"),
    }
    let events = drain(&mut rx);
    assert_eq!(events.len(), 20);
    for e in &events {
        assert_eq!(e.type_, ChangeType::Insert);
        assert_eq!(e.data.as_ref(), Some(&c.get(&e.key).unwrap().to_value()));
    }
    assert_eq!(indexed(&c, "status", "new").len(), 20);
}

/// Bulk inserts into one collection from many connections run side by side
/// and each document gets exactly its entries. Batches overlapping on a key
/// still conflict: exactly one of them wins.
#[test]
fn concurrent_bulk_inserts_into_one_collection() {
    let (engine, _tmp) = setup();
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let engine = engine.clone();
            std::thread::spawn(move || {
                let handler = DriverHandler::new(engine, None);
                for b in 0..10 {
                    let docs: Vec<Value> = (0..200)
                        .map(|i| {
                            json!({"_key": format!("t{t}-b{b}-{i}"),
                                   "status": format!("s{}", i % 4),
                                   "customer_id": format!("c{t}")})
                        })
                        .collect();
                    let resp = document::handle_bulk_insert(&handler, DB.into(), COLL.into(), docs);
                    assert!(
                        matches!(
                            resp,
                            Response::Ok {
                                count: Some(200),
                                ..
                            }
                        ),
                        "{resp:?}"
                    );
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let c = coll(&engine);
    assert_eq!(c.count(), 8 * 10 * 200);
    for s in 0..4 {
        assert_eq!(indexed(&c, "status", &format!("s{s}")).len(), 8 * 10 * 50);
    }
    for t in 0..8 {
        assert_eq!(indexed(&c, "customer_id", &format!("c{t}")).len(), 10 * 200);
    }

    // Eight batches all claiming the same keys: one wins, the rest conflict.
    let wins: usize = (0..8)
        .map(|_| {
            let engine = engine.clone();
            std::thread::spawn(move || {
                let handler = DriverHandler::new(engine, None);
                let docs: Vec<Value> = (0..300)
                    .map(|i| json!({"_key": format!("shared-{i}"), "status": "race"}))
                    .collect();
                matches!(
                    document::handle_bulk_insert(&handler, DB.into(), COLL.into(), docs),
                    Response::Ok { .. }
                ) as usize
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .sum();
    assert_eq!(wins, 1);
    assert_eq!(indexed(&c, "status", "race").len(), 300);
}

/// A schema set through one handle applies to writes through another, and
/// its removal too.
#[test]
fn schema_changes_reach_every_handle() {
    let (engine, _tmp) = setup();
    let handler = DriverHandler::new(engine.clone(), None);
    let writer = coll(&engine);
    writer
        .insert(json!({"_key": "before", "status": "ok"}))
        .unwrap();

    let admin = engine
        .get_database(DB)
        .unwrap()
        .get_collection(COLL)
        .unwrap();
    admin
        .set_json_schema(CollectionSchema::new(
            "s".into(),
            json!({"type": "object", "required": ["status"]}),
            SchemaValidationMode::Strict,
        ))
        .unwrap();

    let err = error_text(document::handle_insert(
        &handler,
        DB.into(),
        COLL.into(),
        Some("bad".into()),
        json!({"other": 1}),
    ));
    assert!(err.contains("Schema validation failed"), "{err}");
    let err = error_text(document::handle_bulk_insert(
        &handler,
        DB.into(),
        COLL.into(),
        vec![json!({"_key": "bad2"})],
    ));
    assert!(err.contains("Schema validation failed"), "{err}");
    ok_value(document::handle_insert(
        &handler,
        DB.into(),
        COLL.into(),
        Some("good".into()),
        json!({"status": "ok"}),
    ));

    admin.remove_json_schema().unwrap();
    ok_value(document::handle_insert(
        &handler,
        DB.into(),
        COLL.into(),
        Some("free".into()),
        json!({"other": 1}),
    ));
    assert!(writer.get("free").is_ok() && writer.get("bad").is_err());
}

/// Writes over a real driver connection, where inserts, bulk inserts and
/// merge updates are stored from the client's bytes: what the client reads
/// back and the errors it gets, including for writes that fall back to the
/// decoded path (a schema'd collection, a `_key` that is not a string, a
/// replace update) and frames the server cannot decode.
#[test]
fn driver_connection_writes() {
    std::env::set_var("SOLIDB_ADMIN_PASSWORD", "write-path-password");
    let (engine, tmp) = setup();
    engine.create_database("_system".to_string()).unwrap();
    solidb::server::auth::AuthService::init(&engine, None, tmp.path().to_str().unwrap()).unwrap();
    let db = engine.get_database(DB).unwrap();
    db.create_collection("schemed".to_string(), None).unwrap();
    db.get_collection("schemed")
        .unwrap()
        .set_json_schema(CollectionSchema::new(
            "s".into(),
            json!({"type": "object", "required": ["status"]}),
            SchemaValidationMode::Strict,
        ))
        .unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let storage = engine.clone();
    rt.spawn(async move {
        use tokio::io::AsyncReadExt;
        loop {
            let (mut stream, peer) = listener.accept().await.unwrap();
            let storage = storage.clone();
            tokio::spawn(async move {
                let mut magic = [0u8; 14];
                stream.read_exact(&mut magic).await.unwrap();
                let mut handler = DriverHandler::new(storage, None);
                handler.handle_connection(stream, peer.to_string()).await;
            });
        }
    });

    rt.block_on(async {
        let mut c = solidb_client::SoliDBClientBuilder::new(&addr)
            .auth(DB, "admin", "write-path-password")
            .pool_size(1)
            .build()
            .await
            .unwrap();
        let coll = || engine.get_database(DB).unwrap().get_collection(COLL).unwrap();

        // An insert answers with the stored document.
        let doc = json!({"status": "pending", "customer_id": "c1", "items": [{"sku": "A", "qty": 2}],
                         "zeta": 1.5, "alpha": -3, "_rev": "forged"});
        let reply = c.insert(DB, COLL, Some("o1"), doc).await.unwrap();
        assert_eq!(reply, coll().get("o1").unwrap().to_value());
        assert_eq!(reply["alpha"], -3);
        assert_ne!(reply["_rev"], "forged");
        assert_eq!(c.get(DB, COLL, "o1").await.unwrap(), reply);

        // A key taken: the decoded path's conflict.
        let err = c
            .insert(DB, COLL, Some("o1"), json!({"status": "x"}))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Document with _key 'o1' already exists"), "{err}");

        // A bulk insert; a duplicate key inside one.
        let docs: Vec<Value> = (0..100)
            .map(|i| json!({"_key": format!("b{i}"), "status": "new", "n": i}))
            .collect();
        assert_eq!(c.bulk_insert(DB, COLL, docs).await.unwrap(), 100);
        let err = c
            .bulk_insert(DB, COLL, vec![json!({"_key": "d"}), json!({"_key": "d"})])
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Duplicate _key 'd' within batch"), "{err}");
        assert_eq!(coll().count(), 101);
        let indexed = coll().index_lookup_eq("status", &json!("new")).unwrap();
        assert_eq!(indexed.len(), 100);

        // A merge update answers with the merged document.
        let reply = c
            .update(DB, COLL, "b7", json!({"status": "shipped", "extra": {"x": 1}}), true)
            .await
            .unwrap();
        let stored = coll().get("b7").unwrap().to_value();
        assert_eq!(reply, stored);
        assert_eq!(stored["n"], 7);
        assert_eq!(stored["extra"], json!({"x": 1}));
        assert_eq!(coll().index_lookup_eq("status", &json!("new")).unwrap().len(), 99);
        let err = c
            .update(DB, COLL, "nope", json!({"a": 1}), true)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("nope"), "{err}");

        // A replace update, a schema'd collection, a `_key` that is not a
        // string: the decoded path, with its answers.
        let reply = c
            .update(DB, COLL, "b8", json!({"only": true}), false)
            .await
            .unwrap();
        assert_eq!(reply, coll().get("b8").unwrap().to_value());
        let err = c
            .insert(DB, "schemed", None, json!({"other": 1}))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Schema validation failed"), "{err}");
        let err = c
            .insert(DB, COLL, None, json!({"_key": 5}))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("_key must be a string"), "{err}");
    });
}
