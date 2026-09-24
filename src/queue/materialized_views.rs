//! Background refresh worker for scheduled materialized views.
//!
//! `CREATE MATERIALIZED VIEW name REFRESH "5m" AS <query>` records a
//! `refresh_schedule` in the per-database `_views` collection. This worker
//! re-runs the view query on that cadence so the view stays fresh without a
//! manual `REFRESH MATERIALIZED VIEW`.
//!
//! Model (v1): each node refreshes its own copy of the view on the interval,
//! recomputing from its local view of the source data. For replicated (non-
//! sharded) source data every node converges to the same result. Refresh is
//! deduplicated per-node via an in-memory next-due map (reset on restart, so a
//! refresh runs shortly after startup).

use super::QueueWorker;
use crate::error::DbError;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse a refresh interval into seconds. Accepts a plain integer (seconds) or a
/// suffixed duration: `s` seconds, `m` minutes, `h` hours, `d` days. Returns
/// `None` for an unparseable or non-positive interval.
pub(crate) fn parse_interval_secs(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_part, mult): (&str, u64) = match s.chars().last() {
        Some('s') | Some('S') => (&s[..s.len() - 1], 1),
        Some('m') | Some('M') => (&s[..s.len() - 1], 60),
        Some('h') | Some('H') => (&s[..s.len() - 1], 3600),
        Some('d') | Some('D') => (&s[..s.len() - 1], 86400),
        Some(c) if c.is_ascii_digit() => (s, 1),
        _ => return None,
    };
    let n: u64 = num_part.trim().parse().ok()?;
    let secs = n.checked_mul(mult)?;
    (secs > 0).then_some(secs)
}

/// Upper bound on one scheduled refresh. The inner query runs with the same
/// cooperative deadline `/cursor` gives a client query (audit H4: the
/// scheduled refresh used to run unbounded).
const MV_REFRESH_TIMEOUT_SECS: u64 = 30;

