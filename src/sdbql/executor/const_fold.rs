//! Plan-time constant folding (audit P8).
//!
//! Replaces variable-free subtrees made of literals, bind variables, operators
//! and *known-pure* builtins with `Expression::Literal`, so a
//! `FILTER doc.name == LOWER(@q)` or `doc.x IN [1, 2, 3]` computes its
//! right-hand side once per query instead of once per row. The folded literal
//! also lets the index optimizer (`index_opt.rs`) see through `CONCAT(@a, "%")`.
//!
//! Rules:
//! - Evaluation goes through the executor's own evaluator with an empty row
//!   context, so a folded value is exactly what the row path would compute.
//! - Only builtins on [`PURE_FUNCTIONS`] fold. It is an allowlist: anything
//!   non-deterministic (`RAND`, `UUID`, `NOW`/`DATE_NOW`), context-dependent
//!   (`CURRENT_USER`, `CAN`), collection-reading (`DOCUMENT`, `FULLTEXT`, …),
//!   side-effecting or unknown (user functions) is left alone.
//! - An error while folding leaves the subtree unfolded: the query then fails
//!   (or not) at run time exactly as before — a `false ? 1/0 : 2` or
//!   `FIRST(@maybe_missing)` in a branch that never runs must not fail the
//!   query at plan time.
//! - `DATE_NOW()`/`NOW()` are **not** folded. AQL evaluates `DATE_NOW()` on
//!   every call (its docs say to bind it with `LET` to get one timestamp), and
//!   SoliDB keeps that: `LET now = DATE_NOW()` before the `FOR` is the way to
//!   compute it once.
//! - Results heavier than [`MAX_FOLDED_NODES`] / [`MAX_FOLDED_BYTES`] are not
//!   inlined, and size-amplifying builtins (`RANGE`, `REPEAT`, `PAD_*`) are
//!   not folded at all, so a never-taken branch cannot allocate at plan time.
//! - A `Range` is folded only when it is small, and never as the top node of a
//!   `FOR` source (the streaming bulk-insert path recognises `FOR i IN a..b`).
//! - Window function calls are left untouched: their extraction keys are
//!   derived from the expression text.

use std::collections::HashMap;

use serde_json::Value;

use super::QueryExecutor;
use crate::sdbql::ast::*;

/// Cap on the JSON nodes of a folded literal.
const MAX_FOLDED_NODES: usize = 10_000;
/// Cap on the string bytes of a folded literal.
const MAX_FOLDED_BYTES: usize = 1 << 20;
/// A literal `a..b` range folds only up to this many elements.
const MAX_FOLDED_RANGE: i64 = 10_000;

