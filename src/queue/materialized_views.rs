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

impl QueueWorker {
    /// One scheduled-materialized-view refresh sweep.
    pub(crate) async fn check_materialized_views(&self) {
        // Serialize with the other periodic scanners; skip if another worker holds it.
        let _lock = match self.claiming_lock.try_lock() {
            Ok(l) => l,
            Err(_) => return,
        };
        let now = now_secs();

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

                match self.refresh_view(&db_name, &view_key) {
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
    }

    /// Refresh a single materialized view by running `REFRESH MATERIALIZED VIEW`.
    fn refresh_view(&self, db_name: &str, view_name: &str) -> Result<(), DbError> {
        let sql = format!("REFRESH MATERIALIZED VIEW {}", view_name);
        let query = crate::sdbql::parser::parse(&sql)?;
        let executor =
            crate::sdbql::QueryExecutor::with_database(&self.storage, db_name.to_string());
        executor.execute(&query)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::parse_interval_secs;

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
