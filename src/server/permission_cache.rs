//! In-memory permission cache for fast authorization checks.
//!
//! This module provides a thread-safe cache for user permissions with TTL expiration.

use crate::server::authorization::{Permission, Role};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Default cache TTL: 60 seconds
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(60);

/// Cached permissions for a subject (user or API key)
#[derive(Clone, Debug)]
pub struct CachedPermissions {
    /// Set of resolved permissions
    pub permissions: HashSet<Permission>,
    /// Role names assigned to this subject
    pub roles: Vec<String>,
    /// Optional database restrictions (for API keys)
    pub scoped_databases: Option<Vec<String>>,
    /// When this entry was cached
    pub cached_at: Instant,
}

impl CachedPermissions {
    /// Create a new cached permissions entry
    pub fn new(
        permissions: HashSet<Permission>,
        roles: Vec<String>,
        scoped_databases: Option<Vec<String>>,
    ) -> Self {
        Self {
            permissions,
            roles,
            scoped_databases,
            cached_at: Instant::now(),
        }
    }

    /// Check if this cache entry has expired
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.cached_at.elapsed() > ttl
    }
}

/// Thread-safe permission cache
#[derive(Clone)]
pub struct PermissionCache {
    /// Subject (username or "api-key:{name}") -> cached permissions
    entries: Arc<RwLock<HashMap<String, CachedPermissions>>>,
    /// Role name -> role definition (for quick lookup without DB access)
    roles: Arc<RwLock<HashMap<String, Role>>>,
    /// Cache TTL
    ttl: Duration,
}

impl PermissionCache {
    /// Create a new permission cache with default TTL (60 seconds)
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            roles: Arc::new(RwLock::new(HashMap::new())),
            ttl: DEFAULT_CACHE_TTL,
        }
    }

    /// Create a new permission cache with custom TTL
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            roles: Arc::new(RwLock::new(HashMap::new())),
            ttl,
        }
    }

    /// Get cached permissions for a subject
    ///
    /// Returns None if not cached or if the cache entry has expired
    pub fn get(&self, subject: &str) -> Option<CachedPermissions> {
        let entries = self.entries.read().unwrap();
        entries.get(subject).and_then(|entry| {
            if entry.is_expired(self.ttl) {
                None
            } else {
                Some(entry.clone())
            }
        })
    }

    /// Store permissions for a subject
    pub fn set(&self, subject: String, permissions: CachedPermissions) {
        let mut entries = self.entries.write().unwrap();
        entries.insert(subject, permissions);

        // Periodically clean up expired entries
        if entries.len() % 100 == 0 {
            self.cleanup_expired_internal(&mut entries);
        }
    }

    /// Invalidate cache for a specific subject
    ///
    /// Called when a user's roles change
    pub fn invalidate(&self, subject: &str) {
        let mut entries = self.entries.write().unwrap();
        entries.remove(subject);
    }

    /// Invalidate all entries that have a specific role
    ///
    /// Called when a role's permissions change
    pub fn invalidate_role(&self, role_name: &str) {
        let mut entries = self.entries.write().unwrap();
        entries.retain(|_, entry| !entry.roles.iter().any(|r| r == role_name));
    }

    /// Invalidate all cache entries
    ///
    /// Called on major permission changes or server restart
    pub fn clear(&self) {
        let mut entries = self.entries.write().unwrap();
        entries.clear();
    }

    /// Clean up expired cache entries
    pub fn cleanup_expired(&self) {
        let mut entries = self.entries.write().unwrap();
        self.cleanup_expired_internal(&mut entries);
    }

    fn cleanup_expired_internal(&self, entries: &mut HashMap<String, CachedPermissions>) {
        entries.retain(|_, entry| !entry.is_expired(self.ttl));
    }

    /// Get a cached role by name
    pub fn get_role(&self, name: &str) -> Option<Role> {
        let roles = self.roles.read().unwrap();
        roles.get(name).cloned()
    }

    /// Cache a role definition
    pub fn set_role(&self, role: Role) {
        let mut roles = self.roles.write().unwrap();
        roles.insert(role.name.clone(), role);
    }

    /// Remove a role from cache
    pub fn remove_role(&self, name: &str) {
        let mut roles = self.roles.write().unwrap();
        roles.remove(name);
    }

    /// Get all cached roles
    pub fn get_all_roles(&self) -> Vec<Role> {
        let roles = self.roles.read().unwrap();
        roles.values().cloned().collect()
    }

    /// Initialize roles cache with built-in roles
    pub fn initialize_builtin_roles(&self) {
        let builtin_roles = Role::builtin_roles();
        let mut roles = self.roles.write().unwrap();
        for role in builtin_roles {
            roles.insert(role.name.clone(), role);
        }
    }

    /// Get the number of cached permission entries (for debugging/monitoring)
    #[allow(dead_code)]
    pub fn entry_count(&self) -> usize {
        let entries = self.entries.read().unwrap();
        entries.len()
    }

    /// Get the number of cached roles (for debugging/monitoring)
    #[allow(dead_code)]
    pub fn role_count(&self) -> usize {
        let roles = self.roles.read().unwrap();
        roles.len()
    }
}

