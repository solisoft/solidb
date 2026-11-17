//! Field rules (`storage::field_rules`, card #848): what a collection asks
//! of a top-level field, on every way a document is written.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::sdbql::{parse, QueryExecutor};
use solidb::server::routes::create_router;
use solidb::storage::field_rules::FieldRule;
use solidb::storage::{Collection, IndexType, StorageEngine};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

fn rules(body: Value) -> Vec<FieldRule> {
    serde_json::from_value(body).unwrap()
}

fn shop_rules() -> Vec<FieldRule> {
    rules(json!([
        {"field": "name", "required": true, "type": "string"},
        {"field": "price", "type": "number", "min": 0, "max": 1000},
        {"field": "status", "in": ["open", "paid"]},
    ]))
}

fn engine() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    (engine, tmp)
}

fn items(engine: &StorageEngine, name: &str) -> Collection {
    engine.create_collection(name.to_string(), None).unwrap();
    let c = engine.get_collection(name).unwrap();
    c.set_field_rules(shop_rules()).unwrap();
    c
}

fn run(engine: &StorageEngine, q: &str) -> Result<Vec<Value>, String> {
    let query = parse(q).unwrap_or_else(|e| panic!("parse failed for {q}: {e:?}"));
    QueryExecutor::new(engine)
        .execute(&query)
        .map_err(|e| e.to_string())
}

fn refused<T: std::fmt::Debug>(result: Result<T, impl ToString>, why: &str) {
    match result {
        Ok(v) => panic!("accepted, expected '{why}': {v:?}"),
        Err(e) => {
            let e = e.to_string();
            assert!(e.contains("Schema validation failed"), "{e}");
            assert!(e.contains(why), "{e}");
        }
    }
}

#[test]
fn every_write_of_the_collection_is_held_to_the_rules() {
    let (engine, _tmp) = engine();
    // With no index, with one, and with one on a field a rule reads.
    for (name, indexed) in [
        ("bare", None),
        ("other", Some("sku")),
        ("ruled", Some("price")),
    ] {
        let c = items(&engine, name);
        if let Some(field) = indexed {
            c.create_index(
                "i".to_string(),
                vec![field.to_string()],
                IndexType::Persistent,
                false,
            )
            .unwrap();
        }
        c.insert(json!({"_key": "a", "name": "A", "price": 10, "status": "open"}))
            .unwrap();
        c.insert_batch(vec![
            json!({"_key": "b", "name": "B"}),
            json!({"_key": "c", "name": "C", "price": 0.5}),
        ])
        .unwrap();

        refused(c.insert(json!({"price": 1})), "'name' is required");
        refused(c.insert(json!({"name": 3})), "'name' must be a string");
        refused(
            c.insert(json!({"name": "x", "price": -1})),
            "'price' must be at least 0",
        );
        refused(
            c.insert_batch(vec![
                json!({"name": "ok"}),
                json!({"name": "x", "status": "lost"}),
            ]),
            "'status' is not one of the values allowed",
        );
        refused(
            c.update("a", json!({"price": 2000})),
            "'price' must be at most 1000",
        );
        refused(c.update("a", json!({"name": null})), "'name' is required");
        refused(
            c.update_batch(&[
                ("a".to_string(), json!({"price": 3})),
                ("b".to_string(), json!({"price": "3"})),
            ]),
            "'price' must be a number",
        );
        refused(
            c.replace_batch(&[
                ("a".to_string(), json!({"name": "A2"})),
                ("b".to_string(), json!({"price": 3})),
            ]),
            "'name' is required",
        );
        // Nothing of a refused write was stored.
        assert_eq!(c.count(), 3, "{name}");
        assert_eq!(c.get("a").unwrap().data["price"], json!(10), "{name}");
        assert_eq!(c.get("b").unwrap().data.get("price"), None, "{name}");

        // SDBQL, row by row and in batches.
        refused(
            run(&engine, &format!("INSERT {{price: 1}} INTO {name}")),
            "'name' is required",
        );
        refused(
            run(
                &engine,
                &format!("FOR i IN 1..40 INSERT {{name: 'n', price: i * 30}} INTO {name}"),
            ),
            "'price' must be at most 1000",
        );
        refused(
            run(
                &engine,
                &format!("FOR d IN {name} UPDATE d WITH {{status: 'lost'}} IN {name}"),
            ),
            "'status' is not one of the values allowed",
        );
        refused(
            run(&engine, &format!("UPDATE 'a' WITH {{price: -5}} IN {name}")),
            "'price' must be at least 0",
        );
        refused(
            run(&engine, &format!("REPLACE 'a' WITH {{price: 5}} IN {name}")),
            "'name' is required",
        );
        refused(
            run(
                &engine,
                &format!(
                    "UPSERT {{_key: 'zz'}} INSERT {{_key: 'zz', price: 1}} UPDATE {{}} IN {name}"
                ),
            ),
            "'name' is required",
        );
        refused(
            run(
                &engine,
                &format!("INSERT {{_key: 'a', price: 1}} INTO {name} OPTIONS {{overwrite: true}}"),
            ),
            "'name' is required",
        );
        assert_eq!(c.count(), 3, "{name}");

        // What respects them is written.
        run(
            &engine,
            &format!("FOR d IN {name} UPDATE d WITH {{status: 'paid', price: 7}} IN {name}"),
        )
        .unwrap();
        run(
            &engine,
            &format!("INSERT {{name: 'D', price: 1000}} INTO {name}"),
        )
        .unwrap();
        assert_eq!(c.count(), 4, "{name}");
        assert_eq!(c.get("b").unwrap().data["status"], json!("paid"), "{name}");
    }
}

