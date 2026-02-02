//! CRDT (Conflict-free Replicated Data Types) for Automatic Merging
//!
//! CRDTs allow concurrent updates to be merged automatically without conflicts.
//! They form the foundation for offline-first collaborative applications.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// Trait for CRDT types
pub trait CRDT: Clone {
    /// Merge another CRDT into this one
    fn merge(&mut self, other: &Self);

    /// Create a merged copy
    fn merged(&self, other: &Self) -> Self;

    /// Check if this CRDT has been modified from initial state
    fn is_empty(&self) -> bool;
}

/// Last-Write-Wins Register
///
/// Stores a single value with a timestamp. When merging, the value
/// with the higher timestamp wins.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LWWRegister<T> {
    /// The stored value
    value: T,
    /// Timestamp for comparison (HLC)
    timestamp: u64,
    /// Node ID for tie-breaking
    node_id: String,
}

impl<T: Clone + Default> LWWRegister<T> {
    pub fn new(value: T, node_id: impl Into<String>) -> Self {
        Self {
            value,
            timestamp: now(),
            node_id: node_id.into(),
        }
    }

    pub fn with_timestamp(value: T, node_id: impl Into<String>, timestamp: u64) -> Self {
        Self {
            value,
            timestamp,
            node_id: node_id.into(),
        }
    }

    /// Get the current value
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Set a new value (updates timestamp)
    pub fn set(&mut self, value: T) {
        self.value = value;
        self.timestamp = now();
    }

    /// Get the timestamp
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }
}

impl<T: Clone + Default> CRDT for LWWRegister<T> {
    fn merge(&mut self, other: &Self) {
        match self.timestamp.cmp(&other.timestamp) {
            Ordering::Less => {
                // Other is newer
                self.value = other.value.clone();
                self.timestamp = other.timestamp;
                self.node_id = other.node_id.clone();
            }
            Ordering::Equal => {
                // Tie-break by node ID
                if other.node_id > self.node_id {
                    self.value = other.value.clone();
                    self.node_id = other.node_id.clone();
                }
            }
            Ordering::Greater => {
                // Self is newer, do nothing
            }
        }
    }

    fn merged(&self, other: &Self) -> Self {
        let mut result = self.clone();
        result.merge(other);
        result
    }

    fn is_empty(&self) -> bool {
        false // LWWRegister always has a value
    }
}

/// Grow-Only Counter (G-Counter)
///
/// A counter that can only be incremented. Multiple replicas can
/// increment independently, and the merged result is the sum.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GCounter {
    /// Per-node counts
    nodes: HashMap<String, u64>,
}

impl GCounter {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    /// Increment the counter for a node
    pub fn increment(&mut self, node_id: &str) {
        let count = self.nodes.entry(node_id.to_string()).or_insert(0);
        *count += 1;
    }

    /// Get the total count
    pub fn value(&self) -> u64 {
        self.nodes.values().sum()
    }

    /// Get the count for a specific node
    pub fn node_value(&self, node_id: &str) -> u64 {
        self.nodes.get(node_id).copied().unwrap_or(0)
    }
}

impl CRDT for GCounter {
    fn merge(&mut self, other: &Self) {
        for (node_id, count) in &other.nodes {
            let self_count = self.nodes.entry(node_id.clone()).or_insert(0);
            if *count > *self_count {
                *self_count = *count;
            }
        }
    }

    fn merged(&self, other: &Self) -> Self {
        let mut result = self.clone();
        result.merge(other);
        result
    }

    fn is_empty(&self) -> bool {
        self.nodes.is_empty() || self.value() == 0
    }
}

/// Positive-Negative Counter (PN-Counter)
///
/// A counter that can be both incremented and decremented.
/// Useful for tracking counts that can go up and down.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PNCounter {
    /// Increments (positive counts)
    increments: GCounter,
    /// Decrements (negative counts)
    decrements: GCounter,
}

impl PNCounter {
    pub fn new() -> Self {
        Self {
            increments: GCounter::new(),
            decrements: GCounter::new(),
        }
    }

    /// Increment the counter
    pub fn increment(&mut self, node_id: &str) {
        self.increments.increment(node_id);
    }

    /// Decrement the counter
    pub fn decrement(&mut self, node_id: &str) {
        self.decrements.increment(node_id);
    }

    /// Get the current value
    pub fn value(&self) -> i64 {
        self.increments.value() as i64 - self.decrements.value() as i64
    }
}

