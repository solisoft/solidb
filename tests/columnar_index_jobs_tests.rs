//! Columnar indexes on a table that keeps changing: built in the
//! background and followed, resumed after a restart, kept when the oldest
//! rows are removed, and read by aggregates that take a filter.
//!
//! Every answer an index gives is compared with the rows themselves, read
//! whole and filtered here.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::storage::columnar::{BuildState, ColumnarIndexType, GroupByColumn, IndexBuild};
use solidb::sync::columnar;
use solidb::{
    create_router, AggregateOp, ColumnDef, ColumnFilter, ColumnType, ColumnarCollection,
    CompressionType, StorageEngine,
};
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

const KINDS: [ColumnarIndexType; 4] = [
    ColumnarIndexType::Sorted,
    ColumnarIndexType::Hash,
    ColumnarIndexType::Bitmap,
    ColumnarIndexType::Bloom,
];

fn open(dir: &TempDir) -> StorageEngine {
    let engine = StorageEngine::new(dir.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine
}

/// A collection `metrics` of `rows` rows: `metric` cycles over 50 values,
/// `day` grows with the row, `value` is a number the test can compute.
fn fixture(engine: &StorageEngine, rows: i64) -> ColumnarCollection {
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    let column = |name: &str, data_type| ColumnDef {
        name: name.to_string(),
        data_type,
        nullable: false,
        indexed: false,
        index_type: None,
    };
    let columns = vec![
        column("metric", ColumnType::Int64),
        column("host", ColumnType::String),
        column("day", ColumnType::Int64),
        column("value", ColumnType::Int64),
    ];
    let col = columnar::create(&db, None, "metrics", columns, CompressionType::Lz4).unwrap();
    col.insert_rows(some_rows(0, rows)).unwrap();
    col
}

fn some_rows(from: i64, to: i64) -> Vec<Value> {
    (from..to)
        .map(|i| {
            json!({"metric": i % 50, "host": format!("h{}", i % 7), "day": i / 1000, "value": (i * 31) % 100})
        })
        .collect()
}

fn load(engine: &StorageEngine) -> ColumnarCollection {
    let db = engine.get_database("app").unwrap();
    ColumnarCollection::load("metrics".to_string(), "app", db.db_arc()).unwrap()
}

const COLUMNS: [&str; 4] = ["metric", "host", "day", "value"];

fn filters() -> Vec<ColumnFilter> {
    vec![
        ColumnFilter::Eq("metric".to_string(), json!(42)),
        ColumnFilter::Eq("metric".to_string(), json!(4_200)),
        ColumnFilter::In("metric".to_string(), vec![json!(1), json!(49)]),
        ColumnFilter::Eq("host".to_string(), json!("h3")),
        ColumnFilter::And(vec![
            ColumnFilter::Eq("metric".to_string(), json!(7)),
            ColumnFilter::Gte("day".to_string(), json!(60)),
        ]),
        ColumnFilter::Or(vec![
            ColumnFilter::Eq("metric".to_string(), json!(7)),
            ColumnFilter::Eq("host".to_string(), json!("h0")),
        ]),
    ]
}

/// Whether `filter` keeps `row`, worked out here.
fn keeps(filter: &ColumnFilter, row: &Value) -> bool {
    match filter {
        ColumnFilter::Eq(column, wanted) => &row[column] == wanted,
        ColumnFilter::In(column, wanted) => wanted.contains(&row[column]),
        ColumnFilter::Gte(column, bound) => row[column].as_i64() >= bound.as_i64(),
        ColumnFilter::Lt(column, bound) => row[column].as_i64() < bound.as_i64(),
        ColumnFilter::And(sides) => sides.iter().all(|side| keeps(side, row)),
        ColumnFilter::Or(sides) => sides.iter().any(|side| keeps(side, row)),
        other => panic!("not used here: {other:?}"),
    }
}

/// Each filter answers what the rows, read whole, say.
fn answers_as_the_rows(col: &ColumnarCollection, when: &str) {
    let all = col.read_columns(&COLUMNS, None).unwrap();
    for filter in filters() {
        let expected: Vec<Value> = all
            .iter()
            .filter(|row| keeps(&filter, row))
            .cloned()
            .collect();
        let got = col.scan_filtered(&filter, &COLUMNS).unwrap();
        assert_eq!(got.len(), expected.len(), "{when}: {filter:?}");
        assert_eq!(got, expected, "{when}: {filter:?}");
        assert_eq!(
            col.count_filtered(&filter).unwrap(),
            expected.len() as u64,
            "{when}: count of {filter:?}"
        );
    }
}

fn finished(build: &IndexBuild) -> BuildState {
    let start = Instant::now();
    while build.state() == BuildState::Building {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "the build never ended"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    build.state()
}

#[test]
fn an_index_built_in_the_background_answers_as_the_rows() {
    let dir = TempDir::new().unwrap();
    let engine = open(&dir);
    let col = fixture(&engine, 150_000);
    col.insert_rows(some_rows(150_000, 150_010)).unwrap();
    answers_as_the_rows(&col, "before any index");

    let build = col
        .start_index_build("metric", ColumnarIndexType::Sorted, false)
        .unwrap();
    assert_eq!(build.column, "metric");
    // Asked again while it runs, or once it is there.
    match col.start_index_build("metric", ColumnarIndexType::Sorted, false) {
        Ok(same) => assert_eq!(same.id, build.id),
        Err(refused) => assert!(refused.to_string().contains("already has an index")),
    }
    assert_eq!(finished(&build), BuildState::Done);
    assert_eq!(build.progress(), (150_010, 150_010));

    let col = load(&engine);
    assert_eq!(
        col.get_index_type("metric"),
        Some(ColumnarIndexType::Sorted)
    );
    assert_eq!(col.index_coverage("metric"), 150_010);
    assert!(col.metadata().unwrap().index_builds.is_empty());
    let listed = col.list_indexes().unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].size_bytes > 0);
    // Packed: under two bytes a row, where JSON took seven.
    assert!(
        listed[0].size_bytes < 2 * 150_010,
        "{}",
        listed[0].size_bytes
    );
    answers_as_the_rows(&col, "with the index");
    assert!(col
        .start_index_build("metric", ColumnarIndexType::Sorted, false)
        .is_err());
}

