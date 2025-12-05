use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Hybrid Logical Clock for distributed ordering of events.
/// Combines physical time with a logical counter to ensure unique, ordered timestamps
/// even when wall clocks are out of sync.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HybridLogicalClock {
    /// Physical time in milliseconds since UNIX epoch
    pub physical_time: u64,
    /// Logical counter for tie-breaking
    pub logical_counter: u32,
    /// Node ID for final tie-breaking (ensures global uniqueness)
    pub node_id: String,
}

impl HybridLogicalClock {
    /// Create a new HLC with the current time
    pub fn now(node_id: &str) -> Self {
        Self {
            physical_time: current_time_ms(),
            logical_counter: 0,
            node_id: node_id.to_string(),
        }
    }

    /// Create an HLC from components (for deserialization/testing)
    pub fn new(physical_time: u64, logical_counter: u32, node_id: String) -> Self {
        Self {
            physical_time,
            logical_counter,
            node_id,
        }
    }

    /// Generate a new timestamp that is guaranteed to be greater than self
    /// and greater than the current wall clock
    pub fn tick(&self, node_id: &str) -> Self {
        let now = current_time_ms();

        if now > self.physical_time {
            // Wall clock has advanced, reset counter
            Self {
                physical_time: now,
                logical_counter: 0,
                node_id: node_id.to_string(),
            }
        } else {
            // Wall clock hasn't advanced, increment counter
            Self {
                physical_time: self.physical_time,
                logical_counter: self.logical_counter + 1,
                node_id: node_id.to_string(),
            }
        }
    }

    /// Update this clock after receiving a message with another clock.
    /// Returns a new clock that is greater than both self and the received clock.
    pub fn receive(&self, other: &HybridLogicalClock, node_id: &str) -> Self {
        let now = current_time_ms();

        let (physical, logical) = if now > self.physical_time && now > other.physical_time {
            // Wall clock is ahead of both, reset counter
            (now, 0)
        } else if self.physical_time > other.physical_time {
            // Our clock is ahead
            if now >= self.physical_time {
                (now, 0)
            } else {
                (self.physical_time, self.logical_counter + 1)
            }
        } else if other.physical_time > self.physical_time {
            // Remote clock is ahead
            (other.physical_time, other.logical_counter + 1)
        } else {
            // Same physical time, take max logical and increment
            (self.physical_time, self.logical_counter.max(other.logical_counter) + 1)
        };

        Self {
            physical_time: physical,
            logical_counter: logical,
            node_id: node_id.to_string(),
        }
    }

    /// Compare two HLCs. Returns Ordering.
    pub fn compare(&self, other: &HybridLogicalClock) -> std::cmp::Ordering {
        match self.physical_time.cmp(&other.physical_time) {
            std::cmp::Ordering::Equal => {
                match self.logical_counter.cmp(&other.logical_counter) {
                    std::cmp::Ordering::Equal => self.node_id.cmp(&other.node_id),
                    other => other,
                }
            }
            other => other,
        }
    }

    /// Check if this HLC is greater than another
    pub fn is_newer_than(&self, other: &HybridLogicalClock) -> bool {
        self.compare(other) == std::cmp::Ordering::Greater
    }

    /// Serialize to a string for storage (sortable)
    pub fn to_string_key(&self) -> String {
        format!("{:016x}-{:08x}-{}", self.physical_time, self.logical_counter, self.node_id)
    }

    /// Parse from a string key
    pub fn from_string_key(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.splitn(3, '-').collect();
        if parts.len() != 3 {
            return None;
        }

        let physical_time = u64::from_str_radix(parts[0], 16).ok()?;
        let logical_counter = u32::from_str_radix(parts[1], 16).ok()?;
        let node_id = parts[2].to_string();

        Some(Self {
            physical_time,
            logical_counter,
            node_id,
        })
    }
}

impl Ord for HybridLogicalClock {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.compare(other)
    }
}

impl PartialOrd for HybridLogicalClock {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Thread-safe HLC generator for a single node
pub struct HlcGenerator {
    node_id: String,
    last_physical: AtomicU64,
    last_logical: AtomicU64,
}

impl HlcGenerator {
    pub fn new(node_id: String) -> Self {
        Self {
            node_id,
            last_physical: AtomicU64::new(0),
            last_logical: AtomicU64::new(0),
        }
    }

    /// Generate a new unique HLC timestamp
    pub fn now(&self) -> HybridLogicalClock {
        let now = current_time_ms();

        loop {
            let last_phys = self.last_physical.load(Ordering::SeqCst);
            let last_log = self.last_logical.load(Ordering::SeqCst);

            let (new_phys, new_log) = if now > last_phys {
                (now, 0)
            } else {
                (last_phys, last_log + 1)
            };

            // Try to update atomically
            if self.last_physical.compare_exchange(
                last_phys, new_phys, Ordering::SeqCst, Ordering::SeqCst
            ).is_ok() {
                self.last_logical.store(new_log, Ordering::SeqCst);
                return HybridLogicalClock {
                    physical_time: new_phys,
                    logical_counter: new_log as u32,
                    node_id: self.node_id.clone(),
                };
            }
            // CAS failed, retry
        }
    }

    /// Update after receiving a remote HLC
    pub fn receive(&self, remote: &HybridLogicalClock) -> HybridLogicalClock {
        let now = current_time_ms();

        loop {
            let last_phys = self.last_physical.load(Ordering::SeqCst);
            let last_log = self.last_logical.load(Ordering::SeqCst);

            let (new_phys, new_log) = if now > last_phys && now > remote.physical_time {
                (now, 0)
            } else if last_phys > remote.physical_time {
                if now >= last_phys {
                    (now, 0)
                } else {
                    (last_phys, last_log + 1)
                }
            } else if remote.physical_time > last_phys {
                (remote.physical_time, remote.logical_counter as u64 + 1)
            } else {
                (last_phys, last_log.max(remote.logical_counter as u64) + 1)
            };

            if self.last_physical.compare_exchange(
                last_phys, new_phys, Ordering::SeqCst, Ordering::SeqCst
            ).is_ok() {
                self.last_logical.store(new_log, Ordering::SeqCst);
                return HybridLogicalClock {
                    physical_time: new_phys,
                    logical_counter: new_log as u32,
                    node_id: self.node_id.clone(),
                };
            }
        }
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hlc_ordering() {
        let hlc1 = HybridLogicalClock::new(1000, 0, "node-1".to_string());
        let hlc2 = HybridLogicalClock::new(1000, 1, "node-1".to_string());
        let hlc3 = HybridLogicalClock::new(1001, 0, "node-1".to_string());

        assert!(hlc2.is_newer_than(&hlc1));
        assert!(hlc3.is_newer_than(&hlc2));
        assert!(hlc3.is_newer_than(&hlc1));
    }

    #[test]
    fn test_hlc_string_key_roundtrip() {
        let hlc = HybridLogicalClock::new(1234567890, 42, "node-abc".to_string());
        let key = hlc.to_string_key();
        let parsed = HybridLogicalClock::from_string_key(&key).unwrap();

        assert_eq!(hlc, parsed);
    }

    #[test]
    fn test_generator_monotonic() {
        let gen = HlcGenerator::new("test-node".to_string());

        let mut last = gen.now();
        for _ in 0..1000 {
            let current = gen.now();
            assert!(current.is_newer_than(&last));
            last = current;
        }
    }
}

