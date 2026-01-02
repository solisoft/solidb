//! Authentication & Authorization for Lua Scripts
//!
//! This module provides auth functions for Lua scripts in SoliDB:
//! - solidb.auth.user() - Get current user info
//! - solidb.auth.has_role(role) - Check if user has a role
//! - solidb.auth.require_role(role) - Guard that requires a role

use mlua::{Lua, Result as LuaResult, Function, Table};
use serde::{Deserialize, Serialize};

/// User information available to Lua scripts
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScriptUser {
    /// Username (subject from JWT claims)
    pub username: String,
    /// User roles
    pub roles: Vec<String>,
    /// Whether user is authenticated
    pub authenticated: bool,
    /// Scoped databases (if any)
    pub scoped_databases: Option<Vec<String>>,
    /// Token expiration timestamp
    pub exp: Option<u64>,
}

impl ScriptUser {
    /// Create an unauthenticated user
    pub fn anonymous() -> Self {
        Self {
            username: String::new(),
            roles: vec![],
            authenticated: false,
            scoped_databases: None,
            exp: None,
        }
    }

    /// Check if user has a specific role
    pub fn has_role(&self, role: &str) -> bool {
        // Admin role has all permissions
        if self.roles.contains(&"admin".to_string()) {
            return true;
        }
        self.roles.contains(&role.to_string())
    }
}

/// Create the solidb.auth table with authentication functions
pub fn create_auth_table(lua: &Lua, user: &ScriptUser) -> LuaResult<Table> {
    let auth_table = lua.create_table()?;

    // Store user info for access by functions
    let user_clone = user.clone();

    // solidb.auth.user() -> table with user info
    let user_for_fn = user.clone();
    let user_fn = lua.create_function(move |lua, (): ()| {
        let user_table = lua.create_table()?;
        user_table.set("username", user_for_fn.username.clone())?;
        user_table.set("authenticated", user_for_fn.authenticated)?;

        // Roles array
        let roles_table = lua.create_table()?;
        for (i, role) in user_for_fn.roles.iter().enumerate() {
            roles_table.set(i + 1, role.clone())?;
        }
        user_table.set("roles", roles_table)?;

        // Scoped databases (if any)
        if let Some(ref dbs) = user_for_fn.scoped_databases {
            let dbs_table = lua.create_table()?;
            for (i, db) in dbs.iter().enumerate() {
                dbs_table.set(i + 1, db.clone())?;
            }
            user_table.set("scoped_databases", dbs_table)?;
        }

        // Expiration
        if let Some(exp) = user_for_fn.exp {
            user_table.set("exp", exp)?;
        }

        Ok(user_table)
    })?;
    auth_table.set("user", user_fn)?;

    // solidb.auth.has_role(role) -> boolean
    let has_role_fn = create_has_role_function(lua, &user_clone)?;
    auth_table.set("has_role", has_role_fn)?;

    // solidb.auth.require_role(role) -> throws error if missing
    let require_role_fn = create_require_role_function(lua, &user_clone)?;
    auth_table.set("require_role", require_role_fn)?;

    // solidb.auth.is_authenticated() -> boolean
    let is_auth = user_clone.authenticated;
    let is_authenticated_fn = lua.create_function(move |_, (): ()| {
        Ok(is_auth)
    })?;
    auth_table.set("is_authenticated", is_authenticated_fn)?;

    // solidb.auth.require_auth() -> throws error if not authenticated
    let require_auth_fn = create_require_auth_function(lua, &user_clone)?;
    auth_table.set("require_auth", require_auth_fn)?;

    // solidb.auth.has_database_access(db_name) -> boolean
    let has_db_access_fn = create_has_database_access_function(lua, &user_clone)?;
    auth_table.set("has_database_access", has_db_access_fn)?;

    Ok(auth_table)
}

/// Create solidb.auth.has_role(role) function
fn create_has_role_function(lua: &Lua, user: &ScriptUser) -> LuaResult<Function> {
    let roles = user.roles.clone();
    let is_admin = roles.contains(&"admin".to_string());

    lua.create_function(move |_, role: String| {
        // Admin has all roles
        if is_admin {
            return Ok(true);
        }
        Ok(roles.contains(&role))
    })
}

/// Create solidb.auth.require_role(role) function
fn create_require_role_function(lua: &Lua, user: &ScriptUser) -> LuaResult<Function> {
    let roles = user.roles.clone();
    let is_admin = roles.contains(&"admin".to_string());
    let authenticated = user.authenticated;

    lua.create_function(move |_, role: String| {
        if !authenticated {
            return Err(mlua::Error::RuntimeError(
                "UNAUTHORIZED:401:Authentication required".to_string()
            ));
        }

        // Admin has all roles
        if is_admin {
            return Ok(true);
        }

        if !roles.contains(&role) {
            return Err(mlua::Error::RuntimeError(
                format!("FORBIDDEN:403:Role '{}' required", role)
            ));
        }

        Ok(true)
    })
}

