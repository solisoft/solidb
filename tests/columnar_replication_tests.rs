//! A columnar collection must reach the other nodes (card #882).
//!
//! Only the HTTP insert used to be written to the replication log: a peer
//! never learned that the collection existed, so every row it received
//! failed to load it, and nothing sent over the driver was logged at all.
//!
//! Each test writes on one node through a transport, reads what that put in
//! the replication log, and applies it to a second node with the functions
//! the replication worker calls.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use solidb::driver::handlers::database as driver;
use solidb::driver::{DriverHandler, Response};
use solidb::scripting::ScriptStats;
use solidb::storage::columnar::ColumnarIndexType;
use solidb::sync::{columnar, LogEntry, Operation, SyncLog};
use solidb::{create_router, AggregateOp, ColumnarCollection, CompressionType, StorageEngine};
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

/// A node with a database `app` and, when `logging`, a replication log.
fn node(logging: bool) -> (StorageEngine, Option<Arc<SyncLog>>, TempDir) {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(dir.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let log = logging.then(|| {
        let path = dir.path().join("log");
        Arc::new(SyncLog::new("n1".to_string(), path.to_str().unwrap(), 64).unwrap())
    });
    (engine, log, dir)
}

/// What the log holds about `metrics` (the router logs its own writes to
/// `_system` beside it).
fn entries(log: &SyncLog) -> Vec<LogEntry> {
    log.get_entries_after(0, 100_000)
        .into_iter()
        .filter(|e| e.collection == "metrics")
        .collect()
}

/// What the replication worker does with each columnar entry.
fn apply(peer: &StorageEngine, entries: &[LogEntry]) {
    let db = peer.get_database("app").unwrap();
    for e in entries {
        match e.operation {
            Operation::ColumnarCreateCollection => {
                columnar::apply_create(&db, &e.collection, e.data.as_ref().unwrap()).unwrap()
            }
            Operation::ColumnarInsert => {
                columnar::apply_insert(&db, &e.collection, &e.key, e.data.as_ref().unwrap())
                    .unwrap()
            }
            Operation::ColumnarDropCollection => columnar::apply_drop(&db, &e.collection).unwrap(),
            Operation::ColumnarDelete => {
                columnar::apply_delete(&db, &e.collection, &e.key).unwrap()
            }
            Operation::ColumnarTruncate => columnar::apply_truncate(&db, &e.collection).unwrap(),
            Operation::ColumnarCreateIndex => {
                columnar::apply_create_index(&db, &e.collection, &e.key, e.data.as_ref().unwrap())
                    .unwrap()
            }
            Operation::ColumnarDropIndex => {
                columnar::apply_drop_index(&db, &e.collection, &e.key).unwrap()
            }
            ref other => panic!("unexpected entry {other:?}"),
        }
    }
}

fn load(engine: &StorageEngine, name: &str) -> solidb::DbResult<ColumnarCollection> {
    let db = engine.get_database("app").unwrap();
    ColumnarCollection::load(name.to_string(), "app", db.db_arc())
}

fn operations(entries: &[LogEntry]) -> Vec<Operation> {
    entries.iter().map(|e| e.operation).collect()
}

fn rows(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| json!({"sensor": format!("s{}", i % 3), "value": i as i64}))
        .collect()
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
        .header("Authorization", format!("Bearer {}", token));
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
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
async fn http_create_insert_and_drop_reach_a_peer() {
    let (a, log, _da) = node(true);
    let log = log.unwrap();
    let (b, _, _db) = node(false);
    let app = create_router(
        a.clone(),
        None,
        Some(log.clone()),
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    let token = common::seed_user_token(&a, "test_admin", &["admin"]);

    let (status, body) = call(
        &app,
        &token,
        "POST",
        "/_api/database/app/columnar",
        Some(json!({"name": "metrics", "columns": [
            {"name": "sensor", "type": "string"},
            {"name": "value", "type": "int64"},
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(
        &app,
        &token,
        "POST",
        "/_api/database/app/columnar/metrics/insert",
        Some(json!({"rows": rows(10)})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<String> = serde_json::from_value(body["ids"].clone()).unwrap();

    let logged = entries(&log);
    assert_eq!(logged[0].operation, Operation::ColumnarCreateCollection);
    assert_eq!(logged.len(), 11, "one create, then a row each");
    assert_eq!(
        logged[1..]
            .iter()
            .map(|e| e.key.clone())
            .collect::<Vec<_>>(),
        ids,
        "rows are logged under the ids the origin gave them"
    );

    apply(&b, &logged);
    let on_b = load(&b, "metrics").expect("the peer has the collection");
    assert_eq!(on_b.count(), 10);
    assert_eq!(
        on_b.aggregate("value", AggregateOp::Sum).unwrap(),
        json!(45.0)
    );
    assert_eq!(on_b.list_row_uuids().unwrap(), ids);

    let (status, body) = call(
        &app,
        &token,
        "DELETE",
        "/_api/database/app/columnar/metrics",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let logged = entries(&log);
    assert_eq!(
        logged.last().unwrap().operation,
        Operation::ColumnarDropCollection
    );
    apply(&b, &logged[11..]);
    assert!(load(&b, "metrics").is_err(), "the peer dropped it too");
}

#[test]
fn driver_create_insert_and_drop_reach_a_peer() {
    let (a, log, _da) = node(true);
    let log = log.unwrap();
    let (b, _, _db) = node(false);
    let handler = DriverHandler::new(Arc::new(a.clone()), Some(log.clone()));
    let ok = |r: Response| assert!(matches!(r, Response::Ok { .. }), "{r:?}");

    ok(driver::handle_create_columnar(
        &handler,
        "app".to_string(),
        "metrics".to_string(),
        vec![
            json!({"name": "sensor", "data_type": "string", "nullable": false, "indexed": false}),
            json!({"name": "value", "data_type": "int64", "nullable": false, "indexed": false}),
        ],
    ));
    ok(driver::handle_insert_columnar(
        &handler,
        "app".to_string(),
        "metrics".to_string(),
        rows(10),
    ));

    let logged = entries(&log);
    assert_eq!(logged.len(), 11);
    assert_eq!(logged[0].operation, Operation::ColumnarCreateCollection);
    apply(&b, &logged);
    let on_a = load(&a, "metrics").unwrap();
    let on_b = load(&b, "metrics").expect("the peer has the collection");
    assert_eq!(on_b.count(), 10);
    assert_eq!(
        on_b.list_row_uuids().unwrap(),
        on_a.list_row_uuids().unwrap()
    );
    assert_eq!(
        on_b.metadata().unwrap().columns.len(),
        on_a.metadata().unwrap().columns.len()
    );

    ok(driver::handle_delete_columnar(
        &handler,
        "app".to_string(),
        "metrics".to_string(),
    ));
    assert!(load(&a, "metrics").is_err());
    let logged = entries(&log);
    assert_eq!(
        operations(&logged[11..]),
        vec![Operation::ColumnarDropCollection]
    );
    apply(&b, &logged[11..]);
    assert!(load(&b, "metrics").is_err(), "the peer dropped it too");
}

/// A joining node replays a log from its start, over what it already has.
#[test]
fn a_replayed_log_changes_nothing() {
    let (a, log, _da) = node(true);
    let log = log.unwrap();
    let (b, _, _db) = node(false);
    let db = a.get_database("app").unwrap();
    let columns = load_columns();
    columnar::create(&db, Some(&log), "metrics", columns, CompressionType::Lz4).unwrap();
    // Enough rows to seal a block, and a few left in the tail.
    columnar::insert(&db, Some(&log), "metrics", rows(5000)).unwrap();
    columnar::insert(&db, Some(&log), "metrics", rows(7)).unwrap();

    let logged = entries(&log);
    assert_eq!(logged.len(), 5008);
    apply(&b, &logged);
    apply(&b, &logged);
    let on_b = load(&b, "metrics").unwrap();
    assert_eq!(on_b.count(), 5007, "a second create did not reset the rows");
    assert_eq!(
        on_b.aggregate("value", AggregateOp::Sum).unwrap(),
        load(&a, "metrics")
            .unwrap()
            .aggregate("value", AggregateOp::Sum)
            .unwrap()
    );

    // Dropped twice is dropped.
    columnar::drop(&db, Some(&log), "metrics").unwrap();
    let logged = entries(&log);
    apply(&b, &logged[5008..]);
    apply(&b, &logged[5008..]);
    assert!(load(&b, "metrics").is_err());
}

/// A log nobody reads takes nothing, and nothing is refused for it.
#[test]
fn a_deferred_log_takes_no_entry() {
    let (a, log, _da) = node(true);
    let log = log.unwrap();
    log.defer_until_needed();
    let db = a.get_database("app").unwrap();
    columnar::create(
        &db,
        Some(&log),
        "metrics",
        load_columns(),
        CompressionType::Lz4,
    )
    .unwrap();
    let ids = columnar::insert(&db, Some(&log), "metrics", rows(3)).unwrap();
    assert_eq!(ids.len(), 3);
    assert!(entries(&log).is_empty());
    columnar::drop(&db, None, "metrics").unwrap();
}

fn load_columns() -> Vec<solidb::ColumnDef> {
    serde_json::from_value(json!([
        {"name": "sensor", "data_type": "string", "nullable": false, "indexed": false},
        {"name": "value", "data_type": "int64", "nullable": false, "indexed": false},
    ]))
    .unwrap()
}

/// The peer stores the collection the way the origin was asked to.
#[test]
fn the_compression_setting_reaches_a_peer() {
    let (a, log, _da) = node(true);
    let log = log.unwrap();
    let (b, _, _db) = node(false);
    let db = a.get_database("app").unwrap();
    columnar::create(
        &db,
        Some(&log),
        "metrics",
        load_columns(),
        CompressionType::None,
    )
    .unwrap();
    apply(&b, &entries(&log));
    assert_eq!(
        load(&b, "metrics").unwrap().metadata().unwrap().compression,
        CompressionType::None
    );
}

/// An index made or dropped on one node is made or dropped on the others,
/// built there from the rows that node has.
#[tokio::test]
async fn indexes_reach_a_peer() {
    let (a, log, _da) = node(true);
    let log = log.unwrap();
    let (b, _, _db) = node(false);
    let app = create_router(
        a.clone(),
        None,
        Some(log.clone()),
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    let token = common::seed_user_token(&a, "test_admin", &["admin"]);
    let db = a.get_database("app").unwrap();
    columnar::create(
        &db,
        Some(&log),
        "metrics",
        load_columns(),
        CompressionType::Lz4,
    )
    .unwrap();
    columnar::insert(&db, Some(&log), "metrics", rows(30)).unwrap();

    // Over HTTP, with a type; over the driver, which names none.
    let (status, body) = call(
        &app,
        &token,
        "POST",
        "/_api/database/app/columnar/metrics/index",
        Some(json!({"column": "sensor", "index_type": "bitmap"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let handler = DriverHandler::new(Arc::new(a.clone()), Some(log.clone()));
    let created = driver::handle_create_columnar_index(
        &handler,
        "app".to_string(),
        "metrics".to_string(),
        "value".to_string(),
    );
    assert!(matches!(created, Response::Ok { .. }), "{created:?}");

    let logged = entries(&log);
    apply(&b, &logged);
    // Replayed: nothing to do, and no error.
    apply(&b, &logged);
    let on_b = load(&b, "metrics").unwrap();
    assert_eq!(
        on_b.get_index_type("sensor"),
        Some(ColumnarIndexType::Bitmap)
    );
    assert_eq!(
        on_b.get_index_type("value"),
        Some(ColumnarIndexType::Sorted)
    );
    // The peer's index answers from the peer's rows.
    let filter = solidb::ColumnFilter::Eq("sensor".to_string(), json!("s1"));
    assert_eq!(
        on_b.scan_filtered(&filter, &["value"]).unwrap().len(),
        load(&a, "metrics")
            .unwrap()
            .scan_filtered(&filter, &["value"])
            .unwrap()
            .len()
    );
    assert_eq!(on_b.scan_filtered(&filter, &["value"]).unwrap().len(), 10);

    let (status, body) = call(
        &app,
        &token,
        "DELETE",
        "/_api/database/app/columnar/metrics/index/sensor",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let dropped = driver::handle_delete_columnar_index(
        &handler,
        "app".to_string(),
        "metrics".to_string(),
        "value".to_string(),
    );
    assert!(matches!(dropped, Response::Ok { .. }), "{dropped:?}");
    let rest = entries(&log);
    let rest = &rest[logged.len()..];
    assert_eq!(
        operations(rest),
        vec![Operation::ColumnarDropIndex, Operation::ColumnarDropIndex]
    );
    apply(&b, rest);
    apply(&b, rest);
    let on_b = load(&b, "metrics").unwrap();
    assert!(!on_b.has_index("sensor") && !on_b.has_index("value"));
}

/// Every value of `value` a node holds, ascending.
fn values(engine: &StorageEngine) -> Vec<i64> {
    let mut values: Vec<i64> = load(engine, "metrics")
        .unwrap()
        .read_column("value", None)
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    values.sort();
    values
}

/// Rows deleted by id, by filter, and a truncate, over HTTP (card #885):
/// done on the node, and on a peer that applies its log.
#[tokio::test]
async fn http_deletes_and_truncate_reach_a_peer() {
    let (a, log, _da) = node(true);
    let log = log.unwrap();
    let (b, _, _db) = node(false);
    let app = create_router(
        a.clone(),
        None,
        Some(log.clone()),
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    let token = common::seed_user_token(&a, "test_admin", &["admin"]);
    let db = a.get_database("app").unwrap();
    columnar::create(
        &db,
        Some(&log),
        "metrics",
        load_columns(),
        CompressionType::Lz4,
    )
    .unwrap();
    // A block of 5,000 and 20 rows in the tail.
    let mut ids = columnar::insert(&db, Some(&log), "metrics", rows(5000)).unwrap();
    ids.extend(columnar::insert(&db, Some(&log), "metrics", rows(5020)[5000..].to_vec()).unwrap());
    let mut applied = 0;
    let mut catch_up = |b: &StorageEngine| {
        let logged = entries(&log);
        apply(b, &logged[applied..]);
        applied = logged.len();
    };
    catch_up(&b);
    assert_eq!(values(&b).len(), 5020);

    // One row, by id: the first of the block.
    let uri = format!("/_api/database/app/columnar/metrics/row/{}", ids[0]);
    let (status, body) = call(&app, &token, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "deleted");
    // Gone: a second delete finds nothing.
    let (status, _) = call(&app, &token, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // By ids: two of the block, one of the tail, one that does not exist.
    let (status, body) = call(
        &app,
        &token,
        "POST",
        "/_api/database/app/columnar/metrics/delete",
        Some(json!({"ids": [ids[10], ids[11], ids[5005], "no-such-row"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], 3);

    // By filter, with an `and`: values 100..200 of sensor s1.
    let (status, body) = call(
        &app,
        &token,
        "POST",
        "/_api/database/app/columnar/metrics/delete",
        Some(json!({"filter": {"and": [
            {"column": "sensor", "op": "EQ", "value": "s1"},
            {"column": "value", "op": "GTE", "value": 100},
            {"column": "value", "op": "LT", "value": 200},
        ]}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], 34);

    // Neither, or both: refused, nothing deleted.
    for bad in [
        json!({}),
        json!({"ids": [], "filter": {"column": "value", "op": "GT", "value": 0}}),
    ] {
        let (status, _) = call(
            &app,
            &token,
            "POST",
            "/_api/database/app/columnar/metrics/delete",
            Some(bad),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    let expected: Vec<i64> = (0..5020)
        .filter(|v| ![0, 10, 11, 5005].contains(v) && !(v % 3 == 1 && (100..200).contains(v)))
        .collect();
    assert_eq!(values(&a), expected);
    assert_eq!(load(&a, "metrics").unwrap().count(), expected.len());
    catch_up(&b);
    assert_eq!(values(&b), expected);
    assert_eq!(
        load(&b, "metrics").unwrap().list_row_uuids().unwrap(),
        load(&a, "metrics").unwrap().list_row_uuids().unwrap()
    );
    // What is left still aggregates from the blocks.
    assert_eq!(
        load(&a, "metrics")
            .unwrap()
            .aggregate("value", AggregateOp::Sum)
            .unwrap(),
        json!(expected.iter().sum::<i64>() as f64)
    );

    let (status, body) = call(
        &app,
        &token,
        "PUT",
        "/_api/database/app/columnar/metrics/truncate",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(values(&a).is_empty());
    catch_up(&b);
    assert!(values(&b).is_empty());
    // The columns stay, and rows go in again.
    columnar::insert(&db, Some(&log), "metrics", rows(3)).unwrap();
    catch_up(&b);
    assert_eq!(values(&b), [0, 1, 2]);
}

/// The same over the driver.
#[test]
fn driver_deletes_and_truncate_reach_a_peer() {
    let (a, log, _da) = node(true);
    let log = log.unwrap();
    let (b, _, _db) = node(false);
    let handler = DriverHandler::new(Arc::new(a.clone()), Some(log.clone()));
    let db = a.get_database("app").unwrap();
    columnar::create(
        &db,
        Some(&log),
        "metrics",
        load_columns(),
        CompressionType::Lz4,
    )
    .unwrap();
    let ids = columnar::insert(&db, Some(&log), "metrics", rows(30)).unwrap();
    let count = |r: Response| match r {
        Response::Ok { count, .. } => count,
        other => panic!("{other:?}"),
    };

    let by_ids = driver::handle_delete_columnar_rows(
        &handler,
        "app".to_string(),
        "metrics".to_string(),
        Some(vec![ids[0].clone(), ids[29].clone()]),
        None,
    );
    assert_eq!(count(by_ids), Some(2));
    let by_filter = driver::handle_delete_columnar_rows(
        &handler,
        "app".to_string(),
        "metrics".to_string(),
        None,
        Some(r#"{"column": "value", "op": "LT", "value": 10}"#.to_string()),
    );
    assert_eq!(count(by_filter), Some(9));
    let neither = driver::handle_delete_columnar_rows(
        &handler,
        "app".to_string(),
        "metrics".to_string(),
        None,
        None,
    );
    assert!(matches!(neither, Response::Error { .. }));

    apply(&b, &entries(&log));
    assert_eq!(values(&a), (10..29).collect::<Vec<i64>>());
    assert_eq!(values(&b), values(&a));

    let truncated =
        driver::handle_truncate_columnar(&handler, "app".to_string(), "metrics".to_string());
    assert!(matches!(truncated, Response::Ok { .. }), "{truncated:?}");
    let logged = entries(&log);
    assert_eq!(
        logged.last().unwrap().operation,
        Operation::ColumnarTruncate
    );
    apply(&b, &logged[logged.len() - 1..]);
    assert!(values(&a).is_empty() && values(&b).is_empty());
}

/// `POST …/query` takes what the driver's query takes, and an offset
/// (card #895): columns left out, a filter with `and`, an order, a page.
#[tokio::test]
async fn http_query_orders_and_pages() {
    let (a, _, _da) = node(false);
    let app = create_router(
        a.clone(),
        None,
        None,
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    let token = common::seed_user_token(&a, "test_admin", &["admin"]);
    let db = a.get_database("app").unwrap();
    columnar::create(&db, None, "metrics", load_columns(), CompressionType::Lz4).unwrap();
    columnar::insert(&db, None, "metrics", rows(20)).unwrap();
    let query = |body: Value| {
        let (app, token) = (app.clone(), token.clone());
        async move {
            call(
                &app,
                &token,
                "POST",
                "/_api/database/app/columnar/metrics/query",
                Some(body),
            )
            .await
        }
    };
    let values = |body: &Value| -> Vec<i64> {
        body["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["value"].as_i64().unwrap())
            .collect()
    };

    // No columns: all of them.
    let (status, body) = query(json!({"limit": 1})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"], json!([{"sensor": "s0", "value": 0}]));

    let (_, body) =
        query(json!({"columns": ["value"], "order_by": "value DESC", "limit": 3})).await;
    assert_eq!(values(&body), [19, 18, 17]);
    let (_, body) = query(json!({
        "columns": ["value"], "order_by": "value DESC", "offset": 3, "limit": 3
    }))
    .await;
    assert_eq!(values(&body), [16, 15, 14]);
    // Past the end: nothing, and no error.
    let (_, body) = query(json!({"columns": ["value"], "offset": 50})).await;
    assert_eq!(body["count"], 0);

    let (_, body) = query(json!({
        "columns": ["value"],
        "filter": {"and": [
            {"column": "sensor", "op": "EQ", "value": "s1"},
            {"column": "value", "op": "GT", "value": 9},
        ]},
        "order_by": "value",
        "offset": 1,
    }))
    .await;
    assert_eq!(values(&body), [13, 16, 19]);

    let (status, _) = query(json!({"columns": ["value"], "order_by": "sensor"})).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an order on a column not returned"
    );
}
