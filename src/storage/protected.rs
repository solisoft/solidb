//! Collections that hold credentials, authorization state, or executable code.
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
//!
//! There are two tiers:
//!
//! * [`PROTECTED_COLLECTIONS`] — neither readable nor writable by name.
//!   Credentials (`_env`, `_admins`, `_api_keys`) and the authorization state
//!   the server reads back to decide permissions (`_roles`, `_user_roles`).
//! * [`WRITE_PROTECTED_COLLECTIONS`] — readable, but never *written* by name.
//!   Collections whose contents the server later executes or schedules:
//!   Lua scripts and the services that route to them, triggers, the job
//!   queue, the view/graph catalog, and the instance config. Listing them is
//!   a documented feature (the admin UI browses `_scripts`); writing them
//!   through the generic document API bypassed the Admin gate on the
//!   dedicated endpoints and got Lua running as `_system`.

use crate::error::{DbError, DbResult};

/// The boundary is every place a collection name arrives from outside: the
/// document API, SDBQL, the driver, import, truncate, blob uploads, and the
/// Lua bindings (`db:collection(name)`, `db:query`, `db:transaction`), which
/// write as the script's caller.
///
/// Collections never served through a caller-supplied collection name.
///
/// `_roles` and `_user_roles` are here because they *are* the authorization
/// decision: `AuthService::get_user_roles` trusts every row of `_user_roles`
/// for the username it matches, and `load_permissions_from_storage` prefers a
/// stored `_roles` definition over the built-in one. With only Write on
/// `_system`, inserting one document into either made the caller an admin.
pub const PROTECTED_COLLECTIONS: [&str; 5] =
    ["_env", "_admins", "_api_keys", "_roles", "_user_roles"];

/// Collections that a caller-supplied name may read but never write.
///
/// Each of these is *interpreted* by the server after it is stored: `_scripts`
/// holds Lua source that `/api/{db}/{service}/{path}` executes,
/// `_services` decides which scripts are routable, `_triggers` schedules
/// work that runs with the `_system` admin identity, `_views` and `_graphs`
/// are the query catalog (a `_views` row names the collection a `REFRESH`
/// truncates), and `_config` holds instance settings.
///
/// Writes must go through the dedicated endpoints, which are Admin-gated and
/// validate their input.
pub const WRITE_PROTECTED_COLLECTIONS: [&str; 7] = [
    "_scripts",
    "_services",
    "_triggers",
    "_views",
    "_graphs",
    "_config",
    "_rag_pipelines",
];

/// Collections a caller-supplied name may write **only with Admin**.
///
/// `_jobs` is interpreted by the server like the write-protected tier — the
/// trigger dispatcher executes its `status == 'pending'` rows as `_system` —
/// so a principal with plain Write must not reach it. But it is also the
/// Soli framework's job store: `perform_later` inserts into `_jobs` by name
/// and the worker updates rows in place, always with an admin credential.
/// An admin can already do anything the dispatcher can, so admitting admins
/// here gives away nothing, and denying everyone breaks every Soli app.
pub const ADMIN_WRITE_COLLECTIONS: [&str; 1] = ["_jobs"];

/// Who is asking to write a collection by name.
///
/// Every write path has to say, so that the tiers above are enforced in one
/// place — the storage getters — rather than re-derived in each handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteActor {
    /// A request from a client, HTTP or driver, carrying its authorization.
    Client { can_admin: bool },
    /// Server-side code acting on its own behalf (queue worker, replication,
    /// tests). Never use this for a name that came in over the wire.
    Server,
}

impl WriteActor {
    pub fn client(can_admin: bool) -> Self {
        Self::Client { can_admin }
    }
}

/// Extract the collection part of a possibly-qualified name.
///
/// Accepts both the bare name (`_env`) and the qualified column-family form
/// (`mydb:_env`), because SDBQL's `DOCUMENT()` builtin and the storage engine
/// both address collections as `{database}:{collection}`.
#[inline]
fn bare_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// True when `name` refers to a credential or authorization-state collection.
pub fn is_protected_collection(name: &str) -> bool {
    PROTECTED_COLLECTIONS.contains(&bare_name(name))
}

/// True when `name` refers to a collection that may be read but not written
/// through a caller-supplied name.
///
/// Does *not* include [`PROTECTED_COLLECTIONS`]; a write path must check both,
/// which is what [`is_write_denied_collection`] does.
pub fn is_write_protected_collection(name: &str) -> bool {
    WRITE_PROTECTED_COLLECTIONS.contains(&bare_name(name))
}