#[test]
fn a_build_a_restart_interrupted_is_resumed() {
    let dir = TempDir::new().unwrap();
    let id = {
        let engine = open(&dir);
        let col = fixture(&engine, 80_000);
        // Accepted and written down; the process stops before it is built.
        let build = col
            .mark_index_build("host", ColumnarIndexType::Hash)
            .unwrap();
        assert!(!col.has_index("host"));
        assert_eq!(col.metadata().unwrap().index_builds["host"].id, build.id);
        build.id.clone()
    };

    let engine = open(&dir);
    // Loading the collection finds the build and starts it again.
    let col = load(&engine);
    let build = col.index_build("host").expect("the build is resumed");
    assert_eq!(build.id, id);
    assert_eq!(finished(&build), BuildState::Done);
    let col = load(&engine);
    assert_eq!(col.get_index_type("host"), Some(ColumnarIndexType::Hash));
    assert_eq!(col.index_coverage("host"), 80_000);
    assert!(col.metadata().unwrap().index_builds.is_empty());
    answers_as_the_rows(&col, "after the resumed build");
    // Loaded again: nothing left to resume.
    assert_eq!(load(&engine).index_build("host").unwrap().id, id);
}

#[test]
fn a_build_stopped_with_the_database_is_built_by_the_next_process() {
    let dir = TempDir::new().unwrap();
    {
        let engine = open(&dir);
        let col = fixture(&engine, 200_000);
        col.start_index_build("metric", ColumnarIndexType::Sorted, false)
            .unwrap();
        // The engine goes while the build may still be running: its
        // thread is stopped and joined, built or not.
    }
    let engine = open(&dir);
    let col = load(&engine);
    if let Some(build) = col.index_build("metric") {
        assert_eq!(finished(&build), BuildState::Done);
    }
    let col = load(&engine);
    assert_eq!(
        col.get_index_type("metric"),
        Some(ColumnarIndexType::Sorted)
    );
    assert!(col.metadata().unwrap().index_builds.is_empty());
    answers_as_the_rows(&col, "after the restart");
}

#[test]
fn writes_made_while_an_index_is_built_are_answered_for() {
    let dir = TempDir::new().unwrap();
    let engine = open(&dir);
    let col = fixture(&engine, 200_000);
    let build = col
        .start_index_build("metric", ColumnarIndexType::Sorted, false)
        .unwrap();
    // Rows after the others, rows out of the middle, the first rows.
    let writer = load(&engine);
    writer.insert_rows(some_rows(200_000, 205_000)).unwrap();
    writer
        .delete_where(&ColumnFilter::Eq("day".to_string(), json!(100)))
        .unwrap();
    writer.insert_rows(some_rows(205_000, 205_003)).unwrap();
    writer
        .delete_where(&ColumnFilter::Lt("day".to_string(), json!(3)))
        .unwrap();
    assert_eq!(finished(&build), BuildState::Done);

    let col = load(&engine);
    assert_eq!(col.count(), 205_003 - 1_000 - 3_000);
    assert!(col.has_index("metric"));
    answers_as_the_rows(&col, "after the build and the writes");
}

