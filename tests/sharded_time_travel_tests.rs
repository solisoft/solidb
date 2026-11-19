//! SYSTEM_TIME / VALID_TIME on a sharded collection (card #994).
//!
//! The scatter-gather branch returned before the FOR's time clauses were
//! read: `SYSTEM_TIME AS OF` answered with today's documents and
//! `VALID_TIME` filtered nothing. Shard collections keep no version history,
//! so a historical scan is refused; VALID_TIME filters document fields, so
//! it applies to the gathered rows like a local scan.

use serde_json::Value;
use solidb::sdbql::{parse, QueryExecutor};
use solidb::sharding::coordinator::{CollectionShardConfig, ShardCoordinator};
use solidb::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;

struct Fixture {
    engine: Arc<StorageEngine>,
    coord: Arc<ShardCoordinator>,
    _tmp: TempDir,
}

async fn fixture() -> Fixture {
    let tmp = TempDir::new().unwrap();
    let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
    engine.create_database("d".to_string()).unwrap();
    let db = engine.get_database("d").unwrap();
    db.create_collection("c".to_string(), None).unwrap();
    let config = CollectionShardConfig {
        num_shards: 3,
        shard_key: "_key".to_string(),
        replication_factor: 1,
    };
    db.get_collection("c")
        .unwrap()
        .set_shard_config(&config)
        .unwrap();
    let coord = Arc::new(ShardCoordinator::new(engine.clone(), None, None));
    coord.init_collection("d", "c", &config).unwrap();
    coord.create_shards("d", "c").await.unwrap();
    Fixture {
        engine,
        coord,
        _tmp: tmp,
    }
}

/// The executor blocks its thread waiting on the coordinator, so it runs on
/// the blocking pool while the runtime keeps driving the spawned calls.
async fn run(f: &Fixture, q: &str) -> Result<Vec<Value>, String> {
    let (engine, coord, q) = (f.engine.clone(), f.coord.clone(), q.to_string());
    tokio::task::spawn_blocking(move || {
        let query = parse(&q).map_err(|e| e.to_string())?;
        QueryExecutor::with_database(&engine, "d".to_string())
            .with_shard_coordinator(coord)
            .execute(&query)
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap()
}

async fn seed(f: &Fixture) {
    run(
        f,
        r#"FOR d IN [
            {_key: "old", valid_from: 0, valid_to: 10},
            {_key: "cur", valid_from: 20, valid_to: 100},
            {_key: "open", valid_from: 5}
        ] INSERT d INTO c"#,
    )
    .await
    .unwrap();
}

fn sorted_keys(rows: &[Value]) -> Vec<String> {
    let mut keys: Vec<String> = rows
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    keys.sort();
    keys
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn system_time_on_a_sharded_collection_is_refused() {
    let f = fixture().await;
    seed(&f).await;
    let err = run(&f, "FOR d IN c SYSTEM_TIME AS OF 1 RETURN d._key")
        .await
        .expect_err("a historical scan of shards must not answer with today's rows");
    assert!(err.contains("SYSTEM_TIME AS OF is not supported"), "{err}");
    assert!(err.contains("sharded"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn valid_time_filters_the_gathered_rows() {
    let f = fixture().await;
    seed(&f).await;
    let rows = run(&f, "FOR d IN c VALID_TIME AS OF 25 RETURN d._key")
        .await
        .unwrap();
    assert_eq!(sorted_keys(&rows), vec!["cur", "open"]);

    // The LIMIT applies to what survives the filter, not to the gather.
    let rows = run(&f, "FOR d IN c VALID_TIME AS OF 2 LIMIT 5 RETURN d._key")
        .await
        .unwrap();
    assert_eq!(sorted_keys(&rows), vec!["old"]);

    let rows = run(&f, "FOR d IN c VALID_TIME FROM 8 TO 22 RETURN d._key")
        .await
        .unwrap();
    assert_eq!(sorted_keys(&rows), vec!["cur", "old", "open"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plain_scan_of_shards_is_unchanged() {
    let f = fixture().await;
    seed(&f).await;
    let rows = run(&f, "FOR d IN c RETURN d._key").await.unwrap();
    assert_eq!(sorted_keys(&rows), vec!["cur", "old", "open"]);
    let rows = run(&f, "FOR d IN c LIMIT 2 RETURN d._key").await.unwrap();
    assert_eq!(rows.len(), 2);
}