impl CRDT for PNCounter {
    fn merge(&mut self, other: &Self) {
        self.increments.merge(&other.increments);
        self.decrements.merge(&other.decrements);
    }

    fn merged(&self, other: &Self) -> Self {
        let mut result = self.clone();
        result.merge(other);
        result
    }

    fn is_empty(&self) -> bool {
        self.value() == 0
    }
}

/// Observed-Removed Set (OR-Set)
///
/// A set that supports add and remove operations. Each element
/// has a unique tag, so concurrent add/remove can be resolved.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ORSet<T: Clone + Eq + std::hash::Hash> {
    /// Elements with their unique tags
    elements: HashMap<T, HashSet<u64>>,
    /// Removed tags (tombstones)
    removed: HashSet<u64>,
    /// Tag counter for this node
    tag_counter: u64,
    /// Node ID for generating unique tags
    node_id: String,
}

impl<T: Clone + Eq + std::hash::Hash> ORSet<T> {
    pub fn new(node_id: impl Into<String>) -> Self {
        Self {
            elements: HashMap::new(),
            removed: HashSet::new(),
            tag_counter: 0,
            node_id: node_id.into(),
        }
    }

    /// Add an element to the set
    pub fn add(&mut self, element: T) {
        self.tag_counter += 1;
        let tag = generate_tag(&self.node_id, self.tag_counter);
        self.elements.entry(element).or_default().insert(tag);
    }

    /// Remove an element from the set
    pub fn remove(&mut self, element: &T) {
        if let Some(tags) = self.elements.remove(element) {
            for tag in tags {
                self.removed.insert(tag);
            }
        }
    }

    /// Check if element is in the set
    pub fn contains(&self, element: &T) -> bool {
        self.elements.contains_key(element)
    }

    /// Get all elements
    pub fn elements(&self) -> impl Iterator<Item = &T> {
        self.elements.keys()
    }

    /// Get count of elements
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
}

impl<T: Clone + Eq + std::hash::Hash> CRDT for ORSet<T> {
    fn merge(&mut self, other: &Self) {
        // Add all elements from other
        for (element, tags) in &other.elements {
            let entry = self.elements.entry(element.clone()).or_default();
            for tag in tags {
                if !self.removed.contains(tag) {
                    entry.insert(*tag);
                }
            }
        }

        // Apply other's removed tags
        for tag in &other.removed {
            self.removed.insert(*tag);
        }

        // Clean up elements whose tags are all removed
        self.elements.retain(|_, tags| {
            tags.retain(|tag| !self.removed.contains(tag));
            !tags.is_empty()
        });
    }

    fn merged(&self, other: &Self) -> Self {
        let mut result = self.clone();
        result.merge(other);
        result
    }

    fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
}

/// Generate a unique tag for OR-Set elements
fn generate_tag(node_id: &str, counter: u64) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    node_id.hash(&mut hasher);
    counter.hash(&mut hasher);
    hasher.finish()
}

/// Get current timestamp
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Container for document CRDT fields
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CRDTDocument {
    /// Regular fields (non-CRDT)
    pub regular_fields: serde_json::Map<String, serde_json::Value>,
    /// CRDT counters
    pub counters: HashMap<String, PNCounter>,
    /// CRDT sets (using String elements for simplicity)
    pub sets: HashMap<String, ORSet<String>>,
    /// CRDT registers
    pub registers: HashMap<String, LWWRegister<serde_json::Value>>,
}

impl CRDTDocument {
    pub fn new() -> Self {
        Self {
            regular_fields: serde_json::Map::new(),
            counters: HashMap::new(),
            sets: HashMap::new(),
            registers: HashMap::new(),
        }
    }

