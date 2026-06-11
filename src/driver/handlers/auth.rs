use crate::driver::protocol::{DriverError, Response};
use crate::driver::DriverHandler;

/// Handle authentication (username/password or API key)
pub async fn handle_auth(
    handler: &mut DriverHandler,
    database: String,
    username: String,
    password: String,
    api_key: Option<String>,
) -> Response {
    // Verify requested database exists
    if let Err(e) = handler.storage.get_database(&database) {
        return Response::error(DriverError::DatabaseError(format!(
            "Database not found: {}",
            e
        )));
    }

    // Get the _system database for auth lookup
    let system_db = match handler.storage.get_database("_system") {
        Ok(db) => db,
        Err(e) => {
            return Response::error(DriverError::AuthError(format!(
                "System database error: {}",
                e
            )))
        }
    };

    // Check for API key authentication (if provided)
    if let Some(key) = api_key {
        return handle_api_key_auth(handler, &system_db, &database, &key).await;
    }

    // Username/password authentication
    // Get admins collection (username is the _key)
    let admins = match system_db.get_collection("_admins") {
        Ok(coll) => coll,
        Err(_) => {
            return Response::error(DriverError::AuthError(
                "Admins collection not found".to_string(),
            ))
        }
    };

    // Find user by username (username IS the _key in _admins collection)
    let user_doc = match admins.get(&username) {
        Ok(doc) => doc,
        Err(_) => {
            return Response::error(DriverError::AuthError("Invalid credentials".to_string()))
        }
    };

    // Parse user
    let user: crate::server::auth::User = match serde_json::from_value(user_doc.to_value()) {
        Ok(u) => u,
        Err(_) => {
            return Response::error(DriverError::AuthError("Invalid credentials".to_string()))
        }
    };

    // Verify password using AuthService
    if !crate::server::auth::AuthService::verify_password(&password, &user.password_hash) {
        return Response::error(DriverError::AuthError("Invalid credentials".to_string()));
    }

    // Resolve the user's roles to a permission snapshot for this connection;
    // every subsequent command is checked against it.
    let role_names = crate::server::auth::AuthService::get_user_roles(&handler.storage, &username)
        .unwrap_or_default();
    let permissions = crate::server::AuthorizationService::load_permissions_from_storage(
        &handler.storage,
        &role_names,
    );

    // The principal must at least read the database it authenticates against.
    if let Err(e) = crate::server::AuthorizationService::check_permission_raw(
        &permissions,
        crate::server::PermissionAction::Read,
        Some(&database),
        None,
    ) {
        return Response::error(DriverError::AuthError(e.to_string()));
    }

    handler.session_subject = username;
    handler.session_permissions = permissions;
    handler.session_scoped_databases = None;
    handler.authenticated_db = Some(database);
    Response::ok_empty()
}

async fn handle_api_key_auth(
    handler: &mut DriverHandler,
    _system_db: &crate::storage::Database,
    database: &str,
    api_key: &str,
) -> Response {
    // O(1) cached lookup (shared with the HTTP path) instead of scanning the
    // whole _api_keys collection per auth attempt.
    let api_key_data =
        match crate::server::auth::AuthService::lookup_api_key(&handler.storage, api_key) {
            Some(k) => k,
            None => return Response::error(DriverError::AuthError("Invalid API key".to_string())),
        };

    // Check if API key is expired
    if let Some(ref expires_at) = api_key_data.expires_at {
        if let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(expires_at) {
            if expiry < chrono::Utc::now() {
                return Response::error(DriverError::AuthError("API key expired".to_string()));
            }
        }
    }

    // Check database scope if applicable
    if let Some(scoped_dbs) = &api_key_data.scoped_databases {
        if !scoped_dbs.is_empty() && !scoped_dbs.contains(&database.to_string()) {
            return Response::error(DriverError::AuthError(
                "API key does not have access to this database".to_string(),
            ));
        }
    }

    // Resolve the key's roles to a permission snapshot for this connection.
    let permissions = crate::server::AuthorizationService::load_permissions_from_storage(
        &handler.storage,
        &api_key_data.roles,
    );
    let scoped = api_key_data
        .scoped_databases
        .clone()
        .filter(|dbs| !dbs.is_empty());

    if let Err(e) = crate::server::AuthorizationService::check_permission_raw(
        &permissions,
        crate::server::PermissionAction::Read,
        Some(database),
        scoped.as_deref(),
    ) {
        return Response::error(DriverError::AuthError(e.to_string()));
    }

    // Set authenticated state with API key name as identifier
    handler.session_subject = format!("apikey:{}", api_key_data.id);
    handler.session_permissions = permissions;
    handler.session_scoped_databases = scoped;
    handler.authenticated_db = Some(database.to_string());
    Response::ok_empty()
}
