use super::DriverHandler;
use crate::driver::protocol::{DriverError, Response};

/// Handle authentication
pub async fn handle_auth(
    handler: &mut DriverHandler,
    database: String,
    username: String,
    password: String,
) -> Response {
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

    // Verify requested database exists
    if let Err(e) = handler.storage.get_database(&database) {
        return Response::error(DriverError::DatabaseError(format!(
            "Database not found: {}",
            e
        )));
    }

    // Set authenticated state (admin users have access to all databases)
    handler.authenticated_db = Some(database);
    Response::ok_empty()
}
