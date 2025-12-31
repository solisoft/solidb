//! Role-Based Access Control (RBAC) authorization service for SoliDB.
//!
//! This module provides permission checking and role management functionality.

use crate::error::{DbError, DbResult};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Permission action types
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionAction {
    /// Full access: create/delete databases, manage users, cluster ops
    Admin,
    /// Insert, update, delete documents; create indexes
    Write,
    /// Get, list, query (SELECT only)
    Read,
}

impl PermissionAction {
    /// Check if this action implies another action
    /// Admin > Write > Read
    pub fn implies(&self, other: &PermissionAction) -> bool {
        match self {
            PermissionAction::Admin => true, // Admin implies all
            PermissionAction::Write => matches!(other, PermissionAction::Write | PermissionAction::Read),
            PermissionAction::Read => matches!(other, PermissionAction::Read),
        }
    }
}

/// Permission scope types
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionScope {
    /// Permission applies to all databases
    Global,
    /// Permission applies to a specific database
    Database,
}

/// A single permission granting access to perform an action on a scope
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Permission {
    /// The action this permission grants
    pub action: PermissionAction,
    /// The scope of this permission
    pub scope: PermissionScope,
    /// Database name (None for global scope)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
}

impl Permission {
    /// Create a global admin permission
    pub fn global_admin() -> Self {
        Self {
            action: PermissionAction::Admin,
            scope: PermissionScope::Global,
            database: None,
        }
    }

    /// Create a global write permission
    pub fn global_write() -> Self {
        Self {
            action: PermissionAction::Write,
            scope: PermissionScope::Global,
            database: None,
        }
    }

    /// Create a global read permission
    pub fn global_read() -> Self {
        Self {
            action: PermissionAction::Read,
            scope: PermissionScope::Global,
            database: None,
        }
    }

    /// Create a database-scoped permission
    pub fn database_permission(action: PermissionAction, database: &str) -> Self {
        Self {
            action,
            scope: PermissionScope::Database,
            database: Some(database.to_string()),
        }
    }
}

/// Role definition stored in _system._roles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    /// Role name (also used as _key)
    #[serde(rename = "_key")]
    pub name: String,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Permissions granted by this role
    pub permissions: Vec<Permission>,
    /// Whether this is a built-in role (cannot be deleted)
    #[serde(default)]
    pub is_builtin: bool,
    /// Creation timestamp (RFC3339)
    pub created_at: String,
    /// Last update timestamp (RFC3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl Role {
    /// Create the built-in admin role
    pub fn builtin_admin() -> Self {
        Self {
            name: "admin".to_string(),
            description: Some("Full system access".to_string()),
            permissions: vec![Permission::global_admin()],
            is_builtin: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: None,
        }
    }

    /// Create the built-in editor role
    pub fn builtin_editor() -> Self {
        Self {
            name: "editor".to_string(),
            description: Some("Read and write access to all databases".to_string()),
            permissions: vec![Permission::global_write(), Permission::global_read()],
            is_builtin: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: None,
        }
    }

    /// Create the built-in viewer role
    pub fn builtin_viewer() -> Self {
        Self {
            name: "viewer".to_string(),
            description: Some("Read-only access to all databases".to_string()),
            permissions: vec![Permission::global_read()],
            is_builtin: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: None,
        }
    }

    /// Get all built-in roles
    pub fn builtin_roles() -> Vec<Self> {
        vec![Self::builtin_admin(), Self::builtin_editor(), Self::builtin_viewer()]
    }
}

/// User-to-role assignment stored in _system._user_roles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRole {
    /// Assignment ID (UUID, also used as _key)
    #[serde(rename = "_key")]
    pub id: String,
    /// Username (references _admins._key)
    pub username: String,
    /// Role name (references _roles._key)
    pub role: String,
    /// Database scope (None for global assignment)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    /// Assignment timestamp (RFC3339)
    pub assigned_at: String,
    /// Who assigned this role
    pub assigned_by: String,
}

