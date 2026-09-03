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
        Ok("warn")
            if std::env::var("SOLIDB_DB_AUTHZ_ALLOW_WARN")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false) =>
        {
            tracing::warn!(
                "SOLIDB_DB_AUTHZ_MODE=warn: per-database authorization failures are \
                     logged but NOT enforced"
            );
            AuthzMode::Warn
        }
        Ok("warn") => {
            tracing::error!(
                "SOLIDB_DB_AUTHZ_MODE=warn is ignored without SOLIDB_DB_AUTHZ_ALLOW_WARN=1; \
                 enforcing authorization"
            );
            AuthzMode::Enforce
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
/// **`path` must be the matched route *template*** (`/_api/database/{db}/
/// document/{collection}/{key}`), never the concrete request path. Every rule
/// below keys off path structure, and on a concrete path the last segment is
/// usually caller-supplied — a document key, a collection name, an index name.
/// Classifying `PUT /_api/database/app/document/settings/query` by its
/// concrete path matched the `/query` read-suffix and let a read-only
/// principal overwrite the document named `query`; the same trick reached
/// `DELETE .../document/c/search` and `DELETE .../index/users/cursor`.
/// Templates contain only literals and `{param}` placeholders, so no request
/// input can reach these comparisons.
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
    if is_script_or_service_mutation(method, path) {
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
/// Creating or changing Lua services/scripts is equivalent to installing
/// server-side code; listing them stays a Read.
fn is_script_or_service_mutation(method: &Method, path: &str) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return false;
    }
    let Some(rest) = path
        .strip_prefix("/_api/database/")
        .and_then(|r| r.split_once('/'))
        .map(|(_, rest)| rest)
    else {
        return false;
    };
    rest == "scripts"
        || rest.starts_with("scripts/")
        || rest == "services"
        || rest.starts_with("services/")
}

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

