//! UPDATE / REPLACE / REMOVE / INSERT with OPTIONS and OLD / NEW on a sharded
//! collection (single node: every shard is local, so no network is involved).

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
        r#"FOR i IN 1..6 INSERT {_key: CONCAT("k", i), n: i, tag: "x", extra: {a: 1}} INTO c"#,
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn update_returns_the_merged_document_as_new_and_old_is_the_pre_image() {
    let f = fixture().await;
    seed(&f).await;

    let out = run(
        &f,
        r#"UPDATE "k2" WITH {n: 20} IN c RETURN {old: OLD.n, new: NEW.n, tag: NEW.tag}"#,
    )
    .await
    .unwrap();
    // NEW carries the fields the patch did not touch.
    assert_eq!(out, vec![json!({"old": 2, "new": 20, "tag": "x"})]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replace_swaps_the_document_and_needs_it_to_exist() {
    let f = fixture().await;
    seed(&f).await;

    let out = run(&f, r#"REPLACE "k3" WITH {only: true} IN c RETURN NEW"#)
        .await
        .unwrap();
    assert_eq!(out[0]["only"], true);
    assert!(
        out[0].get("tag").is_none(),
        "replaced, not merged: {}",
        out[0]
    );

    assert!(run(&f, r#"REPLACE "nope" WITH {a: 1} IN c"#).await.is_err());
    // ignoreErrors skips the missing document.
    assert!(run(
        &f,
        r#"REPLACE "nope" WITH {a: 1} IN c OPTIONS {ignoreErrors: true}"#
    )
    .await
    .is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn update_options_keep_null_and_merge_objects() {
    let f = fixture().await;
    seed(&f).await;

    let out = run(
        &f,
        r#"UPDATE "k1" WITH {tag: null, extra: {b: 2}} IN c
           OPTIONS {keepNull: false, mergeObjects: true} RETURN NEW"#,
    )
    .await
    .unwrap();
    assert!(
        out[0].get("tag").is_none(),
        "keepNull:false drops it: {}",
        out[0]
    );
    assert_eq!(out[0]["extra"], json!({"a": 1, "b": 2}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remove_returns_old_and_insert_honours_overwrite_mode() {
    let f = fixture().await;
    seed(&f).await;

    let out = run(&f, r#"REMOVE "k4" IN c RETURN OLD.n"#).await.unwrap();
    assert_eq!(out, vec![json!(4)]);
    assert_eq!(run(&f, "FOR d IN c RETURN d._key").await.unwrap().len(), 5);

    // ignore: the existing document stays.
    run(
        &f,
        r#"INSERT {_key: "k5", n: 500} INTO c OPTIONS {overwriteMode: "ignore"}"#,
    )
    .await
    .unwrap();
    assert_eq!(
        run(&f, r#"FOR d IN c FILTER d._key == "k5" RETURN d.n"#)
            .await
            .unwrap(),
        vec![json!(5)]
    );
    // replace: it is swapped, and OLD shows the previous version.
    let out = run(
        &f,
        r#"INSERT {_key: "k5", n: 500} INTO c OPTIONS {overwriteMode: "replace"} RETURN {o: OLD.n, n: NEW.n}"#,
    )
    .await
    .unwrap();
    assert_eq!(out, vec![json!({"o": 5, "n": 500})]);
    // An explicit conflict mode is an error for an existing key.
    assert!(run(
        &f,
        r#"INSERT {_key: "k5"} INTO c OPTIONS {overwriteMode: "conflict"}"#
    )
    .await
    .is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn key_filters_find_documents_in_shards() {
    let f = fixture().await;
    seed(&f).await;
    // Documents report the logical collection in `_id`, not their shard's.
    let ids = run(&f, "FOR d IN c RETURN d._id").await.unwrap();
    assert!(
        ids.iter().all(|v| v.as_str().unwrap().starts_with("c/")),
        "{:?}",
        ids
    );
    assert_eq!(
        run(&f, r#"FOR d IN c FILTER d._id == "c/k5" RETURN d.n"#)
            .await
            .unwrap(),
        vec![json!(5)]
    );
    for q in [
        r#"FOR d IN c FILTER d._key == "k5" RETURN d.n"#,
        r#"FOR d IN c FILTER d._key IN ["k5", "zz"] RETURN d.n"#,
    ] {
        assert_eq!(run(&f, q).await.unwrap(), vec![json!(5)], "{}", q);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plain_insert_of_an_existing_key_conflicts() {
    let f = fixture().await;
    seed(&f).await;
    assert!(run(&f, r#"INSERT {_key: "k5", n: 99} INTO c"#)
        .await
        .is_err());
    // Nothing was overwritten, and keyless inserts still batch.
    assert_eq!(
        run(&f, r#"FOR d IN c FILTER d._key == "k5" RETURN d.n"#)
            .await
            .unwrap(),
        vec![json!(5)]
    );
    run(&f, "FOR i IN 1..4 INSERT {n: i} INTO c").await.unwrap();
    assert_eq!(run(&f, "FOR d IN c RETURN 1").await.unwrap().len(), 10);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replicas_are_sent_the_stored_document_not_the_patch() {
    use solidb::sync::log::SyncLog;
    let tmp = TempDir::new().unwrap();
    let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
    engine.create_database("d".to_string()).unwrap();
    let db = engine.get_database("d").unwrap();
    db.create_collection("c".to_string(), None).unwrap();
    let config = CollectionShardConfig {
        num_shards: 2,
        shard_key: "_key".to_string(),
        replication_factor: 1,
    };
    db.get_collection("c")
        .unwrap()
        .set_shard_config(&config)
        .unwrap();
    let log_dir = tmp.path().join("log");
    let log = Arc::new(SyncLog::new("n1".to_string(), log_dir.to_str().unwrap(), 64).unwrap());
    let coord = Arc::new(ShardCoordinator::new(
        engine.clone(),
        None,
        Some(log.clone()),
    ));
    coord.init_collection("d", "c", &config).unwrap();
    coord.create_shards("d", "c").await.unwrap();

    coord
        .insert("d", "c", &config, json!({"_key": "k", "a": 1, "b": 2}))
        .await
        .unwrap();
    coord
        .update("d", "c", &config, "k", json!({"b": 20}))
        .await
        .unwrap();
    coord
        .replace("d", "c", &config, "k", json!({"c": 3}))
        .await
        .unwrap();

    let entries: Vec<Value> = log
        .get_entries_after(0, 1000)
        .into_iter()
        .filter(|e| e.key == "k" && format!("{:?}", e.operation) == "Update")
        .map(|e| serde_json::from_slice(e.data.as_deref().unwrap()).unwrap())
        .collect();
    assert_eq!(entries.len(), 2, "{:?}", entries);
    assert_eq!(
        entries[0]["a"], 1,
        "the update entry lost a field: {}",
        entries[0]
    );
    assert_eq!(entries[0]["b"], 20);
    assert_eq!(entries[1]["c"], 3);
    assert!(
        entries[1].get("a").is_none(),
        "a replace keeps no old field: {}",
        entries[1]
    );
    assert_eq!(entries[1]["_key"], "k");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn update_and_replace_find_documents_sharded_by_another_field() {
    let tmp = TempDir::new().unwrap();
    let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
    engine.create_database("d".to_string()).unwrap();
    let db = engine.get_database("d").unwrap();
    db.create_collection("c".to_string(), None).unwrap();
    let config = CollectionShardConfig {
        num_shards: 4,
        shard_key: "tenant".to_string(),
        replication_factor: 1,
    };
    db.get_collection("c")
        .unwrap()
        .set_shard_config(&config)
        .unwrap();
    let coord = Arc::new(ShardCoordinator::new(engine.clone(), None, None));
    coord.init_collection("d", "c", &config).unwrap();
    coord.create_shards("d", "c").await.unwrap();

    // Enough documents that some key hash and tenant hash disagree.
    let keys: Vec<String> = (0..12).map(|i| format!("k{i}")).collect();
    let mut misrouted_by_key = 0;
    for (i, key) in keys.iter().enumerate() {
        let tenant = format!("t{}", i % 5);
        if coord.route(key, 4) != coord.route(&tenant, 4) {
            misrouted_by_key += 1;
        }
        coord
            .insert(
                "d",
                "c",
                &config,
                json!({"_key": key, "tenant": tenant, "n": i}),
            )
            .await
            .unwrap();
    }
    assert!(misrouted_by_key > 0, "fixture does not exercise the bug");

    for key in &keys {
        let updated = coord
            .update("d", "c", &config, key, json!({"n": 100}))
            .await;
        assert!(updated.is_ok(), "update {} failed: {:?}", key, updated);
        let replaced = coord
            .replace("d", "c", &config, key, json!({"only": true}))
            .await;
        assert!(replaced.is_ok(), "replace {} failed: {:?}", key, replaced);
        let stored = coord.get("d", "c", key).await.unwrap();
        assert_eq!(stored["only"], true);
        assert!(
            stored.get("n").is_none(),
            "replaced, not merged: {}",
            stored
        );
        assert!(
            stored.get("tenant").is_some(),
            "the shard key is kept: {}",
            stored
        );
    }

    // Changing the shard key would strand the document on the wrong shard.
    let moved = coord
        .update("d", "c", &config, "k1", json!({"tenant": "elsewhere"}))
        .await;
    assert!(
        matches!(moved, Err(solidb::DbError::BadRequest(_))),
        "{:?}",
        moved
    );
}

#[test]
fn two_engines_do_not_share_shard_configs() {
    let make = |shards: u16| {
        let tmp = TempDir::new().unwrap();
        let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
        engine.create_database("d".to_string()).unwrap();
        let db = engine.get_database("d").unwrap();
        db.create_collection("c".to_string(), None).unwrap();
        db.get_collection("c")
            .unwrap()
            .set_shard_config(&CollectionShardConfig {
                num_shards: shards,
                shard_key: "_key".to_string(),
                replication_factor: 1,
            })
            .unwrap();
        (tmp, engine)
    };
    // Same database and collection ids in both, different configs.
    let (_t1, a) = make(3);
    let (_t2, b) = make(7);
    let shards = |e: &StorageEngine| {
        e.get_database("d")
            .unwrap()
            .get_collection("c")
            .unwrap()
            .get_shard_config()
            .unwrap()
            .num_shards
    };
    assert_eq!(shards(&a), 3);
    assert_eq!(shards(&b), 7);
}

/// Card #799: an UPDATE computed from the document the query read does not
/// overwrite what another writer stored in between. The shard compares the
/// revision; the row is then evaluated again on the current document.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_updates_of_one_sharded_document_lose_nothing() {
    let f = Arc::new(fixture().await);
    seed(&f).await;
    run(&f, r#"UPDATE "k1" WITH {n: 0, log: []} IN c"#)
        .await
        .unwrap();

    const WRITERS: usize = 8;
    const ROUNDS: usize = 25;
    let mut writers = Vec::new();
    for w in 0..WRITERS {
        let f = f.clone();
        writers.push(tokio::spawn(async move {
            let mut conflicts = 0;
            for r in 0..ROUNDS {
                let q = format!(
                    r#"FOR d IN c FILTER d._key == "k1" UPDATE d WITH {{n: d.n + 1, log: PUSH(d.log, "{w}-{r}")}} IN c RETURN NEW.n"#
                );
                // A writer that keeps being overtaken is told so, and tries
                // again: what it must never do is write over a value it did
                // not read.
                loop {
                    match run(&f, &q).await {
                        Ok(rows) => {
                            assert_eq!(rows.len(), 1, "{rows:?}");
                            break;
                        }
                        Err(e) if e.contains("retry the query") => conflicts += 1,
                        Err(e) => panic!("{e}"),
                    }
                }
            }
            conflicts
        }));
    }
    for writer in writers {
        writer.await.unwrap();
    }
    let out = run(
        &f,
        r#"FOR d IN c FILTER d._key == "k1" RETURN [d.n, LENGTH(d.log), COUNT_DISTINCT(d.log)]"#,
    )
    .await
    .unwrap();
    let total = WRITERS * ROUNDS;
    assert_eq!(out, vec![json!([total, total, total])]);
}

/// A row whose document moved on is evaluated again: its FILTER decides on
/// the current version, so two workers cannot both claim one job.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_sharded_update_claims_a_job_once() {
    let f = Arc::new(fixture().await);
    run(
        &f,
        r#"FOR i IN 1..40 INSERT {_key: CONCAT("j", i), status: "pending"} INTO c"#,
    )
    .await
    .unwrap();
    let mut workers = Vec::new();
    for w in 0..6 {
        let f = f.clone();
        workers.push(tokio::spawn(async move {
            let q = format!(
                r#"FOR j IN c FILTER j.status == "pending" UPDATE j WITH {{status: "taken", by: {w}}} IN c RETURN NEW._key"#
            );
            loop {
                match run(&f, &q).await {
                    Ok(rows) => return rows.len(),
                    Err(e) if e.contains("retry the query") => {}
                    Err(e) => panic!("{e}"),
                }
            }
        }));
    }
    let mut claimed = 0;
    for worker in workers {
        claimed += worker.await.unwrap();
    }
    // Every job taken, each by one worker.
    assert_eq!(claimed, 40);
    let out = run(
        &f,
        r#"RETURN LENGTH(FOR j IN c FILTER j.status == "taken" RETURN 1)"#,
    )
    .await
    .unwrap();
    assert_eq!(out, vec![json!(40)]);
}

/// An UPDATE that names no document the query read is written as before,
/// and one whose document is gone leaves its row out.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sharded_update_by_key_and_of_a_missing_document() {
    let f = fixture().await;
    seed(&f).await;
    let out = run(
        &f,
        r#"FOR i IN 1..3 UPDATE CONCAT("k", i) WITH {n: i * 100} IN c RETURN NEW.n"#,
    )
    .await
    .unwrap();
    assert_eq!(out, vec![json!(100), json!(200), json!(300)]);
    let missing = run(&f, r#"UPDATE "nope" WITH {n: 1} IN c"#).await;
    assert!(missing.is_err(), "{missing:?}");
}

/// What `PUT …/document/c/key` with `If-Match` asks of the coordinator on a
/// sharded collection: a stale revision writes nothing and says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_coordinator_refuses_an_update_computed_from_an_old_revision() {
    let f = fixture().await;
    seed(&f).await;
    let config = CollectionShardConfig {
        num_shards: 3,
        shard_key: "_key".to_string(),
        replication_factor: 1,
    };
    let current = f.coord.get("d", "c", "k3").await.unwrap();
    let rev = current["_rev"].as_str().unwrap().to_string();

    let stale = f
        .coord
        .update_guarded(
            "d",
            "c",
            &config,
            "k3",
            json!({"n": 99}),
            Some("not-the-revision"),
        )
        .await
        .unwrap();
    assert!(stale.is_none());
    assert_eq!(f.coord.get("d", "c", "k3").await.unwrap()["n"], json!(3));

    let written = f
        .coord
        .update_guarded("d", "c", &config, "k3", json!({"n": 30}), Some(&rev))
        .await
        .unwrap()
        .expect("the current revision writes");
    assert_eq!(written["n"], json!(30));
    assert_ne!(written["_rev"].as_str().unwrap(), rev);
    // The revision it replaced is stale in its turn.
    let again = f
        .coord
        .update_guarded("d", "c", &config, "k3", json!({"n": 31}), Some(&rev))
        .await
        .unwrap();
    assert!(again.is_none());
    // And without a revision the update is written as before.
    let plain = f
        .coord
        .update("d", "c", &config, "k3", json!({"n": 32}))
        .await
        .unwrap();
    assert_eq!(plain["n"], json!(32));
}
