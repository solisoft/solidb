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

    // Set authenticated state (admin users have access to all databases)
    handler.authenticated_db = Some(database);
    Response::ok_empty()
}

async fn handle_api_key_auth(
    handler: &mut DriverHandler,
    system_db: &crate::storage::Database,
    database: &str,
    api_key: &str,
) -> Response {
    // Get API keys collection
    let api_keys_coll = match system_db.get_collection("_api_keys") {
        Ok(coll) => coll,
        Err(_) => {
            return Response::error(DriverError::AuthError(
                "API keys collection not found".to_string(),
            ))
        }
    };

    // Find API key by hash
    let key_hash = crate::server::auth::AuthService::hash_api_key(api_key);

    // Search through API keys to find match using constant-time comparison
    let mut api_key_doc = None;
    for doc in api_keys_coll.scan(None) {
        let doc_clone = doc.clone();
        let doc_value = doc_clone.to_value();
        let hash_value_opt = doc_value.get("key_hash").and_then(|v| v.as_str());
        if let Some(hash_value) = hash_value_opt {
            if hash_value.len() == key_hash.len()
                && crate::server::auth::constant_time_eq(hash_value.as_bytes(), key_hash.as_bytes())
            {
                api_key_doc = Some(doc_clone);
                break;
            }
        }
    }

    let api_key_doc = match api_key_doc {
        Some(doc) => doc,
        None => return Response::error(DriverError::AuthError("Invalid API key".to_string())),
    };

    // Parse API key
    let api_key_data: crate::server::auth::ApiKey =
        match serde_json::from_value(api_key_doc.to_value()) {
            Ok(k) => k,
            Err(_) => {
                return Response::error(DriverError::AuthError(
                    "Invalid API key format".to_string(),
                ))
            }
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

    // Set authenticated state with API key name as identifier
    handler.authenticated_db = Some(database.to_string());
    Response::ok_empty()
}
