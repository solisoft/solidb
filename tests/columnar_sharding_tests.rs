//! A sharded columnar collection on one node (card #899): every shard is
//! local, so no network is involved. Its rows go to the shard their key
//! hashes to, and every query answers what the same query answers on one
//! collection holding the same rows.

use serde_json::{json, Value};
use solidb::sdbql::{parse, QueryExecutor};
use solidb::sharding::columnar as sharded;
use solidb::sync::columnar;
use solidb::{ColumnDef, ColumnType, ColumnarCollection, CompressionType, StorageEngine};
use std::collections::HashMap;
use tempfile::TempDir;

fn columns() -> Vec<ColumnDef> {
    let column = |name: &str, data_type| ColumnDef {
        name: name.to_string(),
        data_type,
        nullable: true,
        indexed: false,
        index_type: None,
    };
    vec![
        column("host", ColumnType::String),
        column("v", ColumnType::Int64),
        column("ms", ColumnType::Float64),
    ]
}

fn rows(from: i64, to: i64) -> Vec<Value> {
    (from..to)
        .map(|i| json!({"host": format!("h{}", i % 11), "v": i, "ms": i as f64 / 3.0}))
        .collect()
}

fn run(engine: &StorageEngine, query: &str) -> Result<Vec<Value>, String> {
    QueryExecutor::with_database_and_bind_vars(engine, "app".to_string(), HashMap::new())
        .execute(&parse(query).expect("query parses"))
        .map_err(|e| e.to_string())
}

const QUERIES: &[&str] = &[
    "FOR r IN $c COLLECT h = r.host AGGREGATE n = COUNT(1), s = SUM(r.v), a = AVG(r.ms), lo = MIN(r.v), hi = MAX(r.ms) RETURN {h, n, s, a, lo, hi}",
    "FOR r IN $c FILTER r.v >= 100 AND r.v < 140 SORT r.v RETURN r",
    "FOR r IN $c FILTER r.host == 'h3' COLLECT WITH COUNT INTO n RETURN n",
    "RETURN LENGTH(FOR r IN $c RETURN 1)",
    "FOR r IN $c COLLECT AGGREGATE d = COUNT_DISTINCT(r.host), m = MEDIAN(r.v) RETURN {d, m}",
];

fn answers(engine: &StorageEngine, name: &str) -> Vec<Vec<Value>> {
    QUERIES
        .iter()
        .map(|q| {
            let q = q.replace("$c", name);
            run(engine, &q).unwrap_or_else(|e| panic!("{q}: {e}"))
        })
        .collect()
}

#[test]
fn a_sharded_collection_answers_as_one_collection() {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(dir.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();

    let m = sharded::create(
        None,
        &db,
        None,
        "m",
        columns(),
        CompressionType::Lz4,
        4,
        "host",
        1,
    )
    .unwrap();
    assert_eq!(m.sharding().unwrap().num_shards, 4);
    let plain = columnar::create(&db, None, "plain", columns(), CompressionType::Lz4).unwrap();

    // Through the storage call, and through SDBQL.
    let ids = sharded::insert(None, &db, "m", rows(0, 9_000)).unwrap();
    assert_eq!(ids.len(), 9_000);
    plain.insert_rows(rows(0, 9_000)).unwrap();
    run(
        &engine,
        "FOR i IN 9000..9049 INSERT {host: CONCAT('h', i % 11), v: i, ms: i / 3} INTO m",
    )
    .unwrap();
    plain.insert_rows(rows(9_000, 9_050)).unwrap();

    assert_eq!(answers(&engine, "m"), answers(&engine, "plain"));

    // The rows are in the shards, each host in one of them.
    let mut total = 0;
    let mut where_host: HashMap<String, String> = HashMap::new();
    for shard in 0..4 {
        let name = sharded::physical("m", shard);
        let part = ColumnarCollection::load(name.clone(), "app", db.db_arc()).unwrap();
        total += part.count();
        for (_, row) in part.rows_with_ids().unwrap() {
            let host = row["host"].as_str().unwrap().to_string();
            let before = where_host.insert(host.clone(), name.clone());
            assert!(before.is_none_or(|b| b == name), "{host} in two shards");
        }
    }
    assert_eq!(total, 9_050);
    assert!(
        where_host
            .values()
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 1
    );
    // The collection that stands for the shards holds nothing.
    assert_eq!(m.count(), 0);

    // A row the schema refuses is refused before any shard is written.
    assert!(sharded::insert(None, &db, "m", vec![json!({"host": "h1", "v": "nope"})]).is_err());
    assert_eq!(
        run(&engine, "RETURN LENGTH(FOR r IN m RETURN 1)").unwrap(),
        vec![json!(9_050)]
    );

    // What this version does not route is refused.
    assert!(columnar::truncate(&db, None, "m").is_err());
    assert!(columnar::delete_rows(&db, None, "m", &ids[..1]).is_err());
    assert!(columnar::compact(&db, "m").is_err());
    assert!(m.create_tag("t").is_err());
    assert!(columnar::create_branch(&db, "m", "b", None).is_err());
    assert!(sharded::create(
        None,
        &db,
        None,
        "x",
        columns(),
        CompressionType::Lz4,
        2,
        "nope",
        1
    )
    .is_err());

    // Dropped, its shards go with it.
    sharded::drop(None, &db, None, "m", &m.sharding().unwrap()).unwrap();
    for shard in 0..4 {
        assert!(
            ColumnarCollection::load(sharded::physical("m", shard), "app", db.db_arc()).is_err()
        );
    }
    assert!(ColumnarCollection::load("m".to_string(), "app", db.db_arc()).is_err());
}
