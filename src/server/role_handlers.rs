//! Role and user management API handlers for RBAC
//!
//! Provides endpoints for:
//! - Role CRUD operations (admin only)
//! - User role assignment/revocation (admin only)
//! - Self-service permission queries

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::error::DbError;
use crate::server::auth::{Claims, ADMIN_COLL};
use crate::server::authorization::{
    AuthorizationService, Permission, PermissionAction, PermissionScope, Role, UserRole,
    ROLES_COLLECTION, USER_ROLES_COLLECTION,
};
use crate::server::handlers::AppState;
use crate::sync::{LogEntry, Operation};

// ===========================================
// Request/Response Types
// ===========================================

#[derive(Debug, Deserialize)]
pub struct CreateRoleRequest {
    pub name: String,
    pub description: Option<String>,
    pub permissions: Vec<PermissionInput>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub description: Option<String>,
    pub permissions: Option<Vec<PermissionInput>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PermissionInput {
    pub action: String,   // "admin", "write", "read"
    pub scope: String,    // "global", "database"
    pub database: Option<String>,
}

impl PermissionInput {
    fn to_permission(&self) -> Result<Permission, DbError> {
        let action = match self.action.to_lowercase().as_str() {
            "admin" => PermissionAction::Admin,
            "write" => PermissionAction::Write,
            "read" => PermissionAction::Read,
            other => return Err(DbError::BadRequest(format!("Invalid action: {}", other))),
        };

        let scope = match self.scope.to_lowercase().as_str() {
            "global" => PermissionScope::Global,
            "database" => PermissionScope::Database,
            other => return Err(DbError::BadRequest(format!("Invalid scope: {}", other))),
        };

        if scope == PermissionScope::Database && self.database.is_none() {
            return Err(DbError::BadRequest(
                "database field is required for database-scoped permissions".to_string(),
            ));
        }

        Ok(Permission {
            action,
            scope,
            database: self.database.clone(),
        })
    }
}

#[derive(Debug, Serialize)]
pub struct RoleResponse {
    pub name: String,
    pub description: String,
    pub permissions: Vec<PermissionOutput>,
    pub is_builtin: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct PermissionOutput {
    pub action: String,
    pub scope: String,
    pub database: Option<String>,
}

impl From<&Permission> for PermissionOutput {
    fn from(p: &Permission) -> Self {
        Self {
            action: match p.action {
                PermissionAction::Admin => "admin".to_string(),
                PermissionAction::Write => "write".to_string(),
                PermissionAction::Read => "read".to_string(),
            },
            scope: match p.scope {
                PermissionScope::Global => "global".to_string(),
                PermissionScope::Database => "database".to_string(),
            },
            database: p.database.clone(),
        }
    }
}

impl From<&Role> for RoleResponse {
    fn from(role: &Role) -> Self {
        Self {
            name: role.name.clone(),
            description: role.description.clone().unwrap_or_default(),
            permissions: role.permissions.iter().map(PermissionOutput::from).collect(),
            is_builtin: role.is_builtin,
            created_at: role.created_at.clone(),
            updated_at: role.updated_at.clone().unwrap_or_else(|| role.created_at.clone()),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AssignRoleRequest {
    pub role: String,
    pub database: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UserRoleResponse {
    pub id: String,
    pub username: String,
    pub role: String,
    pub database: Option<String>,
    pub assigned_at: String,
    pub assigned_by: String,
}

impl From<&UserRole> for UserRoleResponse {
    fn from(ur: &UserRole) -> Self {
        Self {
            id: ur.id.clone(),
            username: ur.username.clone(),
            role: ur.role.clone(),
            database: ur.database.clone(),
            assigned_at: ur.assigned_at.clone(),
            assigned_by: ur.assigned_by.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CurrentUserResponse {
    pub username: String,
    pub roles: Vec<String>,
    pub permissions: Vec<PermissionOutput>,
}

// ===========================================
// Role Management Handlers (Admin Only)
// ===========================================

/// List all roles
pub async fn list_roles(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<Vec<RoleResponse>>, DbError> {
    // Check admin permission
    AuthorizationService::check_permission(&claims, &state, PermissionAction::Admin, None).await?;

    let db = state.storage.get_database("_system")?;
    let collection = db.get_collection(ROLES_COLLECTION)?;

    let roles: Vec<Role> = collection
        .scan(None)
        .into_iter()
        .filter_map(|doc| {
            // doc.data doesn't include _key, need to merge it
            let mut data = doc.data.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert("_key".to_string(), serde_json::Value::String(doc.key.clone()));
            }
            serde_json::from_value(data).ok()
        })
        .collect();

    Ok(Json(roles.iter().map(RoleResponse::from).collect()))
}

/// Create a new custom role
pub async fn create_role(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<CreateRoleRequest>,
) -> Result<(StatusCode, Json<RoleResponse>), DbError> {
    // Check admin permission
    AuthorizationService::check_permission(&claims, &state, PermissionAction::Admin, None).await?;

    // Validate role name
    if req.name.starts_with("admin") || req.name.starts_with("editor") || req.name.starts_with("viewer") {
        return Err(DbError::BadRequest(
            "Cannot create role with reserved name prefix (admin, editor, viewer)".to_string(),
        ));
    }

    // Parse permissions
    let permissions: Result<Vec<Permission>, _> = req
        .permissions
        .iter()
        .map(|p| p.to_permission())
        .collect();
    let permissions = permissions?;

    let now = chrono::Utc::now().to_rfc3339();
    let role = Role {
        name: req.name.clone(),
        description: req.description.clone(),
        permissions,
        is_builtin: false,
        created_at: now.clone(),
        updated_at: Some(now),
    };

    let db = state.storage.get_database("_system")?;
    let collection = db.get_collection(ROLES_COLLECTION)?;

    // Check if role already exists
    if collection.get(&req.name).is_ok() {
        return Err(DbError::ConflictError(format!(
            "Role '{}' already exists",
            req.name
        )));
    }

    let doc_value = serde_json::to_value(&role)?;
    collection.insert(doc_value.clone())?;

    // Record for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: "_system".to_string(),
            collection: ROLES_COLLECTION.to_string(),
            operation: Operation::Insert,
            key: req.name.clone(),
            data: serde_json::to_vec(&doc_value).ok(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        log.append(entry);
    }

    // Invalidate permission cache
    state.permission_cache.clear();

    Ok((StatusCode::CREATED, Json(RoleResponse::from(&role))))
}

/// Get a specific role
pub async fn get_role(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(role_name): Path<String>,
) -> Result<Json<RoleResponse>, DbError> {
    // Check admin permission
    AuthorizationService::check_permission(&claims, &state, PermissionAction::Admin, None).await?;

    let db = state.storage.get_database("_system")?;
    let collection = db.get_collection(ROLES_COLLECTION)?;

    let doc = collection
        .get(&role_name)
        .map_err(|_| DbError::RoleNotFound(role_name.clone()))?;

    let role: Role = serde_json::from_value(doc.data)?;
    Ok(Json(RoleResponse::from(&role)))
}

/// Update a custom role
pub async fn update_role(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(role_name): Path<String>,
    Json(req): Json<UpdateRoleRequest>,
) -> Result<Json<RoleResponse>, DbError> {
    // Check admin permission
    AuthorizationService::check_permission(&claims, &state, PermissionAction::Admin, None).await?;

    let db = state.storage.get_database("_system")?;
    let collection = db.get_collection(ROLES_COLLECTION)?;

    let doc = collection
        .get(&role_name)
        .map_err(|_| DbError::RoleNotFound(role_name.clone()))?;

    let mut role: Role = serde_json::from_value(doc.data)?;

    // Cannot modify builtin roles
    if role.is_builtin {
        return Err(DbError::Forbidden(
            "Cannot modify builtin roles".to_string(),
        ));
    }

    // Update fields
    if let Some(desc) = req.description {
        role.description = Some(desc);
    }

    if let Some(perms) = req.permissions {
        let permissions: Result<Vec<Permission>, _> =
            perms.iter().map(|p| p.to_permission()).collect();
        role.permissions = permissions?;
    }

    role.updated_at = Some(chrono::Utc::now().to_rfc3339());

    let doc_value = serde_json::to_value(&role)?;
    collection.update(&role_name, doc_value.clone())?;

    // Record for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: "_system".to_string(),
            collection: ROLES_COLLECTION.to_string(),
            operation: Operation::Update,
            key: role_name.clone(),
            data: serde_json::to_vec(&doc_value).ok(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        log.append(entry);
    }

    // Invalidate cache for this role
    state.permission_cache.invalidate_role(&role_name);

    Ok(Json(RoleResponse::from(&role)))
}

/// Delete a custom role
pub async fn delete_role(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(role_name): Path<String>,
) -> Result<StatusCode, DbError> {
    // Check admin permission
    AuthorizationService::check_permission(&claims, &state, PermissionAction::Admin, None).await?;

    let db = state.storage.get_database("_system")?;
    let collection = db.get_collection(ROLES_COLLECTION)?;

    let doc = collection
        .get(&role_name)
        .map_err(|_| DbError::RoleNotFound(role_name.clone()))?;

    let role: Role = serde_json::from_value(doc.data)?;

    // Cannot delete builtin roles
    if role.is_builtin {
        return Err(DbError::Forbidden(
            "Cannot delete builtin roles".to_string(),
        ));
    }

    collection.delete(&role_name)?;

    // Record for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: "_system".to_string(),
            collection: ROLES_COLLECTION.to_string(),
            operation: Operation::Delete,
            key: role_name.clone(),
            data: None,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        log.append(entry);
    }

    // Invalidate cache for this role
    state.permission_cache.invalidate_role(&role_name);

    Ok(StatusCode::NO_CONTENT)
}

// ===========================================
// User Role Assignment Handlers (Admin Only)
// ===========================================

/// Get roles assigned to a user
pub async fn get_user_roles(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(username): Path<String>,
) -> Result<Json<Vec<UserRoleResponse>>, DbError> {
    // Check admin permission
    AuthorizationService::check_permission(&claims, &state, PermissionAction::Admin, None).await?;

    let db = state.storage.get_database("_system")?;
    let collection = db.get_collection(USER_ROLES_COLLECTION)?;

    let user_roles: Vec<UserRole> = collection
        .scan(None)
        .into_iter()
        .filter_map(|doc| {
            let mut data = doc.data.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert("_key".to_string(), serde_json::Value::String(doc.key.clone()));
            }
            serde_json::from_value::<UserRole>(data).ok()
        })
        .filter(|ur| ur.username == username)
        .collect();

    Ok(Json(user_roles.iter().map(UserRoleResponse::from).collect()))
}

/// Assign a role to a user
pub async fn assign_role(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(username): Path<String>,
    Json(req): Json<AssignRoleRequest>,
) -> Result<(StatusCode, Json<UserRoleResponse>), DbError> {
    // Check admin permission
    AuthorizationService::check_permission(&claims, &state, PermissionAction::Admin, None).await?;

    let db = state.storage.get_database("_system")?;

    // Verify role exists
    let roles_coll = db.get_collection(ROLES_COLLECTION)?;
    if roles_coll.get(&req.role).is_err() {
        return Err(DbError::RoleNotFound(req.role.clone()));
    }

    // Verify user exists
    let admins_coll = db.get_collection(ADMIN_COLL)?;
    if admins_coll.get(&username).is_err() {
        return Err(DbError::DocumentNotFound(format!("User '{}' not found", username)));
    }

    // Check if assignment already exists
    let user_roles_coll = db.get_collection(USER_ROLES_COLLECTION)?;
    let existing: Vec<UserRole> = user_roles_coll
        .scan(None)
        .into_iter()
        .filter_map(|doc| {
            let mut data = doc.data.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert("_key".to_string(), serde_json::Value::String(doc.key.clone()));
            }
            serde_json::from_value::<UserRole>(data).ok()
        })
        .filter(|ur| ur.username == username && ur.role == req.role && ur.database == req.database)
        .collect();

    if !existing.is_empty() {
        return Err(DbError::ConflictError(format!(
            "Role '{}' already assigned to user '{}'",
            req.role, username
        )));
    }

    let now = chrono::Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let user_role = UserRole {
        id: id.clone(),
        username: username.clone(),
        role: req.role.clone(),
        database: req.database.clone(),
        assigned_at: now,
        assigned_by: claims.sub.clone(),
    };

    let doc_value = serde_json::to_value(&user_role)?;
    user_roles_coll.insert(doc_value.clone())?;

    // Record for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: "_system".to_string(),
            collection: USER_ROLES_COLLECTION.to_string(),
            operation: Operation::Insert,
            key: id,
            data: serde_json::to_vec(&doc_value).ok(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        log.append(entry);
    }

    // Invalidate cache for this user
    state.permission_cache.invalidate(&username);

    Ok((StatusCode::CREATED, Json(UserRoleResponse::from(&user_role))))
}

/// Revoke a role from a user
/// Query params for revoke_role
#[derive(Debug, Deserialize)]
pub struct RevokeRoleQuery {
    pub database: Option<String>,
}

pub async fn revoke_role(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((username, role_name)): Path<(String, String)>,
    Query(query): Query<RevokeRoleQuery>,
) -> Result<StatusCode, DbError> {
    // Check admin permission
    AuthorizationService::check_permission(&claims, &state, PermissionAction::Admin, None).await?;

    let db = state.storage.get_database("_system")?;
    let collection = db.get_collection(USER_ROLES_COLLECTION)?;

    // Find the assignment (match database scope if provided)
    let assignment: Option<UserRole> = collection
        .scan(None)
        .into_iter()
        .filter_map(|doc| {
            let mut data = doc.data.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert("_key".to_string(), serde_json::Value::String(doc.key.clone()));
            }
            serde_json::from_value::<UserRole>(data).ok()
        })
        .find(|ur| ur.username == username && ur.role == role_name && ur.database == query.database);

    let assignment = assignment.ok_or_else(|| {
        DbError::DocumentNotFound(format!(
            "Role '{}' not assigned to user '{}'",
            role_name, username
        ))
    })?;

    collection.delete(&assignment.id)?;

    // Record for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: "_system".to_string(),
            collection: USER_ROLES_COLLECTION.to_string(),
            operation: Operation::Delete,
            key: assignment.id,
            data: None,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        log.append(entry);
    }

    // Invalidate cache for this user
    state.permission_cache.invalidate(&username);

    Ok(StatusCode::NO_CONTENT)
}

// ===========================================
// Self-Service Handlers
// ===========================================

/// Get current user info and permissions
pub async fn get_current_user(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<CurrentUserResponse>, DbError> {
    let permissions = AuthorizationService::get_effective_permissions(&claims, &state).await?;

    let roles = claims.roles.clone().unwrap_or_default();

    Ok(Json(CurrentUserResponse {
        username: claims.sub.clone(),
        roles,
        permissions: permissions.iter().map(PermissionOutput::from).collect(),
    }))
}

/// Get current user's effective permissions
pub async fn get_my_permissions(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<Vec<PermissionOutput>>, DbError> {
    let permissions = AuthorizationService::get_effective_permissions(&claims, &state).await?;
    Ok(Json(permissions.iter().map(PermissionOutput::from).collect()))
}

// ===========================================
// User Management Handlers (Admin Only)
// ===========================================

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub initial_role: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub username: String,
    pub created_at: Option<String>,
    pub roles: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct UsersListResponse {
    pub users: Vec<UserResponse>,
}

/// List all users
pub async fn list_users(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<UsersListResponse>, DbError> {
    // Check admin permission
    AuthorizationService::check_permission(&claims, &state, PermissionAction::Admin, None).await?;

    let db = state.storage.get_database("_system")?;
    let collection = db.get_collection(ADMIN_COLL)?;

    // Collect users with their created_at timestamps from documents
    let users_with_timestamps: Vec<(crate::server::auth::User, Option<String>)> = collection
        .scan(None)
        .into_iter()
        .filter_map(|doc| {
            // doc.data doesn't include _key, need to merge it
            let mut data = doc.data.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert("_key".to_string(), serde_json::Value::String(doc.key.clone()));
            }
            let user: crate::server::auth::User = serde_json::from_value(data).ok()?;
            let created_at = Some(doc.created_at.to_rfc3339());
            Some((user, created_at))
        })
        .collect();

    // Get roles for each user
    let user_roles_coll = db.get_collection(USER_ROLES_COLLECTION)?;
    let all_user_roles: Vec<UserRole> = user_roles_coll
        .scan(None)
        .into_iter()
        .filter_map(|doc| {
            let mut data = doc.data.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert("_key".to_string(), serde_json::Value::String(doc.key.clone()));
            }
            serde_json::from_value(data).ok()
        })
        .collect();

    let users_with_roles: Vec<UserResponse> = users_with_timestamps
        .into_iter()
        .map(|(user, created_at)| {
            let roles: Vec<String> = all_user_roles
                .iter()
                .filter(|ur| ur.username == user.username)
                .map(|ur| ur.role.clone())
                .collect();
            UserResponse {
                username: user.username,
                created_at,
                roles,
            }
        })
        .collect();

    Ok(Json(UsersListResponse { users: users_with_roles }))
}

/// Create a new user
pub async fn create_user(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserResponse>), DbError> {
    // Check admin permission
    AuthorizationService::check_permission(&claims, &state, PermissionAction::Admin, None).await?;

    // Validate username
    if req.username.is_empty() || req.username.len() > 64 {
        return Err(DbError::BadRequest("Username must be 1-64 characters".to_string()));
    }

    // Validate password
    if req.password.len() < 6 {
        return Err(DbError::BadRequest("Password must be at least 6 characters".to_string()));
    }

    let db = state.storage.get_database("_system")?;
    let collection = db.get_collection(ADMIN_COLL)?;

    // Check if user already exists
    if collection.get(&req.username).is_ok() {
        return Err(DbError::ConflictError(format!("User '{}' already exists", req.username)));
    }

    // Hash password
    let password_hash = crate::server::auth::AuthService::hash_password(&req.password)?;

    let user = crate::server::auth::User {
        username: req.username.clone(),
        password_hash,
    };

    let doc_value = serde_json::to_value(&user)?;
    collection.insert(doc_value.clone())?;

    // Record for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: "_system".to_string(),
            collection: ADMIN_COLL.to_string(),
            operation: Operation::Insert,
            key: req.username.clone(),
            data: serde_json::to_vec(&doc_value).ok(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        log.append(entry);
    }

    // Assign initial role if specified
    let mut roles = Vec::new();
    if let Some(initial_role) = req.initial_role {
        // Verify role exists
        let roles_coll = db.get_collection(ROLES_COLLECTION)?;
        if roles_coll.get(&initial_role).is_ok() {
            let now = chrono::Utc::now().to_rfc3339();
            let id = uuid::Uuid::new_v4().to_string();
            let user_role = UserRole {
                id: id.clone(),
                username: req.username.clone(),
                role: initial_role.clone(),
                database: None,
                assigned_at: now,
                assigned_by: claims.sub.clone(),
            };

            let user_roles_coll = db.get_collection(USER_ROLES_COLLECTION)?;
            let role_doc = serde_json::to_value(&user_role)?;
            user_roles_coll.insert(role_doc.clone())?;

            // Record for replication
            if let Some(ref log) = state.replication_log {
                let entry = LogEntry {
                    sequence: 0,
                    node_id: "".to_string(),
                    database: "_system".to_string(),
                    collection: USER_ROLES_COLLECTION.to_string(),
                    operation: Operation::Insert,
                    key: id,
                    data: serde_json::to_vec(&role_doc).ok(),
                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                    origin_sequence: None,
                };
                log.append(entry);
            }

            roles.push(initial_role);
        }
    }

    Ok((StatusCode::CREATED, Json(UserResponse {
        username: req.username,
        created_at: Some(chrono::Utc::now().to_rfc3339()),
        roles,
    })))
}

/// Delete a user
pub async fn delete_user(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(username): Path<String>,
) -> Result<StatusCode, DbError> {
    // Check admin permission
    AuthorizationService::check_permission(&claims, &state, PermissionAction::Admin, None).await?;

    // Prevent deleting yourself
    if claims.sub == username {
        return Err(DbError::BadRequest("Cannot delete your own account".to_string()));
    }

    // Prevent deleting the default admin
    if username == "admin" {
        return Err(DbError::BadRequest("Cannot delete the default admin user".to_string()));
    }

    let db = state.storage.get_database("_system")?;
    let collection = db.get_collection(ADMIN_COLL)?;

    // Check if user exists
    if collection.get(&username).is_err() {
        return Err(DbError::DocumentNotFound(format!("User '{}' not found", username)));
    }

    // Delete user
    collection.delete(&username)?;

    // Record for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: "_system".to_string(),
            collection: ADMIN_COLL.to_string(),
            operation: Operation::Delete,
            key: username.clone(),
            data: None,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        log.append(entry);
    }

    // Delete all role assignments for this user
    let user_roles_coll = db.get_collection(USER_ROLES_COLLECTION)?;
    let user_roles: Vec<UserRole> = user_roles_coll
        .scan(None)
        .into_iter()
        .filter_map(|doc| serde_json::from_value::<UserRole>(doc.data).ok())
        .filter(|ur| ur.username == username)
        .collect();

    for ur in user_roles {
        user_roles_coll.delete(&ur.id)?;
        if let Some(ref log) = state.replication_log {
            let entry = LogEntry {
                sequence: 0,
                node_id: "".to_string(),
                database: "_system".to_string(),
                collection: USER_ROLES_COLLECTION.to_string(),
                operation: Operation::Delete,
                key: ur.id,
                data: None,
                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                origin_sequence: None,
            };
            log.append(entry);
        }
    }

    // Invalidate cache for this user
    state.permission_cache.invalidate(&username);

    Ok(StatusCode::NO_CONTENT)
}
