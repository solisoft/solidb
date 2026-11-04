use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use super::super::ast::BinaryOperator;

type ContextHasher = foldhash::fast::RandomState;

/// Execution context holding variable bindings.
///
/// Cloned and extended once per row, with a handful of short variable-name
/// keys: SipHash (std's default) cost ~5% of an uncached 50-row scan.
/// foldhash is seeded per process, so keys still do not hash predictably.
///
/// The values sit behind an `Arc`, so that per-row clone copies pointers, not
/// documents. With them inline, every row of a `FOR` deep-copied every
/// variable bound before it: a query whose earlier `LET`s held a few hundred
/// keys and pairs spent a second cloning them across a 2k-document scan,
/// where the same subquery alone took 15ms. A write to a shared value copies
/// it then (`Arc::make_mut`), never a read; and a write to a value only a
/// `Weak` also points at moves it to a new allocation. The subquery memo
/// relies on both: an unchanged address means an unchanged binding.
#[derive(Clone, Default)]
pub struct Context {
    vars: HashMap<String, Arc<Value>, ContextHasher>,
}

impl Context {
    pub fn with_capacity_and_hasher(capacity: usize, hasher: ContextHasher) -> Self {
        Self {
            vars: HashMap::with_capacity_and_hasher(capacity, hasher),
        }
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.vars.get(name).map(Arc::as_ref)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Value> {
        self.vars.get_mut(name).map(Arc::make_mut)
    }

    /// The binding itself, so a caller can compare it by address or bind the
    /// same allocation elsewhere without copying the value.
    pub fn get_shared(&self, name: &str) -> Option<&Arc<Value>> {
        self.vars.get(name)
    }

    /// The value for in-place editing only when this context is its sole
    /// owner (no other `Arc`, no `Weak`); `None` when it is shared, where
    /// [`Self::get_mut`] would copy it first. For code that takes values out
    /// of a row and would rather evaluate than pay for a whole-value copy.
    pub fn get_mut_unique(&mut self, name: &str) -> Option<&mut Value> {
        self.vars.get_mut(name).and_then(Arc::get_mut)
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.vars.contains_key(name)
    }

    pub fn insert(&mut self, name: String, value: Value) -> Option<Value> {
        self.vars.insert(name, Arc::new(value)).map(unshare)
    }

    /// As [`Self::insert`], binding an allocation another holder keeps (a
    /// memoised subquery's rows), so every row shares it instead of getting
    /// its own copy.
    pub fn insert_shared(&mut self, name: String, value: Arc<Value>) -> Option<Value> {
        self.vars.insert(name, value).map(unshare)
    }

    pub fn remove(&mut self, name: &str) -> Option<Value> {
        self.vars.remove(name).map(unshare)
    }

    pub fn len(&self) -> usize {
        self.vars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.vars.keys()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.vars.iter().map(|(name, value)| (name, value.as_ref()))
    }
}

/// The value out of its `Arc`, copied only when another context still holds it.
fn unshare(value: Arc<Value>) -> Value {
    Arc::try_unwrap(value).unwrap_or_else(|shared| (*shared).clone())
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl PartialEq for Context {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .all(|(name, value)| other.get(name) == Some(value))
    }
}

/// As the map of its bindings.
impl Serialize for Context {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.iter())
    }
}

impl Extend<(String, Value)> for Context {
    fn extend<I: IntoIterator<Item = (String, Value)>>(&mut self, iter: I) {
        for (name, value) in iter {
            self.insert(name, value);
        }
    }
}

impl FromIterator<(String, Value)> for Context {
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        let mut ctx = Self::default();
        ctx.extend(iter);
        ctx
    }
}

impl IntoIterator for Context {
    type Item = (String, Value);
    type IntoIter = std::iter::Map<
        std::collections::hash_map::IntoIter<String, Arc<Value>>,
        fn((String, Arc<Value>)) -> (String, Value),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.vars
            .into_iter()
            .map(|(name, value)| (name, unshare(value)))
    }
}

/// Statistics about mutation operations performed during query execution
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MutationStats {
    pub documents_inserted: usize,
    pub documents_updated: usize,
    pub documents_removed: usize,
}

impl MutationStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn total(&self) -> usize {
        self.documents_inserted + self.documents_updated + self.documents_removed
    }

    pub fn has_mutations(&self) -> bool {
        self.total() > 0
    }
}

/// Result of query execution including results and mutation statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryExecutionResult {
    pub results: Vec<Value>,
    pub mutations: MutationStats,
}

/// Bind variables for parameterized queries (prevents SDBQL injection)
pub type BindVars = HashMap<String, Value>;

/// Snapshot of the authenticated caller, used by CAN / CURRENT_USER / ROW_POLICY.
#[derive(Debug, Clone, Default)]
pub struct QueryPrincipal {
    pub user: String,
    pub roles: Vec<String>,
    pub can_read: bool,
    pub can_write: bool,
    pub can_admin: bool,
}