#[test]
fn an_upsert_batch_and_a_transaction_are_held_to_the_rules() {
    let (engine, _tmp) = engine();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    db.create_collection("goods".to_string(), None).unwrap();
    engine.initialize_transactions().unwrap();
    let c = db.get_collection("goods").unwrap();
    c.set_field_rules(shop_rules()).unwrap();
    c.insert(json!({"_key": "a", "name": "A", "price": 1}))
        .unwrap();

    // What an import in upsert mode calls: a document created, one merged.
    refused(
        c.upsert_batch(vec![
            ("a".to_string(), json!({"price": 2})),
            ("new".to_string(), json!({"price": 2})),
        ]),
        "'name' is required",
    );
    refused(
        c.upsert_batch(vec![("a".to_string(), json!({"price": -2}))]),
        "'price' must be at least 0",
    );
    assert_eq!(c.count(), 1);
    assert_eq!(c.get("a").unwrap().data["price"], json!(1));
    c.upsert_batch(vec![
        ("a".to_string(), json!({"price": 2})),
        ("new".to_string(), json!({"name": "N"})),
    ])
    .unwrap();

    // In a transaction the statement that breaks a rule is refused, and
    // the transaction goes on.
    let manager = engine.transaction_manager().unwrap();
    let tx_id = manager.begin(Default::default()).unwrap();
    let tx_arc = manager.get(tx_id).unwrap();
    {
        let mut tx = tx_arc.write().unwrap();
        let (wal, locks) = (manager.wal().clone(), manager.lock_manager().clone());
        refused(
            c.insert_tx(&mut tx, &wal, &locks, json!({"_key": "t1", "price": 5})),
            "'name' is required",
        );
        refused(
            c.update_tx(&mut tx, &wal, &locks, "a", json!({"status": "lost"})),
            "'status' is not one of the values allowed",
        );
        c.insert_tx(
            &mut tx,
            &wal,
            &locks,
            json!({"_key": "t2", "name": "T", "price": 5}),
        )
        .unwrap();
        c.update_tx(&mut tx, &wal, &locks, "a", json!({"status": "paid"}))
            .unwrap();
    }
    engine.commit_transaction(tx_id).unwrap();
    assert_eq!(c.count(), 3);
    assert_eq!(c.get("a").unwrap().data["status"], json!("paid"));
    assert!(c.get("t1").is_err());
}

/// Card #946: a JSON schema was checked by a plain insert and not by a
/// transaction or a batch of upserts, which took what the insert refused.
#[test]
fn a_json_schema_is_checked_in_a_transaction_and_an_upsert_batch() {
    use solidb::storage::schema::{CollectionSchema, SchemaValidationMode};
    let (engine, _tmp) = engine();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    db.create_collection("people".to_string(), None).unwrap();
    engine.initialize_transactions().unwrap();
    let c = db.get_collection("people").unwrap();
    c.set_json_schema(CollectionSchema::new(
        "people".to_string(),
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {"name": {"type": "string"}, "age": {"type": "integer", "minimum": 0}}
        }),
        SchemaValidationMode::Strict,
    ))
    .unwrap();
    c.insert(json!({"_key": "a", "name": "Ada", "age": 36}))
        .unwrap();
    let schema = |result: Result<_, solidb::error::DbError>| match result {
        Ok(()) => panic!("accepted"),
        Err(e) => assert!(e.to_string().contains("Schema validation failed"), "{e}"),
    };
    schema(c.insert(json!({"age": 1})).map(|_| ()));

    schema(
        c.upsert_batch(vec![("b".to_string(), json!({"age": 1}))])
            .map(|_| ()),
    );
    schema(
        c.upsert_batch(vec![("a".to_string(), json!({"age": -1}))])
            .map(|_| ()),
    );
    assert_eq!(c.count(), 1);
    assert_eq!(c.get("a").unwrap().data["age"], json!(36));
    c.upsert_batch(vec![("a".to_string(), json!({"age": 37}))])
        .unwrap();

    let manager = engine.transaction_manager().unwrap();
    let tx_id = manager.begin(Default::default()).unwrap();
    let tx_arc = manager.get(tx_id).unwrap();
    {
        let mut tx = tx_arc.write().unwrap();
        let (wal, locks) = (manager.wal().clone(), manager.lock_manager().clone());
        schema(
            c.insert_tx(&mut tx, &wal, &locks, json!({"_key": "t1", "age": 5}))
                .map(|_| ()),
        );
        schema(
            c.update_tx(&mut tx, &wal, &locks, "a", json!({"name": 7}))
                .map(|_| ()),
        );
        c.insert_tx(&mut tx, &wal, &locks, json!({"_key": "t2", "name": "Tess"}))
            .unwrap();
        c.update_tx(&mut tx, &wal, &locks, "a", json!({"age": 38}))
            .unwrap();
    }
    engine.commit_transaction(tx_id).unwrap();
    assert_eq!(c.count(), 2);
    assert_eq!(c.get("a").unwrap().data["age"], json!(38));
}