/// The principal a scheduled refresh runs as, from the `owner` recorded in
/// the `_views` row at creation time.
///
/// Audit H4: the refresh used to run with no principal at all, which the
/// executor treats as `WriteActor::Server` — so a definition written by a
/// plain Write user ran with the server's rights every interval. A row with
/// no recorded owner (created before this field existed, or by server-side
/// code) refreshes as a read-only, non-admin principal instead: it cannot
/// write a protected tier and row policies apply to it.
pub(crate) fn refresh_principal(view: &serde_json::Value) -> (crate::sdbql::QueryPrincipal, bool) {
    let owner = view.get("owner");
    let user = owner.and_then(|o| o.get("user")).and_then(|u| u.as_str());
    match user {
        Some(user) if !user.is_empty() => {
            let roles = owner
                .and_then(|o| o.get("roles"))
                .and_then(|r| r.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|r| r.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            (crate::sdbql::QueryPrincipal::from_roles(user, roles), true)
        }
        _ => (
            crate::sdbql::QueryPrincipal::from_roles(
                LEGACY_REFRESH_USER,
                vec!["viewer".to_string()],
            ),
            false,
        ),
    }
}

/// User name a view with no recorded owner is refreshed as. Not a real
/// account: it only matters to row policies written against CURRENT_USER.
const LEGACY_REFRESH_USER: &str = "_mv_refresh_legacy";

/// Views already warned about for lacking an owner, so the warning is logged
/// once per process rather than every interval.
static WARNED_NO_OWNER: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::OnceLock::new();

impl QueueWorker {
    /// One scheduled-materialized-view refresh sweep.
    pub(crate) async fn check_materialized_views(&self) {
        // Serialize with the other periodic scanners; skip if another worker holds it.
        let _lock = match self.claiming_lock.try_lock() {
            Ok(l) => l,
            Err(_) => return,
        };
        let now = now_secs();

        // Collect what is due first: the refreshes below are awaited, and a
        // storage scan must not be held across an await point.
        let mut due_views: Vec<(String, String, serde_json::Value)> = Vec::new();
        for db_name in self.storage.list_databases() {
            let views_coll_name = format!("{}:_views", db_name);
            let views_coll = match self.storage.get_collection(&views_coll_name) {
                Ok(c) => c,
                Err(_) => continue, // no views in this db
            };

            for doc in views_coll.scan(None) {
                let value = doc.to_value();
                if value.get("type").and_then(|t| t.as_str()) != Some("materialized") {
                    continue;
                }
                let interval = match value
                    .get("refresh_schedule")
                    .and_then(|s| s.as_str())
                    .and_then(parse_interval_secs)
                {
                    Some(i) => i,
                    None => continue, // manual-refresh view or bad interval
                };

                let view_key = doc.key.clone();
                let due_key = format!("{}:{}", db_name, view_key);

                // Per-node dedup: only refresh when due, then arm the next slot.
                {
                    let mut due = self.mv_next_due.lock().unwrap();
                    match due.get(&due_key) {
                        Some(&next) if now < next => continue,
                        _ => {
                            due.insert(due_key.clone(), now + interval);
                        }
                    }
                }
                due_views.push((db_name.clone(), view_key, value));
            }
        }

        for (db_name, view_key, value) in due_views {
            let (principal, has_owner) = refresh_principal(&value);
            if !has_owner {
                let due_key = format!("{}:{}", db_name, view_key);
                let first = WARNED_NO_OWNER
                    .get_or_init(Default::default)
                    .lock()
                    .map(|mut w| w.insert(due_key))
                    .unwrap_or(false);
                if first {
                    tracing::warn!(
                        "MV refresh worker: view '{}' in '{}' has no recorded owner; \
                         refreshing it as a read-only principal. Recreate the view to \
                         refresh it under its creator's permissions.",
                        view_key,
                        db_name
                    );
                }
            }

            match self.refresh_view(&db_name, &view_key, principal).await {
                Ok(()) => tracing::debug!(
                    "MV refresh worker: refreshed '{}' in '{}'",
                    view_key,
                    db_name
                ),
                Err(e) => tracing::warn!(
                    "MV refresh worker: failed to refresh '{}' in '{}': {}",
                    view_key,
                    db_name,
                    e
                ),
            }
        }
    }

    /// Refresh a single materialized view by running `REFRESH MATERIALIZED
    /// VIEW` as `principal`, on the blocking pool and under a deadline.
    async fn refresh_view(
        &self,
        db_name: &str,
        view_name: &str,
        principal: crate::sdbql::QueryPrincipal,
    ) -> Result<(), DbError> {
        // Backtick-quote so any name the parser accepted at creation parses
        // back to the same identifier.
        let sql = format!("REFRESH MATERIALIZED VIEW `{}`", view_name.replace('`', ""));
        let query = crate::sdbql::parser::parse(&sql)?;
        let storage = self.storage.clone();
        let db_name = db_name.to_string();
        tokio::task::spawn_blocking(move || {
            let executor = crate::sdbql::QueryExecutor::with_database(&storage, db_name)
                .with_principal(principal)
                .with_timeout(std::time::Duration::from_secs(MV_REFRESH_TIMEOUT_SECS));
            executor.execute(&query).map(|_| ())
        })
        .await
        .map_err(|e| DbError::InternalError(format!("MV refresh task failed: {}", e)))?
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_interval_secs, refresh_principal};
    use serde_json::json;

    #[test]
    fn test_refresh_principal_uses_recorded_owner() {
        let (p, has_owner) =
            refresh_principal(&json!({"owner": {"user": "alice", "roles": ["editor"]}}));
        assert!(has_owner);
        assert_eq!(p.user, "alice");
        assert!(p.can_write);
        assert!(!p.can_admin);
    }

    #[test]
    fn test_refresh_principal_legacy_row_is_read_only() {
        // Audit H4: no owner must never mean "run as the server".
        let (p, has_owner) = refresh_principal(&json!({"type": "materialized"}));
        assert!(!has_owner);
        assert!(p.can_read);
        assert!(!p.can_write);
        assert!(!p.can_admin);
        let (_, has_owner) = refresh_principal(&json!({"owner": {"user": ""}}));
        assert!(!has_owner);
    }

    #[test]
    fn test_parse_interval_secs() {
        assert_eq!(parse_interval_secs("30s"), Some(30));
        assert_eq!(parse_interval_secs("5m"), Some(300));
        assert_eq!(parse_interval_secs("1h"), Some(3600));
        assert_eq!(parse_interval_secs("2d"), Some(172800));
        assert_eq!(parse_interval_secs("45"), Some(45)); // plain seconds
        assert_eq!(parse_interval_secs("  10m  "), Some(600));
        assert_eq!(parse_interval_secs("0s"), None); // non-positive
        assert_eq!(parse_interval_secs(""), None);
        assert_eq!(parse_interval_secs("abc"), None);
        assert_eq!(parse_interval_secs("5x"), None); // unknown suffix
    }
}
