//! Per-database authorization middleware.
//!
//! `auth_middleware` (authentication) runs first and inserts `Claims` into
//! the request extensions; this layer then enforces that the principal is
//! actually allowed to touch the database named in the path. Routes without
//! a `{db}` path parameter pass through untouched (global endpoints do their
//! own `check_permission` calls in the handlers).
//!
//! Rollout switch: `SOLIDB_DB_AUTHZ_MODE=warn` logs would-be denials on the
//! `audit` target and lets requests through, so existing deployments can do
//! a dry-run release before flipping to the default `enforce`.

use axum::body::Body;
use axum::extract::{RawPathParams, State};
use axum::http::{Method, Request};
use axum::middleware::Next;
use axum::response::Response;

use crate::error::{DbError, DbResult};
use crate::server::auth::Claims;
use crate::server::authorization::{AuthorizationService, PermissionAction};
use crate::server::handlers::AppState;

/// How authorization failures are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthzMode {
    Enforce,
    Warn,
}

fn authz_mode() -> AuthzMode {
    static MODE: std::sync::OnceLock<AuthzMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("SOLIDB_DB_AUTHZ_MODE").as_deref() {
        Ok("warn") => {
            tracing::warn!(
                "SOLIDB_DB_AUTHZ_MODE=warn: per-database authorization failures are \
                     logged but NOT enforced"
            );
            AuthzMode::Warn
        }
        _ => AuthzMode::Enforce,
    })
}

/// Check `action` on `database` for `claims`, honoring the rollout mode.
/// Shared by this middleware and the handler-level checks (cursor
/// continuation, offline sync, mutating-query upgrade, WebSocket subscribe).
pub async fn enforce(
    claims: &Claims,
    state: &AppState,
    action: PermissionAction,
    database: Option<&str>,
) -> DbResult<()> {
    match AuthorizationService::check_permission(claims, state, action.clone(), database).await {
        Ok(()) => Ok(()),
        Err(e) => match authz_mode() {
            AuthzMode::Enforce => Err(e),
            AuthzMode::Warn => {
                tracing::warn!(
                    target: "audit",
                    user = %claims.sub,
                    database = database.unwrap_or("<global>"),
                    action = ?action,
                    "authz dry-run: request would be denied ({})",
                    e
                );
                Ok(())
            }
        },
    }
}

/// Synchronous variant of [`enforce`] for call sites that already resolved
/// the principal's permission set (e.g. per-item filtering loops). Returns
/// `true` if the action is allowed under the current rollout mode.
pub fn enforce_raw(
    permissions: &std::collections::HashSet<crate::server::authorization::Permission>,
    action: PermissionAction,
    database: Option<&str>,
    scoped_databases: Option<&[String]>,
    subject: &str,
) -> bool {
    match AuthorizationService::check_permission_raw(
        permissions,
        action.clone(),
        database,
        scoped_databases,
    ) {
        Ok(()) => true,
        Err(e) => match authz_mode() {
            AuthzMode::Enforce => false,
            AuthzMode::Warn => {
                tracing::warn!(
                    target: "audit",
                    user = subject,
                    database = database.unwrap_or("<global>"),
                    action = ?action,
                    "authz dry-run: request would be denied ({})",
                    e
                );
                true
            }
        },
    }
}

/// Map an HTTP request on a `{db}`-scoped route to the permission it needs.
///
/// Defaults: GET/HEAD/OPTIONS → Read, everything else → Write. Overrides:
/// - POST endpoints with read semantics (query execution, explain, search)
///   only need Read; the query handlers upgrade to Write themselves when the
///   parsed query contains mutating clauses.
/// - Irreversible bulk destruction (truncate, drop collection) and the Lua
///   REPL (arbitrary code execution) require Admin.
fn required_action(method: &Method, path: &str) -> PermissionAction {
    // Admin overrides
    if path.ends_with("/truncate") {
        return PermissionAction::Admin;
    }
    if path.ends_with("/repl") {
        return PermissionAction::Admin;
    }
    if *method == Method::DELETE && is_collection_drop(path) {
        return PermissionAction::Admin;
    }

    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return PermissionAction::Read;
    }

    // Read-semantics POST/PUT endpoints
    const READ_SUFFIXES: [&str; 9] = [
        "/cursor",
        "/explain",
        "/nl",
        "/near",
        "/within",
        "/search",
        "/aggregate",
        "/sql",
        "/_verify",
    ];
    if READ_SUFFIXES.iter().any(|s| path.ends_with(s)) {
        return PermissionAction::Read;
    }
    // Columnar read query: /_api/database/{db}/columnar/{collection}/query
    // and transactional query: .../transaction/{tx_id}/query
    if path.ends_with("/query") {
        return PermissionAction::Read;
    }

    PermissionAction::Write
}

