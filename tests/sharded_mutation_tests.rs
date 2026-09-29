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
