//! Per-collection opt-in for query-driven auto-indexes.
//!
//! Stored as `_stats:auto_index` (`1` / `0`). Missing key falls back to
//! `SOLIDB_AUTO_INDEX`. An explicit `0` always wins over the env var so
//! disable is a real opt-out.

use super::{Collection, AUTO_INDEX_META_KEY};
use crate::error::{DbError, DbResult};

fn env_auto_index() -> bool {
    match std::env::var("SOLIDB_AUTO_INDEX") {
        Ok(v) => {
            let t = v.trim();
            t == "1" || t.eq_ignore_ascii_case("true")
        }
        Err(_) => false,
    }
}

impl Collection {
    /// Explicit collection flag, else `SOLIDB_AUTO_INDEX`. `disable_auto_index`
    /// persists off and overrides the env var.
    pub fn auto_index_enabled(&self) -> bool {
        let stored = self.db.cf_handle(&self.name).and_then(|cf| {
            self.db
                .get_cf(&cf, AUTO_INDEX_META_KEY.as_bytes())
                .ok()
                .flatten()
        });
        match stored.as_deref() {
            Some(b"0") => false,
            Some(_) => true,
            None => env_auto_index(),
        }
    }

    pub fn enable_auto_index(&self) -> DbResult<()> {
        let cf = self
            .db
            .cf_handle(&self.name)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;
        self.db
            .put_cf(&cf, AUTO_INDEX_META_KEY.as_bytes(), b"1")
            .map_err(|e| DbError::InternalError(format!("enable_auto_index: {}", e)))?;
        Ok(())
    }

    pub fn disable_auto_index(&self) -> DbResult<()> {
        let cf = self
            .db
            .cf_handle(&self.name)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;
        self.db
            .put_cf(&cf, AUTO_INDEX_META_KEY.as_bytes(), b"0")
            .map_err(|e| DbError::InternalError(format!("disable_auto_index: {}", e)))?;
        Ok(())
    }
}