#[test]
fn dropping_an_index_calls_its_build_off() {
    let dir = TempDir::new().unwrap();
    let engine = open(&dir);
    let col = fixture(&engine, 100_000);
    let build = col
        .mark_index_build("metric", ColumnarIndexType::Sorted)
        .unwrap();
    assert!(!col.has_index("metric"));
    // A build that was accepted can be dropped before any index exists.
    col.drop_index("metric").unwrap();
    assert!(col.metadata().unwrap().index_builds.is_empty());
    assert!(load(&engine).index_build("metric").is_none());
    drop(build);

    let build = col
        .start_index_build("metric", ColumnarIndexType::Sorted, false)
        .unwrap();
    // Built or still building: either way nothing is left.
    let _ = load(&engine).drop_index("metric");
    assert_ne!(finished(&build), BuildState::Building);
    let col = load(&engine);
    if col.has_index("metric") {
        // The build ended before the drop saw it, and the drop was refused.
        col.drop_index("metric").unwrap();
    }
    let col = load(&engine);
    assert!(!col.has_index("metric"));
    assert!(col.metadata().unwrap().index_builds.is_empty());
    assert!(col.drop_index("metric").is_err());
    answers_as_the_rows(&col, "after the drop");
}

#[test]
fn an_index_survives_the_removal_of_the_oldest_rows() {
    for kind in KINDS {
        let dir = TempDir::new().unwrap();
        let engine = open(&dir);
        let col = fixture(&engine, 140_000);
        col.create_index("metric", kind.clone()).unwrap();
        col.create_index("host", kind.clone()).unwrap();
        assert_eq!(col.index_coverage("metric"), 140_000);

        // Everything before a day: exactly the first rows, one whole
        // block and part of the next.
        let deleted = col
            .delete_where(&ColumnFilter::Lt("day".to_string(), json!(70)))
            .unwrap();
        assert_eq!(deleted.len(), 70_000);
        let col = load(&engine);
        assert_eq!(col.index_coverage("metric"), 70_000, "{kind:?}");
        assert_eq!(col.index_coverage("host"), 70_000, "{kind:?}");
        answers_as_the_rows(&col, &format!("{kind:?}, after the oldest rows went"));

        // Rows after the others are compared in their blocks.
        col.insert_rows(some_rows(140_000, 146_000)).unwrap();
        assert_eq!(col.index_coverage("metric"), 70_000);
        answers_as_the_rows(&col, &format!("{kind:?}, after an insert"));

        // The oldest again, twice: the bases add up.
        col.delete_where(&ColumnFilter::Lt("day".to_string(), json!(75)))
            .unwrap();
        col.delete_where(&ColumnFilter::Lt("day".to_string(), json!(76)))
            .unwrap();
        let col = load(&engine);
        assert_eq!(col.index_coverage("metric"), 64_000, "{kind:?}");
        answers_as_the_rows(&col, &format!("{kind:?}, after two more"));

        // Rows out of the middle: the index answers for those before them.
        col.delete_where(&ColumnFilter::Eq("day".to_string(), json!(100)))
            .unwrap();
        let col = load(&engine);
        assert_eq!(col.index_coverage("metric"), 24_000, "{kind:?}");
        answers_as_the_rows(&col, &format!("{kind:?}, after a delete in the middle"));

        // Built again: it holds every row.
        col.rebuild_index("metric", kind.clone()).unwrap();
        let col = load(&engine);
        assert_eq!(col.index_coverage("metric"), col.count() as u64);
        answers_as_the_rows(&col, &format!("{kind:?}, rebuilt"));

        // More than it holds: nothing left to answer for.
        col.delete_where(&ColumnFilter::Lt("day".to_string(), json!(200)))
            .unwrap();
        let col = load(&engine);
        assert_eq!(col.count(), 0);
        assert_eq!(col.index_coverage("host"), 0);
        col.insert_rows(some_rows(0, 5_000)).unwrap();
        answers_as_the_rows(&col, &format!("{kind:?}, emptied and filled"));
    }
}