/// `DELETE /_api/database/{db}/collection/{name}` and
/// `DELETE /_api/database/{db}/columnar/{collection}` drop a whole
/// collection; deeper paths (documents, indexes, schema, ...) do not.
fn is_collection_drop(path: &str) -> bool {
    let Some(rest) = path
        .strip_prefix("/_api/database/")
        .and_then(|r| r.split_once('/'))
        .map(|(_, rest)| rest)
    else {
        return false;
    };
    match rest.split_once('/') {
        Some(("collection", tail)) | Some(("columnar", tail)) => !tail.contains('/'),
        _ => false,
    }
}

/// Axum middleware enforcing per-database permissions on routes that carry a
/// `{db}` path parameter. Must run *after* `auth_middleware`.
pub async fn db_authz_middleware(
    State(state): State<AppState>,
    params: RawPathParams,
    req: Request<Body>,
    next: Next,
) -> Result<Response, DbError> {
    let db = params
        .iter()
        .find(|(k, _)| *k == "db")
        .map(|(_, v)| v.to_string());

    // Global routes (no {db} param) do their own permission checks.
    let Some(db) = db else {
        return Ok(next.run(req).await);
    };

    let claims = req
        .extensions()
        .get::<Claims>()
        .cloned()
        .ok_or_else(|| DbError::Forbidden("Missing authentication context".to_string()))?;

    let action = required_action(req.method(), req.uri().path());
    enforce(&claims, &state, action, Some(&db)).await?;

    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_required_action_defaults() {
        let get = Method::GET;
        let post = Method::POST;
        let put = Method::PUT;
        let delete = Method::DELETE;

        assert_eq!(
            required_action(&get, "/_api/database/db1/document/users/k1"),
            PermissionAction::Read
        );
        assert_eq!(
            required_action(&post, "/_api/database/db1/document/users"),
            PermissionAction::Write
        );
        assert_eq!(
            required_action(&delete, "/_api/database/db1/document/users/k1"),
            PermissionAction::Write
        );
        assert_eq!(
            required_action(&put, "/_api/database/db1/collection/users/properties"),
            PermissionAction::Write
        );
    }

    #[test]
    fn test_required_action_read_overrides() {
        let post = Method::POST;
        for path in [
            "/_api/database/db1/cursor",
            "/_api/database/db1/explain",
            "/_api/database/db1/nl",
            "/_api/database/db1/geo/places/location/near",
            "/_api/database/db1/geo/places/location/within",
            "/_api/database/db1/vector/docs/embeddings/search",
            "/_api/database/db1/hybrid/docs/search",
            "/_api/database/db1/columnar/metrics/aggregate",
            "/_api/database/db1/columnar/metrics/query",
            "/_api/database/db1/transaction/tx1/query",
            "/_api/database/db1/sql",
            "/_api/database/db1/document/users/_verify",
        ] {
            assert_eq!(
                required_action(&post, path),
                PermissionAction::Read,
                "expected Read for {}",
                path
            );
        }
    }

    #[test]
    fn test_required_action_admin_overrides() {
        let put = Method::PUT;
        let post = Method::POST;
        let delete = Method::DELETE;

        assert_eq!(
            required_action(&put, "/_api/database/db1/collection/users/truncate"),
            PermissionAction::Admin
        );
        assert_eq!(
            required_action(&delete, "/_api/database/db1/collection/users"),
            PermissionAction::Admin
        );
        assert_eq!(
            required_action(&delete, "/_api/database/db1/columnar/metrics"),
            PermissionAction::Admin
        );
        assert_eq!(
            required_action(&post, "/_api/database/db1/repl"),
            PermissionAction::Admin
        );

        // Deeper DELETE paths are plain writes, not drops
        assert_eq!(
            required_action(&delete, "/_api/database/db1/collection/users/schema"),
            PermissionAction::Write
        );
        assert_eq!(
            required_action(&delete, "/_api/database/db1/columnar/metrics/index/col1"),
            PermissionAction::Write
        );
    }
}
