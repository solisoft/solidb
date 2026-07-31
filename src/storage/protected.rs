//! Collections that hold credentials or authorization state.
//!
//! These live in the ordinary document store — `_env` is where the product
//! tells users to put provider API keys, `_admins` holds argon2 password
//! hashes, `_api_keys` holds API-key hashes — so without an explicit list
//! they are reachable through every generic read path: the document API,
//! SDBQL, and the driver protocol, all of which only require `Read`.
//!
//! Server-side code that legitimately needs these collections
//! (`server::llm_client`, `server::auth::AuthService`, `role_handlers`) goes
//! through the storage API directly and is deliberately unaffected — this
//! guard belongs at the boundaries where a *caller-supplied* collection name
//! is resolved.

use crate::error::DbError;

/// Collections never served through a caller-supplied collection name.
pub const PROTECTED_COLLECTIONS: [&str; 3] = ["_env", "_admins", "_api_keys"];

/// True when `name` refers to a credential collection.
///
/// Accepts both the bare name (`_env`) and the qualified column-family form
/// (`mydb:_env`), because SDBQL's `DOCUMENT()` builtin and the storage engine
/// both address collections as `{database}:{collection}`.
pub fn is_protected_collection(name: &str) -> bool {
    let bare = name.rsplit(':').next().unwrap_or(name);
    PROTECTED_COLLECTIONS.contains(&bare)
}

/// The error returned when a caller-supplied name resolves to a protected
/// collection. `Forbidden` rather than `CollectionNotFound`: these are
/// fixed, documented names, so acknowledging them leaks nothing, and a 403
/// tells an operator what happened.
pub fn protected_collection_error(name: &str) -> DbError {
    DbError::Forbidden(format!(
        "Access denied: '{}' stores credentials and is not readable or \
         writable through this API; use the admin-only endpoints",
        name.rsplit(':').next().unwrap_or(name)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_names_are_protected() {
        assert!(is_protected_collection("_env"));
        assert!(is_protected_collection("_admins"));
        assert!(is_protected_collection("_api_keys"));
    }

    #[test]
    fn qualified_names_are_protected() {
        assert!(is_protected_collection("mydb:_env"));
        assert!(is_protected_collection("_system:_admins"));
        assert!(is_protected_collection("victim:_api_keys"));
    }

    #[test]
    fn ordinary_collections_are_not() {
        assert!(!is_protected_collection("users"));
        assert!(!is_protected_collection("mydb:users"));
        // Other underscore collections stay reachable: they are internal
        // bookkeeping, not credentials, and queries against them are a
        // documented feature (e.g. _scripts, _slow_queries).
        assert!(!is_protected_collection("_scripts"));
        assert!(!is_protected_collection("_slow_queries"));
    }

    #[test]
    fn near_misses_are_not_protected() {
        assert!(!is_protected_collection("_environment"));
        assert!(!is_protected_collection("my_env"));
        assert!(!is_protected_collection("_env2"));
    }
}