impl UserRole {
    /// Create a new global role assignment
    pub fn new_global(username: &str, role: &str, assigned_by: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            username: username.to_string(),
            role: role.to_string(),
            database: None,
            assigned_at: chrono::Utc::now().to_rfc3339(),
            assigned_by: assigned_by.to_string(),
        }
    }

    /// Create a new database-scoped role assignment
    pub fn new_database_scoped(username: &str, role: &str, database: &str, assigned_by: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            username: username.to_string(),
            role: role.to_string(),
            database: Some(database.to_string()),
            assigned_at: chrono::Utc::now().to_rfc3339(),
            assigned_by: assigned_by.to_string(),
        }
    }
}

/// System collection names for RBAC
pub const ROLES_COLLECTION: &str = "_roles";
pub const USER_ROLES_COLLECTION: &str = "_user_roles";

/// Authorization service for checking permissions
pub struct AuthorizationService;

impl AuthorizationService {
    /// Check if the given permissions satisfy the required action on a resource
    ///
    /// # Arguments
    /// * `permissions` - Set of permissions the user has
    /// * `required_action` - The action being performed
    /// * `database` - Optional database name for scoped checks
    /// * `scoped_databases` - Optional list of databases the user is restricted to (for API keys)
    ///
    /// # Returns
    /// * `Ok(())` if permission is granted
    /// * `Err(DbError::Forbidden)` if permission is denied
    pub fn check_permission(
        permissions: &HashSet<Permission>,
        required_action: PermissionAction,
        database: Option<&str>,
        scoped_databases: Option<&[String]>,
    ) -> DbResult<()> {
        // Check database scope restriction (for API keys)
        if let (Some(scoped_dbs), Some(db)) = (scoped_databases, database) {
            if !scoped_dbs.iter().any(|d| d == db) {
                return Err(DbError::Forbidden(format!(
                    "Access denied: API key not authorized for database '{}'",
                    db
                )));
            }
        }

        // Check if user has global admin permission (implies all)
        if permissions.contains(&Permission::global_admin()) {
            return Ok(());
        }

        // Build the required permission
        let required = Permission {
            action: required_action.clone(),
            scope: if database.is_some() {
                PermissionScope::Database
            } else {
                PermissionScope::Global
            },
            database: database.map(String::from),
        };

        // Check if any permission satisfies the requirement
        if Self::has_permission(permissions, &required) {
            Ok(())
        } else {
            Err(DbError::Forbidden(format!(
                "Access denied: insufficient permissions for {:?} on {}",
                required_action,
                database.unwrap_or("global")
            )))
        }
    }

    /// Check if any permission in the set satisfies the requirement
    fn has_permission(permissions: &HashSet<Permission>, required: &Permission) -> bool {
        for perm in permissions {
            // Exact match
            if perm == required {
                return true;
            }

            // Check if action implies required action
            if !perm.action.implies(&required.action) {
                continue;
            }

            // Global scope covers all databases
            if perm.scope == PermissionScope::Global {
                return true;
            }

            // Database scope must match
            if perm.scope == PermissionScope::Database
                && required.scope == PermissionScope::Database
                && perm.database == required.database
            {
                return true;
            }
        }

        false
    }

