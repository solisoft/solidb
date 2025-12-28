//! Key/Value TTL Expiration Tests
//!
//! Tests for:
//! - Creating TTL indexes
//! - Expiration of documents based on timestamp
//! - Cleanup mechanism
//! - Multiple TTL indexes

use solidb::storage::StorageEngine;
use serde_json::json;
use tempfile::TempDir;
use std::time::{SystemTime, UNIX_EPOCH};
use std::thread::sleep;
use std::time::Duration;

fn create_test_env() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine.create_collection("events".to_string(), None).unwrap();
    (engine, tmp_dir)
}

fn current_ts() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

#[test]
fn test_create_ttl_index() {
    let (engine, _tmp) = create_test_env();
    let col = engine.get_collection("events").unwrap();
    
    // Create TTL index expiring 1 second after 'created_at'
    let stats = col.create_ttl_index("idx_ttl".to_string(), "created_at".to_string(), 1).unwrap();
    
    assert_eq!(stats.name, "idx_ttl");
    assert_eq!(stats.field, "created_at");
    assert_eq!(stats.expire_after_seconds, 1);
    
    // Check list
    let indexes = col.list_ttl_indexes();
    assert_eq!(indexes.len(), 1);
    assert_eq!(indexes[0].name, "idx_ttl");
}

#[test]
fn test_ttl_expiration_manual_cleanup() {
    let (engine, _tmp) = create_test_env();
    let col = engine.get_collection("events").unwrap();
    
    // Expire 2 seconds after creation
    col.create_ttl_index("ttl".to_string(), "ts".to_string(), 2).unwrap();
    
    let now = current_ts();
    
    // Doc 1: Just created (expires in 2s)
    col.insert(json!({
        "_key": "fresh",
        "ts": now
    })).unwrap();
    
    // Doc 2: Created 5 seconds ago (expired)
    col.insert(json!({
        "_key": "expired",
        "ts": now - 5
    })).unwrap();
    
    // Run cleanup immediately
    let deleted = col.cleanup_all_expired_documents().unwrap();
    
    // Should have deleted "expired" but not "fresh"
    assert_eq!(deleted, 1);
    assert!(col.get("fresh").is_ok());
    assert!(col.get("expired").is_err());
    
    // Wait 3 seconds (fresh should expire now)
    sleep(Duration::from_secs(3));
    
    let deleted_2 = col.cleanup_all_expired_documents().unwrap();
    assert_eq!(deleted_2, 1);
    assert!(col.get("fresh").is_err());
}

#[test]
fn test_ttl_multiple_indexes() {
    let (engine, _tmp) = create_test_env();
    let col = engine.get_collection("events").unwrap();
    
    // Index 1: 'short' expires in 1s
    col.create_ttl_index("idx1".to_string(), "short".to_string(), 1).unwrap();
    
    // Index 2: 'long' expires in 10s
    col.create_ttl_index("idx2".to_string(), "long".to_string(), 10).unwrap();
    
    let now = current_ts();
    
    // Doc A: short=now (expires in 1s), long=now (expires in 10s)
    col.insert(json!({
        "_key": "A",
        "short": now,
        "long": now
    })).unwrap();
    
    // Doc B: short=future (safe), long=now-20 (expired via index 2)
    col.insert(json!({
        "_key": "B",
        "short": now + 100,
        "long": now - 20
    })).unwrap();
    
    // Run cleanup
    let deleted = col.cleanup_all_expired_documents().unwrap();
    
    // B should be deleted due to 'long' index expiration
    // A should survive (only 0s elapsed)
    assert_eq!(deleted, 1);
    assert!(col.get("A").is_ok());
    assert!(col.get("B").is_err());
    
    // Wait 2s
    sleep(Duration::from_secs(2));
    
    // Run cleanup again
    let deleted_2 = col.cleanup_all_expired_documents().unwrap();
    
    // A should be deleted now due to 'short' index expiration
    assert_eq!(deleted_2, 1);
    assert!(col.get("A").is_err());
}
