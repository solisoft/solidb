//! Point-in-time restore, first part (card #852): a checkpoint says where
//! the replication log stood when it was taken, and the log's entries after
//! that point are written to an archive a restore can replay.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use solidb::sync::log::SyncLog;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

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
    let text = String::from_utf8_lossy(&bytes).to_string();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_checkpoint_names_its_place_in_the_log_and_the_log_is_archived_from_there() {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    let log = Arc::new(
        SyncLog::new(
            "n1".to_string(),
            tmp.path().join("log").to_str().unwrap(),
            64,
        )
        .unwrap(),
    );
    let app = create_router(
        engine.clone(),
        None,
        Some(log.clone()),
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    let admin = common::seed_user_token(&engine, "backup_admin", &["admin"]);
    let base = "/_api/database/_system";
    let insert = |key: String| {
        let app = app.clone();
        let admin = admin.clone();
        async move {
            let (status, body) = call(
                &app,
                &admin,
                "POST",
                "/_api/database/_system/document/things",
                Some(json!({"_key": key, "n": 1})),
            )
            .await;
            assert!(status.is_success(), "{status} {body}");
        }
    };
    let (status, body) = call(
        &app,
        &admin,
        "POST",
        &format!("{base}/collection"),
        Some(json!({"name": "things"})),
    )
    .await;
    assert!(status.is_success(), "{status} {body}");
    for n in 0..5 {
        insert(format!("before{n}")).await;
    }

    // The checkpoint says where the log stood.
    let at_checkpoint = log.current_sequence();
    assert!(at_checkpoint >= 5);
    let (status, body) = call(
        &app,
        &admin,
        "POST",
        "/_api/backup",
        Some(json!({"path": "cp1"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["log_position"]["sequence"], json!(at_checkpoint));
    assert_eq!(body["log_position"]["node_id"], json!("n1"));
    assert_eq!(body["log_position"]["log_active"], json!(true));
    let checkpoint = std::path::PathBuf::from(body["path"].as_str().unwrap());
    let written: Value = serde_json::from_str(
        &std::fs::read_to_string(checkpoint.join("SYNC_LOG_POSITION")).unwrap(),
    )
    .unwrap();
    assert_eq!(written, body["log_position"]);

    // What is written afterwards is archived from that point.
    for n in 0..7 {
        insert(format!("after{n}")).await;
    }
    let (status, body) = call(
        &app,
        &admin,
        "POST",
        "/_api/backup/log",
        Some(json!({"path": "cp1.log", "after": at_checkpoint})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["entries"], json!(7));
    assert_eq!(body["first_sequence"], json!(at_checkpoint + 1));
    assert_eq!(body["last_sequence"], json!(log.current_sequence()));
    assert_eq!(body["complete"], json!(true));
    let entries =
        SyncLog::read_archive(std::path::Path::new(body["path"].as_str().unwrap())).unwrap();
    let keys: Vec<&str> = entries.iter().map(|entry| entry.key.as_str()).collect();
    assert_eq!(
        keys,
        ["after0", "after1", "after2", "after3", "after4", "after5", "after6"]
    );
    assert!(entries.iter().all(|entry| entry.collection == "things"));

    // An archive is not written over; one asked from before the oldest
    // entry kept says that it is not whole.
    let again = json!({"path": "cp1.log", "after": at_checkpoint});
    let (status, _) = call(&app, &admin, "POST", "/_api/backup/log", Some(again)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    log.prune_before(at_checkpoint + 3).unwrap();
    let (status, body) = call(
        &app,
        &admin,
        "POST",
        "/_api/backup/log",
        Some(json!({"path": "late.log", "after": at_checkpoint})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["complete"], json!(false));
    assert_eq!(body["first_sequence"], json!(at_checkpoint + 3));

    // Admin only, as a checkpoint.
    let viewer = common::seed_user_token(&engine, "backup_viewer", &["viewer"]);
    let (status, _) = call(
        &app,
        &viewer,
        "POST",
        "/_api/backup/log",
        Some(json!({"path": "viewer.log"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The restore itself: a checkpoint opened as a data directory, and the
/// archive replayed over it up to an instant. What was written before the
/// instant is there (an update, a delete, a collection created since the
/// checkpoint); what was written after it is not.
#[tokio::test(flavor = "multi_thread")]
async fn a_checkpoint_and_its_log_restore_the_data_of_an_instant() {
    use solidb::sync::{create_command_channel, ConnectionPool, SyncConfig, SyncState, SyncWorker};
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    let log = Arc::new(
        SyncLog::new(
            "n1".to_string(),
            tmp.path().join("log").to_str().unwrap(),
            64,
        )
        .unwrap(),
    );
    let app = create_router(
        engine.clone(),
        None,
        Some(log.clone()),
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    let admin = common::seed_user_token(&engine, "restore_admin", &["admin"]);
    let base = "/_api/database/_system";
    let post = |uri: String, body: Value| {
        let app = app.clone();
        let admin = admin.clone();
        async move {
            let (status, body) = call(&app, &admin, "POST", &uri, Some(body)).await;
            assert!(status.is_success(), "{uri}: {status} {body}");
            body
        }
    };
    let query = |text: &'static str| post(format!("{base}/cursor"), json!({"query": text}));
    post(format!("{base}/collection"), json!({"name": "things"})).await;
    for n in 1..=5 {
        post(
            format!("{base}/document/things"),
            json!({"_key": format!("k{n}"), "v": n}),
        )
        .await;
    }
    let checkpoint = post("/_api/backup".to_string(), json!({"path": "cp"})).await;
    let at_checkpoint = checkpoint["log_position"]["sequence"].as_u64().unwrap();

    // Before the instant: inserts, an update, a delete, a new collection.
    for n in 6..=8 {
        post(
            format!("{base}/document/things"),
            json!({"_key": format!("k{n}"), "v": n}),
        )
        .await;
    }
    query("UPDATE 'k1' WITH {v: 100} IN things").await;
    query("REMOVE 'k2' IN things").await;
    post(format!("{base}/collection"), json!({"name": "later"})).await;
    post(
        format!("{base}/document/later"),
        json!({"_key": "a", "v": 1}),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let instant = chrono::Utc::now().timestamp_millis() as u64;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    // After it.
    for n in 9..=10 {
        post(
            format!("{base}/document/things"),
            json!({"_key": format!("k{n}"), "v": n}),
        )
        .await;
    }
    query("UPDATE 'k3' WITH {v: 300} IN things").await;
    let archive = post(
        "/_api/backup/log".to_string(),
        json!({"path": "cp.log", "after": at_checkpoint}),
    )
    .await;
    assert_eq!(archive["complete"], json!(true));

    // The checkpoint as a data directory of its own.
    let restored_dir = tmp.path().join("restored");
    copy_dir(
        std::path::Path::new(checkpoint["path"].as_str().unwrap()),
        &restored_dir,
    );
    let restored = StorageEngine::new(restored_dir.to_str().unwrap()).unwrap();
    restored.initialize().unwrap();
    let storage = Arc::new(restored.clone());
    let (_commands, command_rx) = create_command_channel();
    let worker = SyncWorker::new(
        storage.clone(),
        Arc::new(SyncState::new(storage.clone(), "restored".to_string())),
        Arc::new(ConnectionPool::new("restored".to_string(), String::new())),
        Arc::new(
            SyncLog::new(
                "restored".to_string(),
                tmp.path().join("restored-log").to_str().unwrap(),
                64,
            )
            .unwrap(),
        ),
        SyncConfig::default(),
        command_rx,
        "restored".to_string(),
        String::new(),
        "127.0.0.1:0".to_string(),
    );
    let things = |engine: &StorageEngine| -> Vec<(String, Value)> {
        let mut docs: Vec<(String, Value)> = engine
            .get_database("_system")
            .unwrap()
            .get_collection("things")
            .unwrap()
            .all()
            .into_iter()
            .map(|doc| (doc.key.clone(), doc.to_value()["v"].clone()))
            .collect();
        docs.sort_by(|a, b| a.0.cmp(&b.0));
        docs
    };
    // As the checkpoint was taken.
    assert_eq!(things(&restored).len(), 5);

    let entries =
        SyncLog::read_archive(std::path::Path::new(archive["path"].as_str().unwrap())).unwrap();
    let replayed = worker
        .replay(entries.clone(), at_checkpoint, Some(instant))
        .await
        .unwrap();
    assert_eq!(
        (
            replayed.applied,
            replayed.in_checkpoint,
            replayed.after_instant
        ),
        (7, 0, 3)
    );
    let wanted: Vec<(String, Value)> = [
        ("k1", 100),
        ("k3", 3),
        ("k4", 4),
        ("k5", 5),
        ("k6", 6),
        ("k7", 7),
        ("k8", 8),
    ]
    .iter()
    .map(|(key, v)| (key.to_string(), json!(v)))
    .collect();
    assert_eq!(things(&restored), wanted);
    // Each document is the one its origin held: same revision, same
    // dates (card #853). `k3` was updated after the instant.
    let stamps = |engine: &StorageEngine, key: &str| {
        let doc = engine
            .get_database("_system")
            .unwrap()
            .get_collection("things")
            .unwrap()
            .get(key)
            .unwrap()
            .to_value();
        (
            doc["_rev"].clone(),
            doc["_created_at"].clone(),
            doc["_updated_at"].clone(),
        )
    };
    for key in ["k1", "k4", "k6", "k8"] {
        assert_eq!(stamps(&restored, key), stamps(&engine, key), "{key}");
    }
    let later = restored
        .get_database("_system")
        .unwrap()
        .get_collection("later")
        .unwrap();
    assert_eq!(later.count(), 1);

    // Replayed twice, the same data.
    worker
        .replay(entries, at_checkpoint, Some(instant))
        .await
        .unwrap();
    assert_eq!(things(&restored), wanted);
}