impl Default for PermissionCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::authorization::PermissionAction;

    #[test]
    fn test_cache_set_and_get() {
        let cache = PermissionCache::new();

        let mut permissions = HashSet::new();
        permissions.insert(Permission::global_admin());

        let cached = CachedPermissions::new(permissions.clone(), vec!["admin".to_string()], None);

        cache.set("user1".to_string(), cached);

        let retrieved = cache.get("user1").unwrap();
        assert!(retrieved.permissions.contains(&Permission::global_admin()));
        assert_eq!(retrieved.roles, vec!["admin".to_string()]);
    }

    #[test]
    fn test_cache_expiration() {
        let cache = PermissionCache::with_ttl(Duration::from_millis(100));

        let permissions = HashSet::new();
        let cached = CachedPermissions::new(permissions, vec![], None);

        cache.set("user1".to_string(), cached);

        // Should be available immediately
        assert!(cache.get("user1").is_some());

        // Wait for expiration
        std::thread::sleep(Duration::from_millis(150));

        // Should be expired
        assert!(cache.get("user1").is_none());
    }

    #[test]
    fn test_invalidate_subject() {
        let cache = PermissionCache::new();

        let permissions = HashSet::new();
        let cached = CachedPermissions::new(permissions, vec!["editor".to_string()], None);

        cache.set("user1".to_string(), cached);
        assert!(cache.get("user1").is_some());

        cache.invalidate("user1");
        assert!(cache.get("user1").is_none());
    }

    #[test]
    fn test_invalidate_role() {
        let cache = PermissionCache::new();

        // User with editor role
        let permissions1 = HashSet::new();
        let cached1 = CachedPermissions::new(permissions1, vec!["editor".to_string()], None);
        cache.set("user1".to_string(), cached1);

        // User with admin role
        let permissions2 = HashSet::new();
        let cached2 = CachedPermissions::new(permissions2, vec!["admin".to_string()], None);
        cache.set("user2".to_string(), cached2);

        // Invalidate editor role
        cache.invalidate_role("editor");

        // User1 should be invalidated (had editor role)
        assert!(cache.get("user1").is_none());

        // User2 should still be cached (had admin role)
        assert!(cache.get("user2").is_some());
    }

    #[test]
    fn test_clear_cache() {
        let cache = PermissionCache::new();

        for i in 0..10 {
            let permissions = HashSet::new();
            let cached = CachedPermissions::new(permissions, vec![], None);
            cache.set(format!("user{}", i), cached);
        }

        assert_eq!(cache.entry_count(), 10);

        cache.clear();

        assert_eq!(cache.entry_count(), 0);
    }

    #[test]
    fn test_role_cache() {
        let cache = PermissionCache::new();

        // Initialize builtin roles
        cache.initialize_builtin_roles();

        // Should have 3 builtin roles
        assert_eq!(cache.role_count(), 3);

        // Get admin role
        let admin = cache.get_role("admin").unwrap();
        assert_eq!(admin.name, "admin");
        assert!(admin.is_builtin);

        // Get editor role
        let editor = cache.get_role("editor").unwrap();
        assert_eq!(editor.name, "editor");

        // Get viewer role
        let viewer = cache.get_role("viewer").unwrap();
        assert_eq!(viewer.name, "viewer");

        // Non-existent role
        assert!(cache.get_role("nonexistent").is_none());
    }

    #[test]
    fn test_scoped_databases() {
        let cache = PermissionCache::new();

        let permissions = HashSet::new();
        let scoped_dbs = Some(vec!["db1".to_string(), "db2".to_string()]);
        let cached = CachedPermissions::new(permissions, vec!["editor".to_string()], scoped_dbs);

        cache.set("api-key:test".to_string(), cached);

        let retrieved = cache.get("api-key:test").unwrap();
        assert_eq!(
            retrieved.scoped_databases,
            Some(vec!["db1".to_string(), "db2".to_string()])
        );
    }
}