    /// Merge another CRDT document into this one
    pub fn merge(&mut self, other: &Self, _node_id: &str) {
        // Merge regular fields (LWW based on timestamp)
        for (key, value) in &other.regular_fields {
            if let Some(existing) = self.regular_fields.get(key) {
                // Check if other has newer timestamp in _modified field
                let other_time = value.get("_modified").and_then(|v| v.as_u64()).unwrap_or(0);
                let self_time = existing
                    .get("_modified")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                if other_time > self_time {
                    self.regular_fields.insert(key.clone(), value.clone());
                }
            } else {
                self.regular_fields.insert(key.clone(), value.clone());
            }
        }

        // Merge CRDT counters
        for (key, other_counter) in &other.counters {
            if let Some(self_counter) = self.counters.get_mut(key) {
                self_counter.merge(other_counter);
            } else {
                self.counters.insert(key.clone(), other_counter.clone());
            }
        }

        // Merge CRDT sets
        for (key, other_set) in &other.sets {
            if let Some(self_set) = self.sets.get_mut(key) {
                self_set.merge(other_set);
            } else {
                self.sets.insert(key.clone(), other_set.clone());
            }
        }

        // Merge CRDT registers
        for (key, other_reg) in &other.registers {
            if let Some(self_reg) = self.registers.get_mut(key) {
                self_reg.merge(other_reg);
            } else {
                self.registers.insert(key.clone(), other_reg.clone());
            }
        }
    }

    /// Convert to JSON for storage
    pub fn to_json(&self) -> serde_json::Value {
        let mut result = serde_json::Map::new();

        // Add regular fields
        for (key, value) in &self.regular_fields {
            result.insert(key.clone(), value.clone());
        }

        // Add CRDT fields with type annotations
        for (key, counter) in &self.counters {
            let mut obj = serde_json::Map::new();
            obj.insert("_type".to_string(), serde_json::json!("PNCounter"));
            obj.insert("_value".to_string(), serde_json::json!(counter.value()));
            result.insert(key.clone(), serde_json::Value::Object(obj));
        }

        for (key, set) in &self.sets {
            let elements: Vec<_> = set.elements().cloned().collect();
            let mut obj = serde_json::Map::new();
            obj.insert("_type".to_string(), serde_json::json!("ORSet"));
            obj.insert("_value".to_string(), serde_json::json!(elements));
            result.insert(key.clone(), serde_json::Value::Object(obj));
        }

        serde_json::Value::Object(result)
    }
}

impl Default for CRDTDocument {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lww_register() {
        let mut reg1 = LWWRegister::new("value1", "node1");
        let mut reg2 = LWWRegister::new("value2", "node2");

        // Simulate reg2 being updated later
        reg2.set("value2");

        reg1.merge(&reg2);
        assert_eq!(reg1.value(), &"value2");
    }

    #[test]
    fn test_g_counter() {
        let mut counter1 = GCounter::new();
        let mut counter2 = GCounter::new();

        counter1.increment("node1");
        counter1.increment("node1");
        counter2.increment("node2");
        counter2.increment("node2");
        counter2.increment("node2");

        counter1.merge(&counter2);
        assert_eq!(counter1.value(), 5);
    }

    #[test]
    fn test_pn_counter() {
        let mut counter1 = PNCounter::new();
        let mut counter2 = PNCounter::new();

        counter1.increment("node1");
        counter1.increment("node1");
        counter2.decrement("node2");

        counter1.merge(&counter2);
        assert_eq!(counter1.value(), 1); // 2 - 1 = 1
    }

    #[test]
    fn test_or_set() {
        let mut set1 = ORSet::new("node1");
        let mut set2 = ORSet::new("node2");

        set1.add("a".to_string());
        set1.add("b".to_string());
        set2.add("b".to_string());
        set2.add("c".to_string());

        set1.merge(&set2);

        assert!(set1.contains(&"a".to_string()));
        assert!(set1.contains(&"b".to_string()));
        assert!(set1.contains(&"c".to_string()));
    }

    #[test]
    fn test_or_set_remove() {
        let mut set1 = ORSet::new("node1");
        let mut set2 = ORSet::new("node2");

        // Both add "a"
        set1.add("a".to_string());
        set2.add("a".to_string());

        // set1 removes "a"
        set1.remove(&"a".to_string());

        // Merge - "a" should still be present (from set2)
        set1.merge(&set2);
        assert!(set1.contains(&"a".to_string()));
    }

    #[test]
    fn test_crdt_document() {
        let mut doc1 = CRDTDocument::new();
        let mut doc2 = CRDTDocument::new();

        // Add counters
        let mut counter1 = PNCounter::new();
        counter1.increment("node1");
        counter1.increment("node1");
        doc1.counters.insert("likes".to_string(), counter1);

        let mut counter2 = PNCounter::new();
        counter2.increment("node2");
        doc2.counters.insert("likes".to_string(), counter2);

        // Merge
        doc1.merge(&doc2, "node1");

        assert_eq!(doc1.counters.get("likes").unwrap().value(), 3);
    }
}