#[test]
fn rules_are_kept_replaced_and_removed() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();
    {
        let engine = StorageEngine::new(&path).unwrap();
        let c = items(&engine, "things");
        assert_eq!(*c.field_rules(), shop_rules());
        // Documents already stored are not checked: a rule holds the
        // writes made from now on.
        c.set_field_rules(Vec::new()).unwrap();
        c.insert(json!({"_key": "old", "price": -3})).unwrap();
        c.set_field_rules(shop_rules()).unwrap();
        assert_eq!(c.get("old").unwrap().data["price"], json!(-3));
        // An update of it is held to them, the rule on `name` included.
        refused(c.update("old", json!({"price": 3})), "'name' is required");
        c.update("old", json!({"price": 3, "name": "Old"})).unwrap();
        // A definition that cannot be declared changes nothing.
        assert!(c
            .set_field_rules(rules(json!([{"field": "a.b", "required": true}])))
            .is_err());
        assert_eq!(*c.field_rules(), shop_rules());
    }
    // Another handle, after a restart.
    let engine = StorageEngine::new(&path).unwrap();
    let c = engine.get_collection("things").unwrap();
    assert_eq!(*c.field_rules(), shop_rules());
    refused(c.insert(json!({"price": 1})), "'name' is required");
    // Replaced: the rules it had are gone.
    c.set_field_rules(rules(json!([{"field": "price", "type": "number"}])))
        .unwrap();
    c.insert(json!({"price": -100})).unwrap();
    refused(c.insert(json!({"price": "1"})), "'price' must be a number");
    c.set_field_rules(Vec::new()).unwrap();
    assert!(c.field_rules().is_empty());
    c.insert(json!({"price": "1"})).unwrap();
}