/// `op` of `value` over `rows`, as the collection reports it.
fn aggregate_of(rows: &[&Value], op: AggregateOp) -> Value {
    let numbers: Vec<f64> = rows
        .iter()
        .filter_map(|row| row["value"].as_f64())
        .collect();
    match op {
        AggregateOp::Count => json!(rows.len()),
        AggregateOp::Sum => json!(numbers.iter().sum::<f64>()),
        AggregateOp::Avg if numbers.is_empty() => Value::Null,
        AggregateOp::Avg => json!(numbers.iter().sum::<f64>() / numbers.len() as f64),
        AggregateOp::Min => numbers
            .iter()
            .copied()
            .reduce(f64::min)
            .map_or(Value::Null, |n| json!(n)),
        AggregateOp::Max => numbers
            .iter()
            .copied()
            .reduce(f64::max)
            .map_or(Value::Null, |n| json!(n)),
        AggregateOp::CountDistinct => {
            let distinct: std::collections::HashSet<String> =
                rows.iter().map(|row| row["value"].to_string()).collect();
            json!(distinct.len())
        }
    }
}

const OPS: [AggregateOp; 6] = [
    AggregateOp::Count,
    AggregateOp::Sum,
    AggregateOp::Avg,
    AggregateOp::Min,
    AggregateOp::Max,
    AggregateOp::CountDistinct,
];

