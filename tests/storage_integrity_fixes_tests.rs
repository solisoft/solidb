//! Regression tests for storage-layer integrity fixes from the September 2026
//! audit: D1 (insert overwrite), D4 (per-key write serialisation), D6
//! (version-history prefix collisions) and H8 (TTL expiry keys in `doc:`).

use serde_json::json;
use solidb::error::DbError;
use solidb::storage::{IndexType, StorageEngine};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

fn engine_with(coll: &str) -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).expect("engine");
    engine.create_collection(coll.to_string(), None).unwrap();
    (engine, tmp)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn d1_insert_of_existing_key_is_a_conflict() {
    let (engine, _tmp) = engine_with("users");
    let users = engine.get_collection("users").unwrap();
    users
        .create_index(
            "email_u".to_string(),
            vec!["email".to_string()],
            IndexType::Hash,
            true,
        )
        .unwrap();

    users
        .insert(json!({"_key": "k", "email": "old@x"}))
        .unwrap();
    let err = users
        .insert(json!({"_key": "k", "email": "new@x"}))
        .unwrap_err();
    assert!(matches!(err, DbError::ConflictError(_)), "got {err:?}");
    assert_eq!(users.count(), 1);
    assert_eq!(users.get("k").unwrap().get("email"), Some(json!("old@x")));

    // insert_or_replace is the overwrite path, and it cleans the old unique
    // entry: "old@x" becomes usable by another document.
    users
        .insert_or_replace(json!({"_key": "k", "email": "new@x"}))
        .unwrap();
    assert_eq!(users.count(), 1);
    users
        .insert(json!({"_key": "other", "email": "old@x"}))
        .expect("old unique value must be free after replace");
}

#[test]
fn d4_concurrent_updates_leave_no_stale_index_entries() {
    let (engine, _tmp) = engine_with("items");
    let items = engine.get_collection("items").unwrap();
    items
        .create_index(
            "v_idx".to_string(),
            vec!["v".to_string()],
            IndexType::Hash,
            false,
        )
        .unwrap();
    items.insert(json!({"_key": "k", "v": -1})).unwrap();

    let threads: Vec<_> = (0..8)
        .map(|t| {
            let items = items.clone();
            std::thread::spawn(move || {
                for i in 0..50 {
                    items.update("k", json!({"v": t * 1000 + i})).unwrap();
                }
            })
        })
        .collect();
    for th in threads {
        th.join().unwrap();
    }

    let final_v = items.get("k").unwrap().get("v").unwrap();
    let mut hits = 0;
    for t in 0..8 {
        for i in 0..50 {
            let v = json!(t * 1000 + i);
            let found = items.index_lookup_eq("v", &v).unwrap_or_default();
            if v == final_v {
                assert_eq!(found.len(), 1);
            }
            hits += found.len();
        }
    }
    assert_eq!(hits, 1, "only the final value may still be indexed");
}

#[test]
fn d4_concurrent_inserts_cannot_share_a_unique_value() {
    let (engine, _tmp) = engine_with("accounts");
    let accounts = engine.get_collection("accounts").unwrap();
    accounts
        .create_index(
            "name_u".to_string(),
            vec!["name".to_string()],
            IndexType::Hash,
            true,
        )
        .unwrap();

    let threads: Vec<_> = (0..8)
        .map(|t| {
            let accounts = accounts.clone();
            std::thread::spawn(move || {
                accounts
                    .insert(json!({"_key": format!("a{t}"), "name": "same"}))
                    .is_ok()
            })
        })
        .collect();
    let ok = threads
        .into_iter()
        .map(|th| th.join().unwrap())
        .filter(|ok| *ok)
        .count();
    assert_eq!(ok, 1);
    assert_eq!(accounts.count(), 1);
}

#[test]
fn d6_version_history_is_bound_to_the_exact_key() {
    let (engine, _tmp) = engine_with("docs");
    let docs = engine.get_collection("docs").unwrap();
    docs.enable_versioning().unwrap();

    docs.insert(json!({"_key": "a", "n": 0})).unwrap();
    docs.insert(json!({"_key": "a:b", "n": 0})).unwrap();
    for n in 1..5 {
        docs.update("a:b", json!({"n": n})).unwrap();
    }

    // History of "a" must not include any version of "a:b".
    assert_eq!(docs.doc_history("a").len(), 1);
    assert_eq!(docs.doc_history("a:b").len(), 5);

    // Writing to "a" (which prunes its history) leaves "a:b" intact.
    docs.update("a", json!({"n": 1})).unwrap();
    assert_eq!(docs.doc_history("a").len(), 2);
    assert_eq!(docs.doc_history("a:b").len(), 5);
}

#[test]
fn h8_forged_ttl_key_does_not_delete_other_documents() {
    let (engine, _tmp) = engine_with("sessions");
    let sessions = engine.get_collection("sessions").unwrap();
    sessions
        .create_ttl_index("ttl".to_string(), "created_at".to_string(), 1)
        .unwrap();

    sessions
        .insert(json!({"_key": "victim", "created_at": now_secs() + 3600}))
        .unwrap();
    // A key shaped like a pre-H8 expiry entry for the victim.
    sessions
        .insert(json!({"_key": "ttl_exp::ttl:0:victim"}))
        .unwrap();

    assert_eq!(sessions.cleanup_all_expired_documents().unwrap(), 0);
    assert!(sessions.get("victim").is_ok());
    assert!(sessions.get("ttl_exp::ttl:0:victim").is_ok());
    assert_eq!(sessions.recalculate_count(), 2);
}

#[test]
fn h8_reaper_deletes_through_the_full_delete_path() {
    let (engine, _tmp) = engine_with("tokens");
    let tokens = engine.get_collection("tokens").unwrap();
    tokens
        .create_index(
            "code_u".to_string(),
            vec!["code".to_string()],
            IndexType::Hash,
            true,
        )
        .unwrap();
    tokens
        .create_ttl_index("ttl".to_string(), "created_at".to_string(), 1)
        .unwrap();

    tokens
        .insert(json!({"_key": "t1", "code": "abc", "created_at": 1000}))
        .unwrap();
    assert_eq!(tokens.count(), 1);
    assert_eq!(tokens.cleanup_all_expired_documents().unwrap(), 1);
    assert_eq!(tokens.count(), 0);

    // The unique entry went with the document.
    tokens
        .insert(json!({"_key": "t2", "code": "abc", "created_at": now_secs()}))
        .expect("unique value must be released by the reaper");

    // An expiry entry made stale by an update must not reap the document.
    tokens
        .insert(json!({"_key": "t3", "code": "def", "created_at": 1000}))
        .unwrap();
    tokens
        .update("t3", json!({"created_at": now_secs() + 3600}))
        .unwrap();
    assert_eq!(tokens.cleanup_all_expired_documents().unwrap(), 0);
    assert!(tokens.get("t3").is_ok());
}