    /// Resolve permissions from roles
    pub fn resolve_permissions(roles: &[Role]) -> HashSet<Permission> {
        let mut permissions = HashSet::new();
        for role in roles {
            for perm in &role.permissions {
                permissions.insert(perm.clone());
            }
        }
        permissions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_action_implies() {
        assert!(PermissionAction::Admin.implies(&PermissionAction::Admin));
        assert!(PermissionAction::Admin.implies(&PermissionAction::Write));
        assert!(PermissionAction::Admin.implies(&PermissionAction::Read));

        assert!(!PermissionAction::Write.implies(&PermissionAction::Admin));
        assert!(PermissionAction::Write.implies(&PermissionAction::Write));
        assert!(PermissionAction::Write.implies(&PermissionAction::Read));

        assert!(!PermissionAction::Read.implies(&PermissionAction::Admin));
        assert!(!PermissionAction::Read.implies(&PermissionAction::Write));
        assert!(PermissionAction::Read.implies(&PermissionAction::Read));
    }

    #[test]
    fn test_global_admin_implies_all() {
        let mut permissions = HashSet::new();
        permissions.insert(Permission::global_admin());

        // Admin should have access to everything
        assert!(AuthorizationService::check_permission(&permissions, PermissionAction::Admin, None, None).is_ok());
        assert!(AuthorizationService::check_permission(&permissions, PermissionAction::Write, None, None).is_ok());
        assert!(AuthorizationService::check_permission(&permissions, PermissionAction::Read, None, None).is_ok());
        assert!(AuthorizationService::check_permission(&permissions, PermissionAction::Write, Some("mydb"), None).is_ok());
    }

    #[test]
    fn test_global_write_implies_read() {
        let mut permissions = HashSet::new();
        permissions.insert(Permission::global_write());
        permissions.insert(Permission::global_read());

        // Write+Read should allow read and write but not admin
        assert!(AuthorizationService::check_permission(&permissions, PermissionAction::Read, None, None).is_ok());
        assert!(AuthorizationService::check_permission(&permissions, PermissionAction::Write, None, None).is_ok());
        assert!(AuthorizationService::check_permission(&permissions, PermissionAction::Admin, None, None).is_err());
    }

    #[test]
    fn test_database_scope_restriction() {
        let mut permissions = HashSet::new();
        permissions.insert(Permission::database_permission(PermissionAction::Write, "allowed_db"));

        // Should work for allowed_db
        assert!(AuthorizationService::check_permission(&permissions, PermissionAction::Write, Some("allowed_db"), None).is_ok());
        assert!(AuthorizationService::check_permission(&permissions, PermissionAction::Read, Some("allowed_db"), None).is_ok());

        // Should fail for other databases
        assert!(AuthorizationService::check_permission(&permissions, PermissionAction::Write, Some("other_db"), None).is_err());
    }

    #[test]
    fn test_api_key_scoped_databases() {
        let mut permissions = HashSet::new();
        permissions.insert(Permission::global_write());
        permissions.insert(Permission::global_read());

        let scoped_dbs = vec!["db1".to_string(), "db2".to_string()];

        // Should work for scoped databases
        assert!(AuthorizationService::check_permission(&permissions, PermissionAction::Write, Some("db1"), Some(&scoped_dbs)).is_ok());
        assert!(AuthorizationService::check_permission(&permissions, PermissionAction::Write, Some("db2"), Some(&scoped_dbs)).is_ok());

        // Should fail for non-scoped databases
        assert!(AuthorizationService::check_permission(&permissions, PermissionAction::Write, Some("db3"), Some(&scoped_dbs)).is_err());
    }

    #[test]
    fn test_builtin_roles() {
        let roles = Role::builtin_roles();
        assert_eq!(roles.len(), 3);

        let admin = &roles[0];
        assert_eq!(admin.name, "admin");
        assert!(admin.is_builtin);
        assert!(admin.permissions.contains(&Permission::global_admin()));

        let editor = &roles[1];
        assert_eq!(editor.name, "editor");
        assert!(editor.permissions.contains(&Permission::global_write()));
        assert!(editor.permissions.contains(&Permission::global_read()));

        let viewer = &roles[2];
        assert_eq!(viewer.name, "viewer");
        assert!(viewer.permissions.contains(&Permission::global_read()));
    }

    #[test]
    fn test_resolve_permissions() {
        let roles = vec![Role::builtin_editor(), Role::builtin_viewer()];
        let permissions = AuthorizationService::resolve_permissions(&roles);

        assert!(permissions.contains(&Permission::global_write()));
        assert!(permissions.contains(&Permission::global_read()));
        assert!(!permissions.contains(&Permission::global_admin()));
    }
}