/// Builtins that are deterministic functions of their arguments alone.
///
/// Anything missing from this list is simply not folded, so the list errs on
/// the side of omission. Do not add a function whose result depends on the
/// clock, randomness, the caller, the database, or that writes anything.
#[rustfmt::skip]
pub const PURE_FUNCTIONS: &[&str] = &[
    // math
    "FLOOR", "CEIL", "CEILING", "ROUND", "ABS", "SQRT", "POW", "POWER", "LOG", "LN", "LOG10",
    "LOG2", "EXP", "SIN", "COS", "TAN", "ASIN", "ACOS", "ATAN", "ATAN2", "DEGREES", "RADIANS",
    "PI", "E", "MOD", "BIT_AND", "BIT_OR", "BIT_XOR", "BIT_NEGATE", "BIT_NOT", "BIT_SHIFT_LEFT",
    "BIT_SHIFT_RIGHT", "CLAMP", "SUM", "MIN", "MAX", "AVG", "AVERAGE", "MEDIAN", "PERCENTILE",
    "QUANTILE", "VARIANCE", "VAR_POP", "VAR_SAMP", "STDDEV", "STDDEV_POP", "STDDEV_SAMP",
    "STDDEV_POPULATION", "COUNT_DISTINCT", "COUNT_UNIQUE", "UNIQUE_COUNT",
    // strings
    "UPPER", "TO_UPPER", "TOUPPER", "LOWER", "TO_LOWER", "TOLOWER", "TRIM", "LTRIM", "RTRIM",
    "CONCAT", "CONCAT_WS", "CONCAT_SEPARATOR", "JOIN", "CONTAINS", "STARTS_WITH", "ENDS_WITH",
    "SPLIT", "SUBSTRING", "SUBSTR", "REPLACE", "SUBSTITUTE", "LEFT", "RIGHT", "CHAR_LENGTH",
    "CHARACTER_LENGTH", "BYTE_LENGTH", "FIND_FIRST", "FIND", "FIND_LAST", "RFIND", "LIKE",
    "REGEX_TEST", "REGEX_REPLACE", "REGEX_MATCHES", "REGEX_SPLIT", "CAPITALIZE", "TITLE_CASE",
    "INITCAP", "WORD_COUNT", "TRUNCATE_TEXT", "MASK", "ENCODE_URI", "URL_ENCODE",
    "ENCODE_URI_COMPONENT", "DECODE_URI", "URL_DECODE", "DECODE_URI_COMPONENT", "SLUGIFY",
    "IS_EMAIL", "IS_URL", "IS_UUID", "IS_BLANK", "LEVENSHTEIN", "SIMILARITY", "FUZZY_MATCH",
    "SOUNDEX", "METAPHONE", "DOUBLE_METAPHONE", "COLOGNE", "COLOGNE_PHONETIC", "CAVERPHONE",
    "NYSIIS", "REVERSE",
    // arrays
    "FIRST", "LAST", "SORTED", "SORTED_DESC", "UNIQUE", "FLATTEN", "PUSH", "POP", "SLICE",
    "POSITION", "INDEX_OF", "NTH", "CONTAINS_ARRAY", "TAKE", "DROP", "CHUNK", "ZIP",
    "ZIP_OBJECT", "COUNT", "OUTERSECTION", "SYMDIFF", "LENGTH", "APPEND", "SHIFT", "UNSHIFT",
    "UNION", "UNION_DISTINCT", "INTERSECTION", "MINUS", "DIFFERENCE",
    // types and documents
    "IS_ARRAY", "IS_LIST", "IS_BOOL", "IS_BOOLEAN", "IS_NUMBER", "IS_NUMERIC", "IS_INTEGER",
    "IS_INT", "IS_STRING", "IS_NULL", "IS_OBJECT", "IS_DOCUMENT", "IS_EMPTY", "IS_DATE", "IS_KEY",
    "IS_SAME_COLLECTION", "TYPEOF", "TYPE_OF", "TYPENAME", "COALESCE", "NOT_NULL", "NULLIF",
    "TO_NUMBER", "TO_NUM", "TO_STRING", "TO_STR", "TO_BOOL", "TO_BOOLEAN", "TO_ARRAY", "TO_LIST",
    "IF", "ATTRIBUTES", "KEYS", "VALUES", "KEEP", "UNSET", "PARSE_IDENTIFIER",
    "PARSE_COLLECTION", "PARSE_KEY", "UNSET_RECURSIVE", "KEEP_RECURSIVE", "GET", "DEEP_MERGE",
    "ENTRIES", "FROM_ENTRIES", "HAS", "MERGE",
    // json / encoding / hashing
    "JSON_PARSE", "PARSE_JSON", "JSON_STRINGIFY", "TO_JSON", "JSON_STRINGIFY_PRETTY",
    "JSON_POINTER", "MD5", "SHA256", "SHA512", "BASE64_ENCODE", "TO_BASE64", "BASE64_DECODE",
    "FROM_BASE64", "HEX_ENCODE", "TO_HEX", "HEX_DECODE", "FROM_HEX", "HMAC_SHA256",
    // dates with an explicit timestamp argument (never NOW / DATE_NOW / HUMAN_TIME)
    "DATE_YEAR", "DATE_MONTH", "DATE_DAY", "DATE_HOUR", "DATE_MINUTE", "DATE_SECOND",
    "DATE_MILLISECOND", "DATE_DAYOFWEEK", "DATE_DAYOFYEAR", "DATE_WEEK", "DATE_ISOWEEK",
    "DATE_ISOWEEKYEAR", "DATE_QUARTER", "DATE_LEAPYEAR", "DATE_COMPARE", "DATE_ISO8601",
    "DATE_TIMESTAMP", "DATE_FORMAT", "DATE_TRUNC", "DATE_ROUND", "DATE_DAYS_IN_MONTH",
    "DATE_ADD", "DATE_SUBTRACT", "DATE_SUB", "DATE_DIFF", "TIME_BUCKET",
    // geometry (pure computations on their arguments)
    "DISTANCE", "GEO_DISTANCE", "GEO_EQUALS", "GEO_WITHIN", "GEO_POINT", "GEO_LINESTRING",
    "GEO_POLYGON", "GEO_MULTIPOINT", "GEO_MULTILINESTRING", "GEO_MULTIPOLYGON", "GEO_CONTAINS",
    "GEO_INTERSECTS", "GEO_IN_RANGE", "GEO_AREA",
    // vectors (pure maths on literal vectors)
    "VECTOR_SIMILARITY", "VECTOR_NORMALIZE", "VECTOR_DISTANCE",
];