/// Permission from the HTTP method alone: GET/HEAD/OPTIONS read, everything
/// else writes.
///
/// The fallback when the route template is unavailable. It deliberately keeps
/// none of the read-suffix overrides: those may only *downgrade* a request to
/// Read, and downgrading on evidence we could not verify is exactly the bug
/// this module now guards against.
fn method_default_action(method: &Method) -> PermissionAction {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        PermissionAction::Read
    } else {
        PermissionAction::Write
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

    // Classify on the matched route template, not the concrete path: the last
    // segment of a concrete path is caller-supplied on most routes, and the
    // read-suffix overrides below would otherwise be selected by a document
    // key or collection name (a key literally named `query` downgraded a PUT
    // to Read). `MatchedPath` is set by the router before any layer runs, so
    // the fallback is unreachable in practice; it degrades to method-only
    // defaults rather than trusting the URI.
    let action = match req.extensions().get::<axum::extract::MatchedPath>() {
        Some(matched) => required_action(req.method(), matched.as_str()),
        None => {
            tracing::warn!(
                target: "audit",
                path = %req.uri().path(),
                "authz: no matched route template; falling back to method defaults"
            );
            method_default_action(req.method())
        }
    };
    enforce(&claims, &state, action, Some(&db)).await?;

    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    // `required_action` is fed the matched route *template*, so every path in
    // these tests is written the way the router reports it.

    #[test]
    fn test_required_action_defaults() {
        let get = Method::GET;
        let post = Method::POST;
        let put = Method::PUT;
        let delete = Method::DELETE;

        assert_eq!(
            required_action(&get, "/_api/database/{db}/document/{collection}/{key}"),
            PermissionAction::Read
        );
        assert_eq!(
            required_action(&post, "/_api/database/{db}/document/{collection}"),
            PermissionAction::Write
        );
        assert_eq!(
            required_action(&delete, "/_api/database/{db}/document/{collection}/{key}"),
            PermissionAction::Write
        );
        assert_eq!(
            required_action(&put, "/_api/database/{db}/collection/{name}/properties"),
            PermissionAction::Write
        );
    }

    #[test]
    fn test_required_action_read_overrides() {
        let post = Method::POST;
        for path in [
            "/_api/database/{db}/cursor",
            "/_api/database/{db}/explain",
            "/_api/database/{db}/nl",
            "/_api/database/{db}/geo/{collection}/{field}/near",
            "/_api/database/{db}/geo/{collection}/{field}/within",
            "/_api/database/{db}/vector/{collection}/{index}/search",
            "/_api/database/{db}/hybrid/{collection}/search",
            "/_api/database/{db}/columnar/{collection}/aggregate",
            "/_api/database/{db}/columnar/{collection}/query",
            "/_api/database/{db}/transaction/{tx_id}/query",
            "/_api/database/{db}/sql",
            "/_api/database/{db}/document/{collection}/_verify",
        ] {
            assert_eq!(
                required_action(&post, path),
                PermissionAction::Read,
                "expected Read for {}",
                path
            );
        }
    }

    /// A document key, collection name or index name is the last segment of
    /// many mutating routes. Classifying on the concrete path let a key named
    /// after a read suffix downgrade the request: `PUT .../document/settings/
    /// query` matched `/query` and a read-only principal overwrote it.
    /// Templates have no such segment, so the same requests classify as Write.
    #[test]
    fn test_read_suffix_cannot_be_forged_by_a_document_key() {
        for (method, template) in [
            (
                Method::PUT,
                "/_api/database/{db}/document/{collection}/{key}",
            ),
            (
                Method::DELETE,
                "/_api/database/{db}/document/{collection}/{key}",
            ),
            (Method::POST, "/_api/database/{db}/document/{collection}"),
            (
                Method::DELETE,
                "/_api/database/{db}/index/{collection}/{index_name}",
            ),
        ] {
            assert_eq!(
                required_action(&method, template),
                PermissionAction::Write,
                "{} {} must stay a write",
                method,
                template
            );
        }
    }

    /// The fallback used when the router did not record a template never
    /// downgrades to Read on anything but a genuinely safe method.
    #[test]
    fn test_method_default_action_never_downgrades_writes() {
        assert_eq!(method_default_action(&Method::GET), PermissionAction::Read);
        assert_eq!(method_default_action(&Method::HEAD), PermissionAction::Read);
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert_eq!(
                method_default_action(&method),
                PermissionAction::Write,
                "{} must default to Write",
                method
            );
        }
    }

    #[test]
    fn test_required_action_admin_overrides() {
        let put = Method::PUT;
        let post = Method::POST;
        let delete = Method::DELETE;

        assert_eq!(
            required_action(&put, "/_api/database/{db}/collection/{name}/truncate"),
            PermissionAction::Admin
        );
        assert_eq!(
            required_action(&delete, "/_api/database/{db}/collection/{name}"),
            PermissionAction::Admin
        );
        assert_eq!(
            required_action(&delete, "/_api/database/{db}/columnar/{collection}"),
            PermissionAction::Admin
        );
        assert_eq!(
            required_action(&post, "/_api/database/{db}/repl"),
            PermissionAction::Admin
        );
        assert_eq!(
            required_action(&post, "/_api/database/{db}/scripts"),
            PermissionAction::Admin
        );
        assert_eq!(
            required_action(&put, "/_api/database/{db}/services/{key}"),
            PermissionAction::Admin
        );
        assert_eq!(
            required_action(&delete, "/_api/database/{db}/scripts/{script_id}"),
            PermissionAction::Admin
        );

        // Deeper DELETE paths are plain writes, not drops
        assert_eq!(
            required_action(&delete, "/_api/database/{db}/collection/{name}/schema"),
            PermissionAction::Write
        );
        assert_eq!(
            required_action(
                &delete,
                "/_api/database/{db}/columnar/{collection}/index/{column}"
            ),
            PermissionAction::Write
        );
    }
}