/// Create solidb.auth.require_auth() function
fn create_require_auth_function(lua: &Lua, user: &ScriptUser) -> LuaResult<Function> {
    let authenticated = user.authenticated;

    lua.create_function(move |_, (): ()| {
        if !authenticated {
            return Err(mlua::Error::RuntimeError(
                "UNAUTHORIZED:401:Authentication required".to_string()
            ));
        }
        Ok(true)
    })
}

/// Create solidb.auth.has_database_access(db_name) function
fn create_has_database_access_function(lua: &Lua, user: &ScriptUser) -> LuaResult<Function> {
    let scoped_databases = user.scoped_databases.clone();
    let is_admin = user.roles.contains(&"admin".to_string());

    lua.create_function(move |_, db_name: String| {
        // Admin has access to all databases
        if is_admin {
            return Ok(true);
        }

        // If no scoping, user has access to all databases they're authorized for
        match &scoped_databases {
            None => Ok(true),
            Some(dbs) => Ok(dbs.contains(&db_name)),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_user_has_role() {
        let user = ScriptUser {
            username: "test".to_string(),
            roles: vec!["editor".to_string(), "viewer".to_string()],
            authenticated: true,
            scoped_databases: None,
            exp: None,
        };

        assert!(user.has_role("editor"));
        assert!(user.has_role("viewer"));
        assert!(!user.has_role("admin"));
    }

    #[test]
    fn test_admin_has_all_roles() {
        let admin = ScriptUser {
            username: "admin".to_string(),
            roles: vec!["admin".to_string()],
            authenticated: true,
            scoped_databases: None,
            exp: None,
        };

        assert!(admin.has_role("admin"));
        assert!(admin.has_role("editor"));
        assert!(admin.has_role("anything"));
    }

    #[test]
    fn test_anonymous_user() {
        let anon = ScriptUser::anonymous();
        assert!(!anon.authenticated);
        assert!(anon.roles.is_empty());
        assert!(!anon.has_role("viewer"));
    }

    #[test]
    fn test_auth_table_creation() {
        let lua = Lua::new();
        let user = ScriptUser {
            username: "testuser".to_string(),
            roles: vec!["editor".to_string()],
            authenticated: true,
            scoped_databases: None,
            exp: Some(1735689600),
        };

        let auth_table = create_auth_table(&lua, &user).unwrap();

        // Test user function
        let user_fn: Function = auth_table.get("user").unwrap();
        let user_info: Table = user_fn.call(()).unwrap();

        let username: String = user_info.get("username").unwrap();
        assert_eq!(username, "testuser");

        let authenticated: bool = user_info.get("authenticated").unwrap();
        assert!(authenticated);
    }

    #[test]
    fn test_has_role_function() {
        let lua = Lua::new();
        let user = ScriptUser {
            username: "testuser".to_string(),
            roles: vec!["editor".to_string()],
            authenticated: true,
            scoped_databases: None,
            exp: None,
        };

        let auth_table = create_auth_table(&lua, &user).unwrap();
        let has_role_fn: Function = auth_table.get("has_role").unwrap();

        let has_editor: bool = has_role_fn.call("editor").unwrap();
        assert!(has_editor);

        let has_admin: bool = has_role_fn.call("admin").unwrap();
        assert!(!has_admin);
    }

    #[test]
    fn test_require_role_success() {
        let lua = Lua::new();
        let user = ScriptUser {
            username: "testuser".to_string(),
            roles: vec!["editor".to_string()],
            authenticated: true,
            scoped_databases: None,
            exp: None,
        };

        let auth_table = create_auth_table(&lua, &user).unwrap();
        let require_role_fn: Function = auth_table.get("require_role").unwrap();

        let result: bool = require_role_fn.call("editor").unwrap();
        assert!(result);
    }

    #[test]
    fn test_require_role_failure() {
        let lua = Lua::new();
        let user = ScriptUser {
            username: "testuser".to_string(),
            roles: vec!["viewer".to_string()],
            authenticated: true,
            scoped_databases: None,
            exp: None,
        };

        let auth_table = create_auth_table(&lua, &user).unwrap();
        let require_role_fn: Function = auth_table.get("require_role").unwrap();

        let result: Result<bool, _> = require_role_fn.call("admin");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("FORBIDDEN"));
    }

    #[test]
    fn test_require_auth_unauthenticated() {
        let lua = Lua::new();
        let user = ScriptUser::anonymous();

        let auth_table = create_auth_table(&lua, &user).unwrap();
        let require_auth_fn: Function = auth_table.get("require_auth").unwrap();

        let result: Result<bool, _> = require_auth_fn.call(());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("UNAUTHORIZED"));
    }
}
