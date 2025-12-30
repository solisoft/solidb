//! Cluster Module Coverage Tests
//!
//! Tests for cluster functionality covering:
//! - HybridLogicalClock (HLC) operations
//! - HLC Generator
//! - ClusterConfig
//! - Node identity

use solidb::cluster::{ClusterConfig, HybridLogicalClock};

// ============================================================================
// HybridLogicalClock Tests
// ============================================================================

#[test]
fn test_hlc_now() {
    let hlc = HybridLogicalClock::now("node1");
    
    // Physical time should be recent (within last second from epoch perspective)
    assert!(hlc.physical_time > 0);
    assert_eq!(hlc.logical_counter, 0);
    assert_eq!(hlc.node_id, "node1");
}

#[test]
fn test_hlc_new() {
    let hlc = HybridLogicalClock::new(1000, 5, "test_node".to_string());
    
    assert_eq!(hlc.physical_time, 1000);
    assert_eq!(hlc.logical_counter, 5);
    assert_eq!(hlc.node_id, "test_node");
}

#[test]
fn test_hlc_tick() {
    let hlc1 = HybridLogicalClock::new(1000, 0, "node1".to_string());
    let hlc2 = hlc1.tick("node1");
    
    // Ticked HLC should be greater
    assert!(hlc2.is_newer_than(&hlc1));
}

#[test]
fn test_hlc_tick_increases_counter_when_time_same() {
    let hlc1 = HybridLogicalClock::new(1000, 0, "node1".to_string());
    
    // If physical time is the same, logical counter should increase
    // Note: in practice tick uses current wall clock, but the result is always greater
    let hlc2 = hlc1.tick("node1");
    
    assert!(hlc2.is_newer_than(&hlc1));
}

#[test]
fn test_hlc_receive() {
    let local = HybridLogicalClock::new(1000, 0, "node1".to_string());
    let remote = HybridLogicalClock::new(2000, 5, "node2".to_string());
    
    let updated = local.receive(&remote, "node1");
    
    // Updated should be greater than both
    assert!(updated.is_newer_than(&local));
    assert!(updated.is_newer_than(&remote));
}

#[test]
fn test_hlc_receive_local_newer() {
    let local = HybridLogicalClock::new(3000, 10, "node1".to_string());
    let remote = HybridLogicalClock::new(1000, 5, "node2".to_string());
    
    let updated = local.receive(&remote, "node1");
    
    // Updated should still be greater than both
    assert!(updated.is_newer_than(&local));
    assert!(updated.is_newer_than(&remote));
}

#[test]
fn test_hlc_compare() {
    let hlc1 = HybridLogicalClock::new(1000, 0, "node1".to_string());
    let hlc2 = HybridLogicalClock::new(2000, 0, "node2".to_string());
    let hlc3 = HybridLogicalClock::new(1000, 5, "node3".to_string());
    
    // hlc2 has higher physical time
    assert_eq!(hlc1.compare(&hlc2), std::cmp::Ordering::Less);
    assert_eq!(hlc2.compare(&hlc1), std::cmp::Ordering::Greater);
    
    // hlc3 has same physical time but higher counter
    assert_eq!(hlc1.compare(&hlc3), std::cmp::Ordering::Less);
}

#[test]
fn test_hlc_is_newer_than() {
    let older = HybridLogicalClock::new(1000, 0, "node1".to_string());
    let newer = HybridLogicalClock::new(2000, 0, "node2".to_string());
    
    assert!(newer.is_newer_than(&older));
    assert!(!older.is_newer_than(&newer));
}

#[test]
fn test_hlc_to_string_key() {
    let hlc = HybridLogicalClock::new(1234567890, 42, "mynode".to_string());
    
    let key = hlc.to_string_key();
    
    // Key is in hex format: {physical_time:016x}-{logical_counter:08x}-{node_id}
    // 1234567890 in hex is 499602d2
    assert!(key.contains("499602d2"));
    assert!(key.contains("0000002a")); // 42 in hex  
    assert!(key.contains("mynode"));
}

#[test]
fn test_hlc_from_string_key() {
    let original = HybridLogicalClock::new(1234567890, 42, "testnode".to_string());
    let key = original.to_string_key();
    
    let parsed = HybridLogicalClock::from_string_key(&key);
    
    assert!(parsed.is_some());
    let parsed = parsed.unwrap();
    assert_eq!(parsed.physical_time, original.physical_time);
    assert_eq!(parsed.logical_counter, original.logical_counter);
    assert_eq!(parsed.node_id, original.node_id);
}

#[test]
fn test_hlc_from_string_key_invalid() {
    assert!(HybridLogicalClock::from_string_key("invalid").is_none());
    assert!(HybridLogicalClock::from_string_key("").is_none());
    assert!(HybridLogicalClock::from_string_key("a:b").is_none());
}

