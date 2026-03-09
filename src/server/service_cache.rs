//! In-memory cache for service lookups to avoid hitting RocksDB on every script request.

use crate::scripting::Service;
use dashmap::DashMap;
use std::time::Instant;

/// Cached service entry with expiration
struct CachedService {
    service: Service,
    cached_at: Instant,
}

/// Cache for service metadata, keyed by (database, service_key).
///
/// Services rarely change, so a short TTL (5s) avoids stale data
/// while eliminating ~99% of RocksDB reads on the hot path.
pub struct ServiceCache {
    entries: DashMap<(String, String), CachedService>,
    ttl_secs: u64,
}

impl ServiceCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            entries: DashMap::new(),
            ttl_secs,
        }
    }

    /// Get a cached service if present and not expired.
    pub fn get(&self, db_name: &str, service_key: &str) -> Option<Service> {
        let key = (db_name.to_string(), service_key.to_string());
        if let Some(entry) = self.entries.get(&key) {
            if entry.cached_at.elapsed().as_secs() < self.ttl_secs {
                return Some(entry.service.clone());
            }
            // Expired — drop the ref before removing
            drop(entry);
            self.entries.remove(&key);
        }
        None
    }

    /// Insert or update a cached service.
    pub fn insert(&self, db_name: &str, service_key: &str, service: Service) {
        let key = (db_name.to_string(), service_key.to_string());
        self.entries.insert(
            key,
            CachedService {
                service,
                cached_at: Instant::now(),
            },
        );
    }

    /// Invalidate a specific service entry.
    pub fn invalidate(&self, db_name: &str, service_key: &str) {
        self.entries
            .remove(&(db_name.to_string(), service_key.to_string()));
    }
}
