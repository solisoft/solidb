//! Version Vectors for Distributed Conflict Resolution
//!
//! Version vectors track the causal history of document updates across multiple nodes,
//! enabling detection of concurrent modifications and conflicts in offline-first sync.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};

/// A version vector tracks the logical clock of each node that has modified a document.
///
/// Example:
/// - Server node "node-1" has made 12 updates
/// - Client "device-abc" has made 5 updates
/// - Vector: {"node-1": 12, "device-abc": 5}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VersionVector {
    /// Map of node_id -> logical clock counter
    versions: HashMap<String, u64>,
    /// Hybrid Logical Clock timestamp for tie-breaking
    hlc_timestamp: u64,
    /// HLC logical counter for tie-breaking when timestamps are equal
    hlc_counter: u32,
}

impl VersionVector {
    /// Create a new empty version vector
    pub fn new() -> Self {
        Self {
            versions: HashMap::new(),
            hlc_timestamp: 0,
            hlc_counter: 0,
        }
    }

    /// Create a version vector from a node ID and initial counter
    pub fn with_node(node_id: impl Into<String>, counter: u64) -> Self {
        let mut versions = HashMap::new();
        versions.insert(node_id.into(), counter);
        Self {
            versions,
            hlc_timestamp: 0,
            hlc_counter: 0,
        }
    }

    /// Increment the counter for a specific node
    pub fn increment(&mut self, node_id: &str) -> u64 {
        let counter = self.versions.entry(node_id.to_string()).or_insert(0);
        *counter += 1;
        *counter
    }

    /// Get the counter for a specific node
    pub fn get(&self, node_id: &str) -> u64 {
        self.versions.get(node_id).copied().unwrap_or(0)
    }

    /// Set the HLC timestamp for this version
    pub fn set_hlc(&mut self, timestamp: u64, counter: u32) {
        self.hlc_timestamp = timestamp;
        self.hlc_counter = counter;
    }

    /// Get the HLC timestamp
    pub fn hlc_timestamp(&self) -> u64 {
        self.hlc_timestamp
    }

    /// Get the HLC counter
    pub fn hlc_counter(&self) -> u32 {
        self.hlc_counter
    }

    /// Check if this version vector dominates (is newer than) another
    ///
    /// Returns true if every counter in self is >= corresponding counter in other
    pub fn dominates(&self, other: &VersionVector) -> bool {
        for (node_id, other_counter) in &other.versions {
            let self_counter = self.versions.get(node_id).copied().unwrap_or(0);
            if self_counter < *other_counter {
                return false;
            }
        }
        true
    }

    /// Check if this version vector is dominated by (is older than) another
    pub fn is_dominated_by(&self, other: &VersionVector) -> bool {
        other.dominates(self)
    }

    /// Determine the relationship between two version vectors
    pub fn compare(&self, other: &VersionVector) -> VectorComparison {
        let self_dominates = self.dominates(other);
        let other_dominates = other.dominates(self);

        match (self_dominates, other_dominates) {
            (true, true) => VectorComparison::Equal,
            (true, false) => VectorComparison::Dominates,
            (false, true) => VectorComparison::Dominated,
            (false, false) => VectorComparison::Concurrent,
        }
    }

    /// Merge two version vectors (taking the maximum of each counter)
    pub fn merge(&mut self, other: &VersionVector) {
        for (node_id, other_counter) in &other.versions {
            let self_counter = self.versions.entry(node_id.clone()).or_insert(0);
            if *other_counter > *self_counter {
                *self_counter = *other_counter;
            }
        }

        // Take the newer HLC
        if other.hlc_timestamp > self.hlc_timestamp {
            self.hlc_timestamp = other.hlc_timestamp;
            self.hlc_counter = other.hlc_counter;
        } else if other.hlc_timestamp == self.hlc_timestamp && other.hlc_counter > self.hlc_counter
        {
            self.hlc_counter = other.hlc_counter;
        }
    }

    /// Create a merged version vector from two vectors
    pub fn merged(&self, other: &VersionVector) -> VersionVector {
        let mut result = self.clone();
        result.merge(other);
        result
    }

    /// Get all node IDs in this vector
    pub fn nodes(&self) -> impl Iterator<Item = &String> {
        self.versions.keys()
    }

    /// Check if this vector is empty
    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }

    /// Serialize to compact bytes for storage/transmission
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }

    /// Serialize to JSON string
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

impl PartialEq for VersionVector {
    fn eq(&self, other: &Self) -> bool {
        self.versions == other.versions
    }
}

impl Eq for VersionVector {}

impl Hash for VersionVector {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash all entries in a consistent order
        let mut entries: Vec<_> = self.versions.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (k, v) in entries {
            k.hash(state);
            v.hash(state);
        }
    }
}