/// True when a caller-supplied `name` must not be written: either tier.
pub fn is_write_denied_collection(name: &str) -> bool {
    is_protected_collection(name) || is_write_protected_collection(name)
}

/// True when a caller-supplied `name` may be written by admins only.
pub fn is_admin_write_collection(name: &str) -> bool {
    ADMIN_WRITE_COLLECTIONS.contains(&bare_name(name))
}

/// The single decision every by-name write goes through.
pub fn check_write_access(name: &str, actor: WriteActor) -> DbResult<()> {
    if is_write_denied_collection(name) {
        return Err(write_denied_collection_error(name));
    }
    if is_admin_write_collection(name) && actor == (WriteActor::Client { can_admin: false }) {
        return Err(admin_write_collection_error(name));
    }
    Ok(())
}

/// The error returned when a non-admin writes an admin-only collection.
pub fn admin_write_collection_error(name: &str) -> DbError {
    DbError::Forbidden(format!(
        "Access denied: '{}' is executed by the server and is writable only \
         with an Admin credential",
        bare_name(name)
    ))
}

/// The error returned when a caller-supplied name resolves to a protected
/// collection. `Forbidden` rather than `CollectionNotFound`: these are
/// fixed, documented names, so acknowledging them leaks nothing, and a 403
/// tells an operator what happened.
pub fn protected_collection_error(name: &str) -> DbError {
    DbError::Forbidden(format!(
        "Access denied: '{}' stores credentials and is not readable or \
         writable through this API; use the admin-only endpoints",
        bare_name(name)
    ))
}

/// The error returned when a caller-supplied name resolves to a
/// write-protected collection.
pub fn write_protected_collection_error(name: &str) -> DbError {
    DbError::Forbidden(format!(
        "Access denied: '{}' is managed by the server and is not writable \
         through this API; use the dedicated admin endpoints",
        bare_name(name)
    ))
}

/// The error for a caller-supplied write to either tier, picking the message
/// that matches the tier the name actually falls in.
pub fn write_denied_collection_error(name: &str) -> DbError {
    if is_protected_collection(name) {
        protected_collection_error(name)
    } else {
        write_protected_collection_error(name)
    }
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
    fn authorization_state_is_protected() {
        // Writing either of these made an `editor` an admin.
        assert!(is_protected_collection("_roles"));
        assert!(is_protected_collection("_user_roles"));
        assert!(is_protected_collection("_system:_user_roles"));
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
        assert!(!is_write_denied_collection("users"));
        assert!(!is_protected_collection("_slow_queries"));
    }

    #[test]
    fn executable_collections_are_readable_but_not_writable() {
        for name in ["_scripts", "_services", "_triggers", "_views"] {
            assert!(
                !is_protected_collection(name),
                "{name} should stay readable"
            );
            assert!(is_write_protected_collection(name), "{name} write guard");
            assert!(is_write_denied_collection(name), "{name} write denied");
        }
        // Qualified form too, for the SDBQL paths.
        assert!(is_write_denied_collection("tenant:_scripts"));
    }

    #[test]
    fn credential_collections_are_also_write_denied() {
        assert!(is_write_denied_collection("_env"));
        assert!(is_write_denied_collection("_user_roles"));
    }

    #[test]
    fn near_misses_are_not_protected() {
        assert!(!is_protected_collection("_environment"));
        assert!(!is_protected_collection("my_env"));
        assert!(!is_protected_collection("_env2"));
        assert!(!is_write_denied_collection("_scripts_backup"));
        assert!(!is_write_denied_collection("my_jobs"));
    }

    #[test]
    fn jobs_is_writable_by_admins_only() {
        assert!(!is_write_denied_collection("_jobs"));
        assert!(is_admin_write_collection("_jobs"));
        assert!(is_admin_write_collection("app:_jobs"));
        assert!(!is_admin_write_collection("my_jobs"));
        assert!(check_write_access("_jobs", WriteActor::client(true)).is_ok());
        assert!(check_write_access("_jobs", WriteActor::Server).is_ok());
        let err = check_write_access("_jobs", WriteActor::client(false)).unwrap_err();
        assert!(err.to_string().contains("Admin"), "{err}");
        // The other tiers stay closed to admins too.
        assert!(check_write_access("_scripts", WriteActor::client(true)).is_err());
        assert!(check_write_access("_env", WriteActor::Server).is_err());
        assert!(check_write_access("users", WriteActor::client(false)).is_ok());
    }
}