/// True when `name` (case-insensitively) is on [`PURE_FUNCTIONS`] and takes
/// at least one argument where a zero-argument form would read the clock
/// (`DATE_*` with no timestamp is not allowed to fold).
pub fn is_pure_function(name: &str, arg_count: usize) -> bool {
    let upper = name.to_ascii_uppercase();
    if upper.starts_with("DATE_") && arg_count == 0 {
        return false;
    }
    PURE_FUNCTIONS.contains(&upper.as_str())
}

/// True when `v` is small enough to inline into the plan.
fn within_fold_budget(v: &Value) -> bool {
    fn walk(v: &Value, nodes: &mut usize, bytes: &mut usize) -> bool {
        *nodes += 1;
        if *nodes > MAX_FOLDED_NODES {
            return false;
        }
        match v {
            Value::String(s) => {
                *bytes += s.len();
                *bytes <= MAX_FOLDED_BYTES
            }
            Value::Array(a) => a.iter().all(|x| walk(x, nodes, bytes)),
            Value::Object(o) => o.iter().all(|(k, x)| {
                *bytes += k.len();
                *bytes <= MAX_FOLDED_BYTES && walk(x, nodes, bytes)
            }),
            _ => true,
        }
    }
    let (mut nodes, mut bytes) = (0, 0);
    walk(v, &mut nodes, &mut bytes)
}

fn literal_i64(e: &Expression) -> Option<i64> {
    match e {
        Expression::Literal(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f as i64)),
        _ => None,
    }
}

impl<'a> QueryExecutor<'a> {
    /// Fold constant subtrees of `query` in place (recursing into subqueries,
    /// CTEs and set-operation operands) using this executor's bind variables.
    /// Returns the number of subtrees replaced by a literal.
    ///
    /// Call it on a *clone* of a cached/parsed query: the folded tree bakes in
    /// the bind variables of this execution.
    pub fn fold_constants(&self, query: &mut Query) -> usize {
        let mut folder = Folder {
            exec: self,
            empty: HashMap::new(),
            folded: 0,
        };
        folder.query(query);
        folder.folded
    }
}

struct Folder<'e, 'a> {
    exec: &'e QueryExecutor<'a>,
    empty: super::types::Context,
    folded: usize,
}

impl Folder<'_, '_> {
    fn query(&mut self, q: &mut Query) {
        if let Some(with) = &mut q.with_clause {
            for cte in &mut with.ctes {
                self.query(&mut cte.query);
            }
        }
        for l in &mut q.let_clauses {
            self.expr(&mut l.expression);
        }
        for f in &mut q.for_clauses {
            self.for_clause(f);
        }
        for j in &mut q.join_clauses {
            self.join(j);
        }
        for f in &mut q.filter_clauses {
            self.expr(&mut f.expression);
        }
        if let Some(s) = &mut q.sort_clause {
            for (e, _) in &mut s.fields {
                self.expr(e);
            }
        }
        if let Some(l) = &mut q.limit_clause {
            self.expr(&mut l.offset);
            if let Some(c) = &mut l.count {
                self.expr(c);
            }
        }
        if let Some(r) = &mut q.return_clause {
            self.expr(&mut r.expression);
        }
        for l in &mut q.post_limit_lets {
            self.expr(&mut l.expression);
        }
        for clause in &mut q.body_clauses {
            self.body_clause(clause);
        }
        for op in &mut q.set_operations {
            self.query(&mut op.query);
        }
        // `create_materialized_view_clause` is deliberately skipped: its query
        // is a stored definition, not something this execution runs.
    }

