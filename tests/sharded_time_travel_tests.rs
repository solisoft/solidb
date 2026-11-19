//! SYSTEM_TIME / VALID_TIME on a sharded collection (cards #994, #1002).
//!
//! The scatter-gather branch returned before the FOR's time clauses were
//! read: `SYSTEM_TIME AS OF` answered with today's documents and
//! `VALID_TIME` filtered nothing. A versioned sharded collection keeps its
//! history in its shards and AS OF gathers it; one that is not versioned
//! refuses a historical scan. VALID_TIME filters document fields, so it
//! applies to the gathered rows like a local scan.

use serde_json::{json, Value};
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
    fixture_versioned(false).await
}

/// `versioned`: the collection keeps history from its creation, so the
/// shards created after it inherit the flag.
async fn fixture_versioned(versioned: bool) -> Fixture {
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
    if versioned {
        db.get_collection("c").unwrap().enable_versioning().unwrap();
    }
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
async fn system_time_on_an_unversioned_sharded_collection_is_refused() {
    let f = fixture().await;
    seed(&f).await;
    let err = run(&f, "FOR d IN c SYSTEM_TIME AS OF 1 RETURN d._key")
        .await
        .expect_err("a historical scan of shards must not answer with today's rows");
    assert!(err.contains("SYSTEM_TIME AS OF is not supported"), "{err}");
    assert!(err.contains("not versioned"), "{err}");
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// A versioned sharded collection (card #1002): the shards keep history,
/// and AS OF, DOC_AS_OF, DOC_HISTORY and SNAPSHOT_DIFF gather it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_versioned_sharded_collection_reads_its_past() {
    for inherit in [true, false] {
        let f = fixture_versioned(inherit).await;
        let db = f.engine.get_database("d").unwrap();
        if !inherit {
            // Turned on after the shards exist: they follow.
            db.get_collection("c").unwrap().enable_versioning().unwrap();
            solidb::sharding::versioning::sync_shard_versioning(&db, "c");
        }
        for shard in 0..3 {
            let physical = db.get_collection(&format!("c_s{shard}")).unwrap();
            assert!(physical.is_versioned(), "shard {shard}, inherit {inherit}");
        }
        run(
            &f,
            r#"FOR i IN 1..6 INSERT {_key: CONCAT("k", i), n: i} INTO c"#,
        )
        .await
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let t1 = now_ms();
        std::thread::sleep(std::time::Duration::from_millis(3));
        run(&f, r#"UPDATE "k1" WITH {n: 100} IN c"#).await.unwrap();
        run(&f, r#"REMOVE "k2" IN c"#).await.unwrap();
        run(&f, r#"INSERT {_key: "k7", n: 7} INTO c"#)
            .await
            .unwrap();

        let past = run(
            &f,
            &format!("FOR d IN c SYSTEM_TIME AS OF {t1} SORT d._key RETURN [d._key, d.n]"),
        )
        .await
        .unwrap();
        assert_eq!(
            past,
            vec![
                json!(["k1", 1]),
                json!(["k2", 2]),
                json!(["k3", 3]),
                json!(["k4", 4]),
                json!(["k5", 5]),
                json!(["k6", 6])
            ]
        );
        let now = run(
            &f,
            &format!(
                "FOR d IN c SYSTEM_TIME AS OF {} RETURN d._key",
                now_ms() + 5
            ),
        )
        .await
        .unwrap();
        assert_eq!(sorted_keys(&now), vec!["k1", "k3", "k4", "k5", "k6", "k7"]);

        let doc = run(&f, &format!(r#"RETURN DOC_AS_OF("c", "k1", {t1})"#))
            .await
            .unwrap();
        assert_eq!(doc[0]["n"], json!(1));
        assert_eq!(
            doc[0]["_id"],
            json!("c/k1"),
            "the logical id, not the shard's"
        );
        let gone = run(
            &f,
            &format!(r#"RETURN DOC_AS_OF("c", "k2", {})"#, now_ms() + 5),
        )
        .await
        .unwrap();
        assert!(gone[0].is_null());
        let history = run(&f, r#"RETURN DOC_HISTORY("c", "k1")[*].value.n"#)
            .await
            .unwrap();
        assert_eq!(history[0], json!([100, 1]), "newest first");
        let diff = run(
            &f,
            &format!(r#"RETURN SNAPSHOT_DIFF("c", {t1}, {})"#, now_ms() + 5),
        )
        .await
        .unwrap();
        assert!(!diff[0].is_null());
    }
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

/// `FOR d IN sharded SORT d._key` read the primary-key index of the local,
/// empty, collection and answered with nothing; `SORT d._id` the same.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_key_sort_of_a_sharded_collection_reads_its_shards() {
    let f = fixture().await;
    seed(&f).await;
    for q in [
        "FOR d IN c SORT d._key RETURN d._key",
        "FOR d IN c SORT d._id RETURN d._key",
    ] {
        let rows = run(&f, q).await.unwrap();
        let keys: Vec<&str> = rows.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(keys, vec!["cur", "old", "open"], "{q}");
    }
}