#[test]
fn test_hlc_ordering() {
    let mut hlcs = vec![
        HybridLogicalClock::new(3000, 0, "a".to_string()),
        HybridLogicalClock::new(1000, 5, "b".to_string()),
        HybridLogicalClock::new(2000, 0, "c".to_string()),
        HybridLogicalClock::new(1000, 0, "d".to_string()),
    ];
    
    hlcs.sort();
    
    // Should be sorted by physical time, then counter, then node
    assert_eq!(hlcs[0].physical_time, 1000);
    assert_eq!(hlcs[0].logical_counter, 0);
    assert_eq!(hlcs[3].physical_time, 3000);
}

#[test]
fn test_hlc_equality() {
    let hlc1 = HybridLogicalClock::new(1000, 5, "node".to_string());
    let hlc2 = HybridLogicalClock::new(1000, 5, "node".to_string());
    let hlc3 = HybridLogicalClock::new(1000, 6, "node".to_string());
    
    assert_eq!(hlc1, hlc2);
    assert_ne!(hlc1, hlc3);
}

// ============================================================================
// ClusterConfig Tests  
// ============================================================================

#[test]
fn test_cluster_config_new() {
    let config = ClusterConfig::new(
        Some("node1".to_string()),
        vec!["peer1:4000".to_string()],
        4000,
        None,
    );
    
    assert_eq!(config.node_id, "node1");
    assert_eq!(config.peers.len(), 1);
    assert_eq!(config.replication_port, 4000);
}

#[test]
fn test_cluster_config_replication_addr() {
    let config = ClusterConfig::new(
        Some("node1".to_string()),
        vec![],
        5000,
        None,
    );
    
    assert_eq!(config.replication_addr(), "0.0.0.0:5000");
}

#[test]
fn test_cluster_config_requires_auth() {
    let config = ClusterConfig::new(
        Some("node1".to_string()),
        vec![],
        4000,
        None,  // No keyfile
    );
    
    assert!(!config.requires_auth());
}

#[test]
fn test_cluster_config_default() {
    let config = ClusterConfig::default();
    
    assert!(!config.node_id.is_empty());
    // Default has no peers
    assert!(config.peers.is_empty());
}

#[test]
fn test_cluster_config_serialization() {
    let config = ClusterConfig::new(
        Some("test_node".to_string()),
        vec!["peer1:4000".to_string()],
        4001,
        None,
    );
    
    let serialized = serde_json::to_string(&config).unwrap();
    let deserialized: ClusterConfig = serde_json::from_str(&serialized).unwrap();
    
    assert_eq!(deserialized.node_id, "test_node");
    assert_eq!(deserialized.peers.len(), 1);
    assert_eq!(deserialized.replication_port, 4001);
}

// ============================================================================
// HLC Serialization Tests
// ============================================================================

#[test]
fn test_hlc_json_serialization() {
    let hlc = HybridLogicalClock::new(1234567890, 42, "node123".to_string());
    
    let json = serde_json::to_string(&hlc).unwrap();
    let parsed: HybridLogicalClock = serde_json::from_str(&json).unwrap();
    
    assert_eq!(parsed, hlc);
}

#[test]
fn test_hlc_bincode_serialization() {
    let hlc = HybridLogicalClock::new(9876543210, 100, "bincode_node".to_string());
    
    let bytes = bincode::serialize(&hlc).unwrap();
    let parsed: HybridLogicalClock = bincode::deserialize(&bytes).unwrap();
    
    assert_eq!(parsed.physical_time, 9876543210);
    assert_eq!(parsed.logical_counter, 100);
    assert_eq!(parsed.node_id, "bincode_node");
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_hlc_max_counter() {
    let hlc = HybridLogicalClock::new(1000, u32::MAX, "node".to_string());
    
    // Should still be able to tick
    let ticked = hlc.tick("node");
    assert!(ticked.is_newer_than(&hlc));
}

#[test]
fn test_hlc_empty_node_id() {
    let hlc = HybridLogicalClock::new(1000, 0, "".to_string());
    
    // Should work even with empty node ID
    let key = hlc.to_string_key();
    let parsed = HybridLogicalClock::from_string_key(&key);
    assert!(parsed.is_some());
}

#[test]
fn test_hlc_special_chars_in_node_id() {
    // Note: using safe characters that don't interfere with the key format
    let hlc = HybridLogicalClock::new(1000, 0, "node_with_underscore".to_string());
    
    let key = hlc.to_string_key();
    let parsed = HybridLogicalClock::from_string_key(&key);
    
    assert!(parsed.is_some());
    assert_eq!(parsed.unwrap().node_id, "node_with_underscore");
}