    fn for_clause(&mut self, f: &mut ForClause) {
        if let Some(src) = &mut f.source_expression {
            match src {
                // Keep `FOR i IN a..b` recognisable; fold only the bounds.
                Expression::Range(a, b) => {
                    self.expr(a);
                    self.expr(b);
                }
                other => {
                    self.expr(other);
                }
            }
        }
        if let Some(st) = &mut f.system_time {
            self.expr(st);
        }
        match &mut f.valid_time {
            Some(ValidTimeSpec::AsOf(e)) => {
                self.expr(e);
            }
            Some(ValidTimeSpec::Range { from, to }) => {
                self.expr(from);
                self.expr(to);
            }
            None => {}
        }
    }

    fn join(&mut self, j: &mut JoinClause) {
        self.expr(&mut j.condition);
        if let Some(asof) = &mut j.asof {
            self.expr(&mut asof.left_time);
            self.expr(&mut asof.right_time);
            if let Some(t) = &mut asof.tolerance {
                self.expr(t);
            }
        }
    }

    fn body_clause(&mut self, clause: &mut BodyClause) {
        match clause {
            BodyClause::For(f) => self.for_clause(f),
            BodyClause::Let(l) => {
                self.expr(&mut l.expression);
            }
            BodyClause::Filter(f) | BodyClause::Search(f) => {
                self.expr(&mut f.expression);
            }
            BodyClause::Insert(i) => {
                self.expr(&mut i.document);
            }
            BodyClause::Update(u) => {
                self.expr(&mut u.selector);
                self.expr(&mut u.changes);
            }
            BodyClause::Upsert(u) => {
                self.expr(&mut u.search);
                self.expr(&mut u.insert);
                self.expr(&mut u.update);
            }
            BodyClause::Remove(r) => {
                self.expr(&mut r.selector);
            }
            BodyClause::Join(j) => self.join(j),
            BodyClause::GraphTraversal(g) => {
                self.expr(&mut g.start_vertex);
                if let Some(p) = &mut g.prune {
                    self.expr(p);
                }
            }
            BodyClause::ShortestPath(s) => {
                self.expr(&mut s.start_vertex);
                self.expr(&mut s.end_vertex);
            }
            BodyClause::Collect(c) => {
                for (_, e) in &mut c.group_vars {
                    self.expr(e);
                }
                for a in &mut c.aggregates {
                    if let Some(e) = &mut a.argument {
                        self.expr(e);
                    }
                }
            }
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }

    /// Fold `e` bottom-up. Returns true when `e` is (now) constant for the
    /// query — a literal or a bound bind variable.
    fn expr(&mut self, e: &mut Expression) -> bool {
        let exec = self.exec;
        let foldable = match e {
            Expression::Literal(_) => return true,
            // A bound bind variable is constant, but is not inlined on its
            // own: `IN @ids` is already served from the bind map.
            Expression::BindVariable(name) => return exec.bind_vars.contains_key(name.as_str()),
            Expression::Variable(_) => return false,

            Expression::FieldAccess(base, _)
            | Expression::OptionalFieldAccess(base, _)
            | Expression::ArraySpreadAccess(base, _) => self.expr(base),
            Expression::DynamicFieldAccess(a, b) | Expression::ArrayAccess(a, b) => {
                let ca = self.expr(a);
                let cb = self.expr(b);
                ca && cb
            }
            Expression::BinaryOp { left, op, right } => {
                let cl = self.expr(left);
                let cr = self.expr(right);
                cl && cr && !matches!(op, BinaryOperator::SemanticMatch)
            }
            Expression::UnaryOp { operand, .. } => self.expr(operand),
            Expression::Object(fields) => {
                let mut all = true;
                for (_, v) in fields.iter_mut() {
                    all &= self.expr(v);
                }
                all
            }
            Expression::Array(items) => {
                let mut all = true;
                for v in items.iter_mut() {
                    all &= self.expr(v);
                }
                all
            }
            Expression::Range(a, b) => {
                let ca = self.expr(a);
                let cb = self.expr(b);
                ca && cb
                    && match (literal_i64(a), literal_i64(b)) {
                        (Some(x), Some(y)) => y
                            .checked_sub(x)
                            .and_then(i64::checked_abs)
                            .is_some_and(|n| n <= MAX_FOLDED_RANGE),
                        _ => false,
                    }
            }
            Expression::FunctionCall { name, args } => {
                let mut all = true;
                for a in args.iter_mut() {
                    all &= self.expr(a);
                }
                all && is_pure_function(name, args.len())
            }
            Expression::Pipeline { left, right } => {
                let cl = self.expr(left);
                // Fold the right call's *arguments* only: the call itself is
                // missing its first argument (the piped value).
                match &mut **right {
                    Expression::FunctionCall { name, args } => {
                        let mut all = true;
                        for a in args.iter_mut() {
                            all &= self.expr(a);
                        }
                        cl && all && is_pure_function(name, args.len() + 1)
                    }
                    other => {
                        self.expr(other);
                        false
                    }
                }
            }
            Expression::Ternary {
                condition,
                true_expr,
                false_expr,
            } => {
                let c = self.expr(condition);
                let t = self.expr(true_expr);
                let f = self.expr(false_expr);
                c && t && f
            }
            Expression::Case {
                operand,
                when_clauses,
                else_clause,
            } => {
                let mut all = true;
                if let Some(o) = operand {
                    all &= self.expr(o);
                }
                for (w, t) in when_clauses.iter_mut() {
                    all &= self.expr(w);
                    all &= self.expr(t);
                }
                if let Some(x) = else_clause {
                    all &= self.expr(x);
                }
                all
            }
            Expression::TemplateString { parts } => {
                let mut all = true;
                for p in parts.iter_mut() {
                    if let TemplateStringPart::Expression(x) = p {
                        all &= self.expr(x);
                    }
                }
                all
            }
            Expression::Lambda { body, .. } => {
                // Parameters are Variables, so only parameter-free parts fold.
                self.expr(body);
                false
            }
            Expression::Subquery(q) => {
                self.query(q);
                false
            }
            // Window calls (and any variant added later) are left as written.
            #[allow(unreachable_patterns)]
            _ => false,
        };

        if !foldable {
            return false;
        }
        // A bare `[literal, ...]` of bind variables etc. is worth folding; a
        // node whose children are all literals already is too. Evaluate once.
        match exec.evaluate_expr_with_context(e, &self.empty) {
            Ok(v) if within_fold_budget(&v) => {
                *e = Expression::Literal(v);
                self.folded += 1;
                true
            }
            // Error: leave it for run time, where it may never be evaluated.
            // Too large: keep the expression as written.
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_excludes_nondeterministic_and_context_functions() {
        for f in [
            "RAND",
            "RANDOM",
            "RANDOM_INT",
            "NOW",
            "DATE_NOW",
            "DATE_NOW_ISO",
            "UUID",
            "UUIDV4",
            "UUID_V7",
            "NANOID",
            "ULID",
            "RANDOM_TOKEN",
            "SAMPLE",
            "DOCUMENT",
            "COLLECTION_COUNT",
            "FULLTEXT",
            "VECTOR_SEARCH",
            "HYBRID_SEARCH",
            "SEARCH_INDEX",
            "EMBED",
            "RERANK",
            "CURRENT_USER",
            "CURRENT_ROLES",
            "CAN",
            "ROW_POLICY",
            "APPLY",
            "CALL",
            "SLEEP",
            "ARGON2_HASH",
            "ARGON2_VERIFY",
            "HUMAN_TIME",
            "RANGE",
            "REPEAT",
            "NEIGHBORS",
            "PAGERANK",
            "BM25",
            "CREATE_VIEW",
            "DROP_GRAPH",
            "ASSERT",
        ] {
            assert!(!is_pure_function(f, 1), "{f} must not be foldable");
        }
    }

    #[test]
    fn date_functions_without_arguments_do_not_fold() {
        assert!(is_pure_function("DATE_YEAR", 1));
        assert!(!is_pure_function("DATE_YEAR", 0));
        assert!(is_pure_function("lower", 1));
    }

    #[test]
    fn fold_budget_rejects_large_values() {
        let big = Value::Array(
            (0..(MAX_FOLDED_NODES + 1))
                .map(|i| Value::from(i as u64))
                .collect(),
        );
        assert!(!within_fold_budget(&big));
        assert!(within_fold_budget(&serde_json::json!({"a": [1, 2, "x"]})));
    }
}