impl QueryPrincipal {
    /// The one mapping from role names to permissions, shared by the HTTP
    /// handlers (`principal_from_claims`) and the Lua bindings so a script
    /// evaluates a query exactly as its caller would through `/cursor`.
    pub fn from_roles(user: impl Into<String>, roles: Vec<String>) -> Self {
        let lower: Vec<String> = roles.iter().map(|r| r.to_ascii_lowercase()).collect();
        let can_admin = lower.iter().any(|r| r == "admin");
        let can_write = can_admin || lower.iter().any(|r| r == "editor" || r == "write");
        let can_read = can_write || lower.iter().any(|r| r == "viewer" || r == "read");
        Self {
            user: user.into(),
            roles,
            can_read,
            can_write,
            can_admin,
        }
    }

    /// A caller with no credential at all: reads nothing by role, writes
    /// nothing privileged.
    pub fn anonymous() -> Self {
        Self::from_roles(String::new(), Vec::new())
    }
}

/// Query execution plan with timing information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryExplain {
    /// Collections accessed by the query
    pub collections: Vec<CollectionAccess>,
    /// LET clause bindings
    pub let_bindings: Vec<LetBinding>,
    /// Filter conditions analyzed
    pub filters: Vec<FilterInfo>,
    /// Sort information
    pub sort: Option<SortInfo>,
    /// Limit information
    pub limit: Option<LimitInfo>,
    /// Execution timing for each step (in microseconds)
    pub timing: ExecutionTiming,
    /// Total documents scanned
    pub documents_scanned: usize,
    /// Total documents returned
    pub documents_returned: usize,
    /// Warnings or suggestions
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionAccess {
    pub name: String,
    pub variable: String,
    pub access_type: String, // "full_scan" or "index_lookup"
    pub index_used: Option<String>,
    pub index_type: Option<String>,
    pub documents_count: usize,
    /// FILTER field that would get `_auto_{field}` on this run (flag on, Write).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_index_candidate: Option<String>,
    /// Where this access sits when it belongs to a subquery: the clause that
    /// holds it (`LET owner`, `FILTER`, `RETURN`), nested ones joined by
    /// ` > `. Absent for the query's own loops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subquery: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LetBinding {
    pub variable: String,
    pub is_subquery: bool,
    pub time_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterInfo {
    pub expression: String,
    pub index_candidate: Option<String>,
    pub can_use_index: bool,
    pub documents_before: usize,
    pub documents_after: usize,
    pub time_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortInfo {
    pub field: String,
    pub direction: String,
    pub time_us: u64,
    /// The index the rows are read from in sorted order, when one serves the
    /// SORT (and LIMIT): the sort then costs nothing and reading stops early.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitInfo {
    pub offset: usize,
    /// Row count, or `null` for a standalone `OFFSET` (no upper bound)
    pub count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTiming {
    pub total_us: u64,
    pub let_clauses_us: u64,
    pub collection_scan_us: u64,
    pub filter_us: u64,
    pub sort_us: u64,
    pub limit_us: u64,
    pub return_projection_us: u64,
}

/// Extracted filter condition for index optimization
#[derive(Debug)]
pub struct IndexableCondition {
    pub field: String,
    pub op: BinaryOperator,
    pub value: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A row's context is a clone of the one before it: the clone must share
    /// the values, and a write through either side must not reach the other.
    #[test]
    fn a_cloned_context_shares_values_and_copies_on_write() {
        let mut outer = Context::default();
        outer.insert("ids".to_string(), json!(["a", "b"]));
        let mut row = outer.clone();
        assert!(Arc::ptr_eq(&outer.vars["ids"], &row.vars["ids"]));

        row.get_mut("ids")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push(json!("c"));
        row.insert("doc".to_string(), json!({"_key": "1"}));
        assert_eq!(outer.get("ids"), Some(&json!(["a", "b"])));
        assert_eq!(row.get("ids"), Some(&json!(["a", "b", "c"])));
        assert!(!outer.contains_key("doc"));

        assert_eq!(row.remove("ids"), Some(json!(["a", "b", "c"])));
        assert_eq!(outer.remove("ids"), Some(json!(["a", "b"])));
        assert!(outer.is_empty());
    }

    #[test]
    fn a_context_serializes_and_iterates_as_its_bindings() {
        let ctx: Context = [("a".to_string(), json!(1)), ("b".to_string(), json!("x"))]
            .into_iter()
            .collect();
        assert_eq!(
            serde_json::to_value(&ctx).unwrap(),
            json!({"a": 1, "b": "x"})
        );
        let mut pairs: Vec<(String, Value)> = ctx.clone().into_iter().collect();
        pairs.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(
            pairs,
            vec![("a".to_string(), json!(1)), ("b".to_string(), json!("x"))]
        );
        assert_eq!(ctx, ctx.clone());
    }
}