async fn call(
    app: &axum::Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", token))
        .body(match body {
            Some(b) => Body::from(b.to_string()),
            None => Body::empty(),
        })
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn rules_are_declared_read_and_removed_over_http() {
    let (engine, _tmp) = engine();
    engine.initialize().unwrap();
    let app = create_router(
        engine.clone(),
        None,
        None,
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    let token = common::seed_user_token(&engine, "rules_admin", &["admin"]);
    let base = "/_api/database/_system";
    let (status, body) = call(
        &app,
        &token,
        "POST",
        &format!("{base}/collection"),
        Some(json!({"name": "goods"})),
    )
    .await;
    assert!(status.is_success(), "{status} {body}");

    let rules_url = format!("{base}/collection/goods/rules");
    let (status, body) = call(&app, &token, "GET", &rules_url, None).await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({"rules": []})));

    let declared = json!({"rules": [
        {"field": "name", "required": true, "type": "string"},
        {"field": "price", "min": 0.0},
    ]});
    let (status, body) = call(&app, &token, "PUT", &rules_url, Some(declared.clone())).await;
    assert_eq!((status, &body), (StatusCode::OK, &declared));
    let (_, body) = call(&app, &token, "GET", &rules_url, None).await;
    assert_eq!(body, declared);

    // A definition that says nothing, and an unknown word.
    for bad in [
        json!({"rules": [{"field": "x"}]}),
        json!({"rules": [{"field": "x", "requried": true}]}),
    ] {
        let (status, body) = call(&app, &token, "PUT", &rules_url, Some(bad)).await;
        assert!(status.is_client_error(), "{status} {body}");
    }
    let (_, body) = call(&app, &token, "GET", &rules_url, None).await;
    assert_eq!(body, declared);

    let doc_url = format!("{base}/document/goods");
    let (status, body) = call(&app, &token, "POST", &doc_url, Some(json!({"price": 3}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.to_string().contains("'name' is required"), "{body}");
    let (status, body) = call(
        &app,
        &token,
        "POST",
        &doc_url,
        Some(json!({"_key": "g1", "name": "G", "price": 3})),
    )
    .await;
    assert!(status.is_success(), "{status} {body}");
    let (status, body) = call(
        &app,
        &token,
        "PUT",
        &format!("{doc_url}/g1"),
        Some(json!({"price": -3})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.to_string().contains("'price' must be at least 0"),
        "{body}"
    );

    let (status, _) = call(&app, &token, "DELETE", &rules_url, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = call(&app, &token, "POST", &doc_url, Some(json!({"price": -3}))).await;
    assert!(status.is_success(), "{status} {body}");
}

/// Card #948: what a collection asks of its documents is logged for the
/// peers, whole, and a peer that applies it refuses what the origin
/// refuses.
#[tokio::test]
async fn rules_and_schema_are_logged_and_applied_by_a_peer() {
    use solidb::sync::log::SyncLog;
    use solidb::sync::protocol::Operation;
    let tmp = TempDir::new().unwrap();
    let origin = StorageEngine::new(tmp.path().join("a").to_str().unwrap()).unwrap();
    origin.initialize().unwrap();
    let log = Arc::new(
        SyncLog::new(
            "n1".to_string(),
            tmp.path().join("log").to_str().unwrap(),
            64,
        )
        .unwrap(),
    );
    let app = create_router(
        origin.clone(),
        None,
        Some(log.clone()),
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    let token = common::seed_user_token(&origin, "rules_admin", &["admin"]);
    let base = "/_api/database/_system";
    call(
        &app,
        &token,
        "POST",
        &format!("{base}/collection"),
        Some(json!({"name": "goods"})),
    )
    .await;
    let rules_url = format!("{base}/collection/goods/rules");
    let schema_url = format!("{base}/collection/goods/schema");
    let declared = json!({"rules": [{"field": "name", "required": true}]});
    let (status, _) = call(&app, &token, "PUT", &rules_url, Some(declared)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        &token,
        "POST",
        &schema_url,
        Some(json!({"schema": {"type": "object", "properties": {"n": {"type": "integer"}}}, "validationMode": "strict"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // A collection created with a schema logs it after its creation.
    let (status, _) = call(
        &app,
        &token,
        "POST",
        &format!("{base}/collection"),
        Some(
            json!({"name": "born_with", "schema": {"type": "object"}, "validationMode": "strict"}),
        ),
    )
    .await;
    assert!(status.is_success());
    let born: Vec<Operation> = log
        .get_entries_after(0, 100)
        .into_iter()
        .filter(|e| e.collection == "born_with")
        .map(|e| e.operation)
        .collect();
    assert_eq!(born, [Operation::CreateCollection, Operation::SetSchema]);

    // A peer with the collection, and nothing declared on it.
    let peer = StorageEngine::new(tmp.path().join("b").to_str().unwrap()).unwrap();
    peer.initialize().unwrap();
    peer.get_database("_system")
        .unwrap()
        .create_collection("goods".to_string(), None)
        .unwrap();
    let goods = peer
        .get_database("_system")
        .unwrap()
        .get_collection("goods")
        .unwrap();
    goods.insert(json!({"n": "not a number"})).unwrap();

    let apply = |from: u64| {
        let entries = log.get_entries_after(from, 100);
        for entry in entries.iter().filter(|e| e.collection == "goods") {
            match entry.operation {
                Operation::SetFieldRules => goods
                    .apply_field_rules(entry.data.as_deref().unwrap_or(b"[]"))
                    .unwrap(),
                Operation::SetSchema => goods.apply_schema(entry.data.as_deref()).unwrap(),
                _ => {}
            }
        }
        entries.last().map_or(from, |e| e.sequence)
    };
    let at = apply(0);
    assert_eq!(goods.field_rules().len(), 1);
    refused(goods.insert(json!({"n": 1})), "'name' is required");
    refused(goods.insert(json!({"name": "x", "n": "no"})), "");
    goods.insert(json!({"name": "x", "n": 1})).unwrap();

    // A write the origin accepted is stored as it is, whatever the peer
    // now asks: replication applies it unchecked.
    solidb::storage::foreign_keys::unchecked(|| {
        goods.insert(json!({"_key": "replicated", "n": "older than the schema"}))
    })
    .unwrap();
    assert!(goods.get("replicated").is_ok());

    // Removed on the origin: removed on the peer.
    let (status, _) = call(&app, &token, "DELETE", &rules_url, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, &token, "DELETE", &schema_url, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    apply(at);
    assert!(goods.field_rules().is_empty());
    assert!(goods.get_json_schema().is_none());
    goods.insert(json!({"n": "anything"})).unwrap();
}