#[test]
fn aggregates_take_a_filter_and_answer_as_the_rows() {
    let dir = TempDir::new().unwrap();
    let engine = open(&dir);
    let col = fixture(&engine, 70_000);
    // Some in the tail too.
    col.insert_rows(some_rows(70_000, 70_100)).unwrap();
    for indexed in [false, true] {
        if indexed {
            col.create_index("metric", ColumnarIndexType::Sorted)
                .unwrap();
        }
        let all = col.read_columns(&COLUMNS, None).unwrap();
        let mut asked: Vec<Option<ColumnFilter>> = filters().into_iter().map(Some).collect();
        asked.push(None);
        asked.push(Some(ColumnFilter::Gte("day".to_string(), json!(40))));
        for filter in &asked {
            let kept: Vec<&Value> = all
                .iter()
                .filter(|row| filter.as_ref().is_none_or(|f| keeps(f, row)))
                .collect();
            for op in OPS {
                let got = match filter {
                    Some(filter) => col.aggregate_where(filter, "value", op).unwrap(),
                    None => col.aggregate("value", op).unwrap(),
                };
                assert_eq!(got, aggregate_of(&kept, op), "{op:?} where {filter:?}");

                // By metric and host: one group per pair, in the order
                // their first row comes, numbers back as floats.
                let by = [
                    GroupByColumn::Simple("metric".to_string()),
                    GroupByColumn::Simple("host".to_string()),
                ];
                let groups = match filter {
                    Some(filter) => col.group_by_where(filter, &by, "value", op).unwrap(),
                    None => col.group_by(&by, "value", op).unwrap(),
                };
                let mut expected: Vec<((i64, String), Vec<&Value>)> = Vec::new();
                for row in &kept {
                    let key = (
                        row["metric"].as_i64().unwrap(),
                        row["host"].as_str().unwrap().to_string(),
                    );
                    match expected.iter_mut().find(|(held, _)| *held == key) {
                        Some((_, rows)) => rows.push(row),
                        None => expected.push((key, vec![row])),
                    }
                }
                assert_eq!(groups.len(), expected.len(), "{op:?} where {filter:?}");
                for (group, ((metric, host), rows)) in groups.iter().zip(&expected) {
                    assert_eq!(
                        group,
                        &json!({"metric": *metric as f64, "host": host, "_agg": aggregate_of(rows, op)}),
                        "{op:?} where {filter:?}"
                    );
                }
            }
        }
    }
    // A column that does not exist is refused, not read as nulls.
    assert!(col.aggregate("nope", AggregateOp::Sum).is_err());
    assert!(col
        .group_by(
            &[GroupByColumn::Simple("nope".to_string())],
            "value",
            AggregateOp::Sum
        )
        .is_err());
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
    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn the_endpoints_build_in_the_background_and_aggregate_with_a_filter() {
    let dir = TempDir::new().unwrap();
    let engine = open(&dir);
    let col = fixture(&engine, 70_000);
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
    let token = common::seed_user_token(&engine, "test_admin", &["admin"]);
    let base = "/_api/database/app/columnar/metrics";

    // Nothing yet.
    let (status, _) = call(&app, &token, "GET", &format!("{base}/index/metric"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Answered at once, with the build to follow.
    let (status, body) = call(
        &app,
        &token,
        "POST",
        &format!("{base}/index?wait=false"),
        Some(json!({"column": "metric", "index_type": "sorted"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["status"], "building");
    assert_eq!(body["column"], "metric");
    assert_eq!(body["index_type"], "sorted");
    let job = body["job"].as_str().unwrap().to_string();
    assert!(!job.is_empty());

    let start = Instant::now();
    let built = loop {
        let (status, body) = call(&app, &token, "GET", &format!("{base}/index/metric"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["job"], json!(job), "{body}");
        if body["status"] == "created" {
            break body;
        }
        assert_eq!(body["status"], "building", "{body}");
        assert!(body["rows_done"].as_u64() <= body["rows_total"].as_u64());
        assert!(start.elapsed() < Duration::from_secs(60));
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(built["rows"], 70_000);
    assert!(built["size_bytes"].as_u64().unwrap() > 0);

    // Without `wait=false`: as it always answered, once the index is built.
    let (status, body) = call(
        &app,
        &token,
        "POST",
        &format!("{base}/index"),
        Some(json!({"column": "host", "index_type": "hash"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"status": "created", "column": "host", "index_type": "hash"})
    );
    let (status, body) = call(
        &app,
        &token,
        "POST",
        &format!("{base}/index"),
        Some(json!({"column": "host", "index_type": "hash"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // Rows inserted since are not under the index until it is built again.
    col.insert_rows(some_rows(70_000, 75_000)).unwrap();
    let (_, body) = call(&app, &token, "GET", &format!("{base}/indexes"), None).await;
    assert_eq!(body["count"], 2, "{body}");
    assert_eq!(body["builds"], json!([]), "{body}");
    for index in body["indexes"].as_array().unwrap() {
        assert_eq!(index["rows"], 70_000, "{body}");
        // And the listing says how far behind the index is.
        assert_eq!(index["collection_rows"], 75_000, "{body}");
        assert_eq!(index["missing"], 5_000, "{body}");
    }
    let (status, body) = call(
        &app,
        &token,
        "POST",
        &format!("{base}/index?rebuild=true"),
        Some(json!({"column": "host", "index_type": "hash"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, body) = call(&app, &token, "GET", &format!("{base}/index/host"), None).await;
    assert_eq!(body["status"], "created", "{body}");
    assert_eq!(body["rows"], 75_000, "{body}");
    assert_eq!(body["missing"], 0, "{body}");

    // An aggregate over the rows a filter keeps.
    let all = load(&engine).read_columns(&COLUMNS, None).unwrap();
    let filter = json!({"and": [
        {"column": "metric", "op": "EQ", "value": 7},
        {"column": "day", "op": "GTE", "value": 60},
    ]});
    let kept: Vec<&Value> = all
        .iter()
        .filter(|row| row["metric"] == 7 && row["day"].as_i64() >= Some(60))
        .collect();
    assert!(!kept.is_empty());
    let (status, body) = call(
        &app,
        &token,
        "POST",
        &format!("{base}/aggregate"),
        Some(json!({"column": "value", "operation": "AVG", "filter": filter})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"], aggregate_of(&kept, AggregateOp::Avg));

    let (status, body) = call(
        &app,
        &token,
        "POST",
        &format!("{base}/aggregate"),
        Some(
            json!({"column": "value", "operation": "SUM", "group_by": ["host"], "filter": filter}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let groups = body["groups"].as_array().unwrap();
    let hosts: std::collections::BTreeSet<&str> = kept
        .iter()
        .map(|row| row["host"].as_str().unwrap())
        .collect();
    assert_eq!(groups.len(), hosts.len(), "{body}");
    for group in groups {
        let of_host: Vec<&Value> = kept
            .iter()
            .copied()
            .filter(|row| row["host"] == group["host"])
            .collect();
        assert_eq!(
            group["_agg"],
            aggregate_of(&of_host, AggregateOp::Sum),
            "{body}"
        );
    }

    // A filter that is not one is refused, where it used to be ignored.
    let (status, _) = call(
        &app,
        &token,
        "POST",
        &format!("{base}/aggregate"),
        Some(json!({"column": "value", "operation": "AVG", "filter": {"column": "metric"}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