impl fmt::Display for VersionVector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let entries: Vec<String> = self
            .versions
            .iter()
            .map(|(k, v)| format!("{}:{}", k, v))
            .collect();
        write!(f, "[{}]", entries.join(", "))
    }
}

/// Result of comparing two version vectors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorComparison {
    /// Both vectors are equal (same counters)
    Equal,
    /// Self dominates other (self is newer)
    Dominates,
    /// Self is dominated by other (other is newer)
    Dominated,
    /// Neither dominates the other (concurrent/conflict)
    Concurrent,
}

impl VectorComparison {
    /// Check if there's a conflict (concurrent modifications)
    pub fn is_conflict(&self) -> bool {
        matches!(self, VectorComparison::Concurrent)
    }

    /// Check if self should win over other
    pub fn self_wins(&self) -> bool {
        matches!(self, VectorComparison::Dominates | VectorComparison::Equal)
    }
}

/// A causal dot represents a single event in the causal history
/// Used for tracking parent revisions in CRDTs
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct CausalDot {
    /// Node that performed the operation
    pub node_id: String,
    /// Sequence number at that node
    pub sequence: u64,
}

impl CausalDot {
    pub fn new(node_id: impl Into<String>, sequence: u64) -> Self {
        Self {
            node_id: node_id.into(),
            sequence,
        }
    }
}

/// Conflict information for detected concurrent modifications
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictInfo {
    /// Document key that has a conflict
    pub document_key: String,
    /// Collection name
    pub collection: String,
    /// Local version vector
    pub local_vector: VersionVector,
    /// Remote version vector
    pub remote_vector: VersionVector,
    /// Local document data (if available)
    pub local_data: Option<serde_json::Value>,
    /// Remote document data
    pub remote_data: Option<serde_json::Value>,
    /// Timestamp when conflict was detected
    pub detected_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_vector_increment() {
        let mut vv = VersionVector::new();
        assert_eq!(vv.increment("node-1"), 1);
        assert_eq!(vv.increment("node-1"), 2);
        assert_eq!(vv.get("node-1"), 2);
        assert_eq!(vv.get("node-2"), 0);
    }

    #[test]
    fn test_version_vector_dominates() {
        let mut vv1 = VersionVector::new();
        vv1.increment("node-1");
        vv1.increment("node-1");

        let mut vv2 = VersionVector::new();
        vv2.increment("node-1");

        assert!(vv1.dominates(&vv2));
        assert!(!vv2.dominates(&vv1));
    }

    #[test]
    fn test_version_vector_concurrent() {
        // node-1 makes 2 changes
        let mut vv1 = VersionVector::new();
        vv1.increment("node-1");
        vv1.increment("node-1");

        // node-2 makes 2 changes (concurrent, no knowledge of node-1's changes)
        let mut vv2 = VersionVector::new();
        vv2.increment("node-2");
        vv2.increment("node-2");

        // Neither dominates the other - they are concurrent
        assert!(!vv1.dominates(&vv2));
        assert!(!vv2.dominates(&vv1));

        let comparison = vv1.compare(&vv2);
        assert_eq!(comparison, VectorComparison::Concurrent);
    }

    #[test]
    fn test_version_vector_merge() {
        let mut vv1 = VersionVector::new();
        vv1.increment("node-1");
        vv1.increment("node-1");

        let mut vv2 = VersionVector::new();
        vv2.increment("node-2");
        vv2.increment("node-2");
        vv2.increment("node-2");

        let merged = vv1.merged(&vv2);
        assert_eq!(merged.get("node-1"), 2);
        assert_eq!(merged.get("node-2"), 3);
    }

    #[test]
    fn test_version_vector_comparison() {
        let mut vv1 = VersionVector::new();
        vv1.increment("node-1");

        let mut vv2 = VersionVector::new();
        vv2.increment("node-1");
        vv2.increment("node-1");

        assert_eq!(vv2.compare(&vv1), VectorComparison::Dominates);
        assert_eq!(vv1.compare(&vv2), VectorComparison::Dominated);
        assert_eq!(vv1.compare(&vv1), VectorComparison::Equal);
    }

    #[test]
    fn test_version_vector_serialization() {
        let mut vv = VersionVector::new();
        vv.increment("node-1");
        vv.increment("node-2");
        vv.set_hlc(1234567890, 42);

        let bytes = vv.to_bytes();
        let restored = VersionVector::from_bytes(&bytes).unwrap();

        assert_eq!(vv.get("node-1"), restored.get("node-1"));
        assert_eq!(vv.get("node-2"), restored.get("node-2"));
        assert_eq!(vv.hlc_timestamp(), restored.hlc_timestamp());
    }
}
