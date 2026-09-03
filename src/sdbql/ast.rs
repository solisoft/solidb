use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A single Common Table Expression (CTE)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CteClause {
    /// CTE name (e.g., "temp" in "WITH temp AS (...)")
    pub name: String,
    /// Optional column names: WITH temp(col1, col2) AS (...)
    pub columns: Vec<String>,
    /// Whether this is a recursive CTE
    pub recursive: bool,
    /// The CTE body query
    pub query: Box<Query>,
}

/// WITH clause containing one or more CTEs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WithClause {
    pub ctes: Vec<CteClause>,
}

/// Set operation combining two query blocks: `a UNION b`, `a INTERSECT c`, ...
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SetOperator {
    /// UNION - concatenates and removes duplicates
    Union,
    /// UNION ALL - concatenates keeping duplicates
    UnionAll,
    /// INTERSECT - rows present in both sides, duplicates removed
    Intersect,
    /// EXCEPT - rows of the left side not present in the right side, duplicates removed
    Except,
}

/// One operand on the right-hand side of a set operation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetOperationClause {
    pub op: SetOperator,
    pub query: Box<Query>,
}

impl SetOperator {
    /// True for the `ALL` variants, which keep duplicate rows
    pub fn is_all(&self) -> bool {
        matches!(self, SetOperator::UnionAll)
    }
}

/// AST node for a complete SDBQL query
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Query {
    /// Optional WITH clause for CTEs (Common Table Expressions)
    pub with_clause: Option<WithClause>,
    /// LET clauses for variable bindings (executed first, before any FOR)
    pub let_clauses: Vec<LetClause>,
    /// Multiple FOR clauses for JOINs (nested loops)
    pub for_clauses: Vec<ForClause>,
    /// JOIN clauses for cross-collection queries
    pub join_clauses: Vec<JoinClause>,
    /// Multiple FILTER clauses (can reference any FOR variable)
    pub filter_clauses: Vec<FilterClause>,
    pub sort_clause: Option<SortClause>,
    pub limit_clause: Option<LimitClause>,
    /// RETURN clause is optional - queries with only mutations (INSERT/UPDATE/REMOVE) don't need it
    pub return_clause: Option<ReturnClause>,
    /// Optional CREATE STREAM clause (wraps the query definition)
    pub create_stream_clause: Option<CreateStreamClause>,
    /// Optional CREATE MATERIALIZED VIEW clause
    pub create_materialized_view_clause: Option<CreateMaterializedViewClause>,
    /// Optional REFRESH MATERIALIZED VIEW clause
    pub refresh_materialized_view_clause: Option<RefreshMaterializedViewClause>,
    /// Optional WINDOW clause for stream processing
    pub window_clause: Option<WindowClause>,

    /// Ordered body clauses (FOR, LET, FILTER) preserving declaration order
    /// This enables correlated subqueries where LET can reference outer FOR variables
    pub body_clauses: Vec<BodyClause>,

    /// Set operations applied after this query block: `q1 UNION q2 INTERSECT q3`
    /// is parsed as `q1` with `set_operations = [UNION q2, INTERSECT q3]`.
    #[serde(default)]
    pub set_operations: Vec<SetOperationClause>,
}

impl Query {
    /// True if executing this query writes data (INSERT/UPDATE/UPSERT/REMOVE
    /// clauses, stream or materialized-view DDL). Used to decide whether a
    /// principal needs Write rather than Read permission.
    pub fn has_mutations(&self) -> bool {
        if self.body_clauses.iter().any(|clause| {
            matches!(
                clause,
                BodyClause::Insert(_)
                    | BodyClause::Update(_)
                    | BodyClause::Upsert(_)
                    | BodyClause::Remove(_)
            )
        }) || self.create_stream_clause.is_some()
            || self.create_materialized_view_clause.is_some()
            || self.refresh_materialized_view_clause.is_some()
            || self
                .set_operations
                .iter()
                .any(|op| op.query.has_mutations())
            || self
                .with_clause
                .as_ref()
                .is_some_and(|with| with.ctes.iter().any(|cte| cte.query.has_mutations()))
            || self
                .create_materialized_view_clause
                .as_ref()
                .is_some_and(|c| c.query.has_mutations())
        {
            return true;
        }

        // A mutation can also hide inside an *expression*: a parenthesised
        // subquery (`RETURN (FOR e IN c INSERT {} INTO c)`) is executed by the
        // full body executor, and the catalog builtins below write `_views` /
        // `_graphs` directly. Neither appears in `body_clauses`, so a check
        // that only walked clauses classified them as reads and `/cursor`
        // never upgraded the caller to Write.
        self.expressions().any(expression_mutates)
    }

    /// Every expression this query block owns directly.
    ///
    /// Not recursive into nested `Query` values — [`expression_mutates`]
    /// handles that by calling [`Self::has_mutations`] on subqueries — but it
    /// must cover every field that can carry an `Expression`, or a mutation
    /// parked in the uncovered one is invisible to authorization.
    fn expressions(&self) -> impl Iterator<Item = &Expression> {
        let let_exprs = self.let_clauses.iter().map(|l| &l.expression);
        let for_exprs = self
            .for_clauses
            .iter()
            .flat_map(|f| f.source_expression.iter().chain(f.system_time.iter()));
        let valid_time = self
            .for_clauses
            .iter()
            .filter_map(|f| f.valid_time.as_ref());
        let filter_exprs = self.filter_clauses.iter().map(|f| &f.expression);
        let sort_exprs = self
            .sort_clause
            .iter()
            .flat_map(|s| s.fields.iter().map(|(e, _)| e));
        let limit_exprs = self
            .limit_clause
            .iter()
            .flat_map(|l| std::iter::once(&l.offset).chain(l.count.iter()));
        let return_expr = self.return_clause.iter().map(|r| &r.expression);
        let join_exprs = self.join_clauses.iter().flat_map(join_expressions);
        let body_exprs = self.body_clauses.iter().flat_map(body_clause_expressions);

        let_exprs
            .chain(for_exprs)
            .chain(valid_time.flat_map(valid_time_expressions))
            .chain(filter_exprs)
            .chain(sort_exprs)
            .chain(limit_exprs)
            .chain(return_expr)
            .chain(join_exprs)
            .chain(body_exprs)
    }
}

/// SDBQL functions that write server state rather than compute a value.
///
/// These take effect through `executor::catalog`, which edits the `_views` and
/// `_graphs` catalog collections. They are ordinary function calls, so no
/// clause-level check can see them: without this list a read-only principal
/// ran `RETURN DROP_GRAPH("prod")` or replaced a materialized view's
/// definition through `/cursor`, which is classified Read.
pub const MUTATING_FUNCTIONS: [&str; 4] =
    ["CREATE_VIEW", "DROP_VIEW", "CREATE_GRAPH", "DROP_GRAPH"];

/// True when `name` (case-insensitively) is a state-changing builtin.
pub fn is_mutating_function(name: &str) -> bool {
    MUTATING_FUNCTIONS
        .iter()
        .any(|f| name.eq_ignore_ascii_case(f))
}

fn valid_time_expressions(spec: &ValidTimeSpec) -> Vec<&Expression> {
    match spec {
        ValidTimeSpec::AsOf(e) => vec![e],
        ValidTimeSpec::Range { from, to } => vec![from, to],
    }
}

fn join_expressions(join: &JoinClause) -> Vec<&Expression> {
    let mut out = vec![&join.condition];
    if let Some(asof) = &join.asof {
        out.push(&asof.left_time);
        out.push(&asof.right_time);
        out.extend(asof.tolerance.iter());
    }
    out
}

fn body_clause_expressions(clause: &BodyClause) -> Vec<&Expression> {
    match clause {
        BodyClause::For(f) => f
            .source_expression
            .iter()
            .chain(f.system_time.iter())
            .chain(
                f.valid_time
                    .iter()
                    .flat_map(|v| valid_time_expressions(v).into_iter()),
            )
            .collect(),
        BodyClause::Let(l) => vec![&l.expression],
        BodyClause::Filter(f) | BodyClause::Search(f) => vec![&f.expression],
        BodyClause::Insert(i) => vec![&i.document],
        BodyClause::Update(u) => vec![&u.selector, &u.changes],
        BodyClause::Upsert(u) => vec![&u.search, &u.insert, &u.update],
        BodyClause::Remove(r) => vec![&r.selector],
        BodyClause::Join(j) => join_expressions(j),
        BodyClause::GraphTraversal(g) => std::iter::once(&g.start_vertex)
            .chain(g.prune.iter())
            .collect(),
        BodyClause::ShortestPath(s) => vec![&s.start_vertex, &s.end_vertex],
        BodyClause::Collect(c) => c
            .group_vars
            .iter()
            .map(|(_, e)| e)
            .chain(c.aggregates.iter().filter_map(|a| a.argument.as_ref()))
            .collect(),
        BodyClause::Window(_) => Vec::new(),
    }
}

/// True when evaluating `expr` can write: it contains a mutating subquery or
/// a state-changing builtin, at any depth.
pub fn expression_mutates(expr: &Expression) -> bool {
    match expr {
        Expression::Subquery(q) => q.has_mutations(),
        Expression::FunctionCall { name, args } => {
            is_mutating_function(name) || args.iter().any(expression_mutates)
        }
        Expression::WindowFunctionCall {
            function,
            arguments,
            over_clause,
        } => {
            is_mutating_function(function)
                || arguments.iter().any(expression_mutates)
                || over_clause.partition_by.iter().any(expression_mutates)
                || over_clause
                    .order_by
                    .iter()
                    .any(|(e, _)| expression_mutates(e))
        }
        Expression::FieldAccess(base, _)
        | Expression::OptionalFieldAccess(base, _)
        | Expression::ArraySpreadAccess(base, _) => expression_mutates(base),
        Expression::DynamicFieldAccess(a, b)
        | Expression::ArrayAccess(a, b)
        | Expression::Range(a, b)
        | Expression::Pipeline { left: a, right: b } => {
            expression_mutates(a) || expression_mutates(b)
        }
        Expression::BinaryOp { left, right, .. } => {
            expression_mutates(left) || expression_mutates(right)
        }
        Expression::UnaryOp { operand, .. } => expression_mutates(operand),
        Expression::Object(fields) => fields.iter().any(|(_, e)| expression_mutates(e)),
        Expression::Array(items) => items.iter().any(expression_mutates),
        Expression::Ternary {
            condition,
            true_expr,
            false_expr,
        } => {
            expression_mutates(condition)
                || expression_mutates(true_expr)
                || expression_mutates(false_expr)
        }
        Expression::Case {
            operand,
            when_clauses,
            else_clause,
        } => {
            operand.as_deref().is_some_and(expression_mutates)
                || when_clauses
                    .iter()
                    .any(|(c, r)| expression_mutates(c) || expression_mutates(r))
                || else_clause.as_deref().is_some_and(expression_mutates)
        }
        Expression::Lambda { body, .. } => expression_mutates(body),
        Expression::TemplateString { parts } => parts.iter().any(|p| match p {
            TemplateStringPart::Expression(e) => expression_mutates(e),
            TemplateStringPart::Literal(_) => false,
        }),
        Expression::Variable(_) | Expression::BindVariable(_) | Expression::Literal(_) => false,
    }
}

/// A clause that can appear in the query body (preserves order for correlated subqueries)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BodyClause {
    For(ForClause),
    Let(LetClause),
    Filter(FilterClause),
    Insert(InsertClause),
    Update(UpdateClause),
    Upsert(UpsertClause),
    Remove(RemoveClause),
    Join(JoinClause),
    GraphTraversal(GraphTraversalClause),
    ShortestPath(ShortestPathClause),
    Collect(CollectClause),
    Window(WindowClause),
    /// Scored filter (`SEARCH expr`); numeric scores are stored as `__search_score`.
    Search(FilterClause),
}

/// Edge direction for graph traversals
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EdgeDirection {
    /// Follow edges where start_vertex == _from
    Outbound,
    /// Follow edges where start_vertex == _to
    Inbound,
    /// Follow edges in both directions
    Any,
}

/// FOR vertex[, edge] IN [depth..depth] OUTBOUND|INBOUND|ANY start_vertex edge_collection
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphTraversalClause {
    /// Variable for the visited vertices
    pub vertex_var: String,
    /// Optional variable for the edges (can be omitted)
    pub edge_var: Option<String>,
    /// Direction of traversal
    pub direction: EdgeDirection,
    /// Starting vertex (expression like "users/alice" or @start)
    pub start_vertex: Expression,
    /// Edge collection to traverse
    pub edge_collection: String,
    /// Minimum traversal depth (default 1)
    pub min_depth: usize,
    /// Maximum traversal depth (default 1)
    pub max_depth: usize,
    /// Optional path variable `FOR v, e, p`
    #[serde(default)]
    pub path_var: Option<String>,
    /// Stop expanding when this expression is true
    #[serde(default)]
    pub prune: Option<Expression>,
}

/// FOR vertex[, edge] IN SHORTEST_PATH start_vertex TO end_vertex OUTBOUND|INBOUND|ANY edge_collection
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortestPathClause {
    /// Variable for the vertices in the path
    pub vertex_var: String,
    /// Optional variable for the edges in the path
    pub edge_var: Option<String>,
    /// Starting vertex
    pub start_vertex: Expression,
    /// Target vertex
    pub end_vertex: Expression,
    /// Direction of traversal
    pub direction: EdgeDirection,
    /// Edge collection to traverse
    pub edge_collection: String,
    /// Optional numeric edge field used as Dijkstra weight
    #[serde(default)]
    pub weight: Option<String>,
    #[serde(default)]
    pub path_var: Option<String>,
    #[serde(default)]
    pub mode: PathFindMode,
    #[serde(default)]
    pub k: Option<usize>,
    #[serde(default)]
    pub min_len: Option<usize>,
    #[serde(default)]
    pub max_len: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum PathFindMode {
    #[default]
    Shortest,
    AllShortest,
    KShortest,
    KPaths,
}

/// CREATE STREAM name AS ...
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateStreamClause {
    pub name: String,
    pub if_not_exists: bool,
}

/// CREATE MATERIALIZED VIEW name [REFRESH "<interval>"] AS ...
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateMaterializedViewClause {
    pub name: String,
    pub if_not_exists: bool,
    /// The query definition
    pub query: Box<Query>,
    /// Optional automatic refresh interval (e.g. "30s", "5m", "1h", "2d", or a
    /// plain number of seconds). When set, a background worker re-runs the view
    /// query on that cadence. `None` = manual `REFRESH MATERIALIZED VIEW` only.
    #[serde(default)]
    pub refresh_schedule: Option<String>,
}

/// REFRESH MATERIALIZED VIEW name
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefreshMaterializedViewClause {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WindowType {
    /// TUMBLING (SIZE "1m") - Fixed non-overlapping windows
    Tumbling,
    /// SLIDING (SIZE "1m") - Sliding windows (hopping)
    Sliding,
}

/// WINDOW TUMBLING (SIZE "1m")
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowClause {
    pub window_type: WindowType,
    /// Duration string (e.g., "1m", "30s", "1h")
    pub duration: String,
}

/// LET variable = expression (can be a subquery)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LetClause {
    pub variable: String,
    pub expression: Expression,
}

/// FOR variable IN collection/expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForClause {
    pub variable: String,
    pub collection: String,
    /// Optional: iterate over a variable (e.g., FOR x IN someLetVar)
    pub source_variable: Option<String>,
    /// Optional: iterate over an expression (e.g., FOR i IN 1..5)
    pub source_expression: Option<Expression>,
    /// `SYSTEM_TIME AS OF` timestamp expression (epoch ms or RFC3339)
    #[serde(default)]
    pub system_time: Option<Expression>,
    /// Application valid-time filter (`valid_from` / `valid_to` fields)
    #[serde(default)]
    pub valid_time: Option<ValidTimeSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ValidTimeSpec {
    AsOf(Expression),
    Range { from: Expression, to: Expression },
}

/// FILTER expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterClause {
    pub expression: Expression,
}

/// INSERT document INTO collection
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertClause {
    pub document: Expression,
    pub collection: String,
}

/// UPDATE document WITH changes IN collection
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateClause {
    /// The document or key to update (usually a variable like `doc` or `doc._key`)
    pub selector: Expression,
    /// The changes to apply (object expression)
    pub changes: Expression,
    /// The collection to update in
    pub collection: String,
}

/// UPSERT search INSERT insert UPDATE update IN collection
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpsertClause {
    pub search: Expression,
    pub insert: Expression,
    pub update: Expression,
    pub collection: String,
    pub replace: bool,
}

/// REMOVE document IN collection
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoveClause {
    /// The document or key to remove (usually a variable like `doc` or `doc._key`)
    pub selector: Expression,
    /// The collection to remove from
    pub collection: String,
}

/// JOIN type (INNER vs LEFT/RIGHT/FULL)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    FullOuter,
    Asof,
}

/// As-of join time alignment
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AsofStrategy {
    Backward,
    Forward,
    Nearest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsofSpec {
    pub left_time: Expression,
    pub right_time: Expression,
    pub strategy: AsofStrategy,
    pub tolerance: Option<Expression>,
}

/// JOIN variable IN collection ON condition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinClause {
    /// Type of join (INNER, LEFT, etc.)
    pub join_type: JoinType,
    /// Variable to bind joined documents to
    pub variable: String,
    /// Collection to join with
    pub collection: String,
    /// Join condition (e.g., user._key == orders.user_key)
    pub condition: Expression,
    #[serde(default)]
    pub asof: Option<AsofSpec>,
}

/// COLLECT var = expr [INTO group [KEEP var1, var2]] [WITH COUNT INTO count] [AGGREGATE ...]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectClause {
    /// Group variables: (variable_name, expression) pairs
    pub group_vars: Vec<(String, Expression)>,
    /// INTO variable (collects grouped documents into an array)
    pub into_var: Option<String>,
    /// Optional KEEP restriction on the variables stored in the INTO array.
    /// Empty = keep every variable currently in scope (default).
    #[serde(default)]
    pub keep_vars: Vec<String>,
    /// WITH COUNT INTO variable
    pub count_var: Option<String>,
    /// AGGREGATE expressions
    pub aggregates: Vec<AggregateExpr>,
}

/// Aggregate expression: var = FUNC(expr)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregateExpr {
    /// Variable to store the result
    pub variable: String,
    /// Aggregate function name (SUM, AVG, MIN, MAX, COUNT, LENGTH, etc.)
    pub function: String,
    /// Argument expression
    pub argument: Option<Expression>,
}

/// SORT expression [ASC|DESC]
/// Supports both field-based sorting (SORT doc.age) and function-based sorting (SORT BM25(doc.content, "query"))
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SortClause {
    pub fields: Vec<(Expression, bool)>, // (expression, ascending)
}

/// LIMIT [offset,] count -- or a standalone OFFSET, which has no count
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitClause {
    pub offset: Expression,
    /// Row count. `None` means "no upper bound" (`OFFSET n` without `LIMIT`):
    /// callers must not substitute a sentinel maximum, because the count is
    /// pushed down into storage scans and index lookups as an allocation hint.
    pub count: Option<Expression>,
}

/// RETURN [DISTINCT] expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReturnClause {
    pub expression: Expression,
    /// RETURN DISTINCT - remove duplicate result rows (first occurrence wins)
    #[serde(default)]
    pub distinct: bool,
}

/// Part of a template string (used in AST after parsing)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TemplateStringPart {
    /// Static text between interpolations
    Literal(String),
    /// Parsed expression inside ${...}
    Expression(Box<Expression>),
}

/// Expression types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expression {
    /// Variable reference (e.g., doc)
    Variable(String),

    /// Bind variable reference (e.g., @name) - for parameterized queries
    BindVariable(String),

    /// Field access (e.g., doc.name)
    FieldAccess(Box<Expression>, String),

    /// Optional field access (e.g., doc?.name) - returns null if base is null
    OptionalFieldAccess(Box<Expression>, String),

    /// Dynamic field access (e.g., doc[@fieldName] or doc["name"])
    DynamicFieldAccess(Box<Expression>, Box<Expression>),

    /// Array element access (e.g., arr[0], arr[i])
    ArrayAccess(Box<Expression>, Box<Expression>),

    /// Array spread access (e.g., arr[*].field extracts field from all elements)
    /// field_path is None for bare [*], Some("field.nested") for chained access
    ArraySpreadAccess(Box<Expression>, Option<String>),

    /// Literal value
    Literal(Value),

    /// Binary operation
    BinaryOp {
        left: Box<Expression>,
        op: BinaryOperator,
        right: Box<Expression>,
    },

    /// Unary operation
    UnaryOp {
        op: UnaryOperator,
        operand: Box<Expression>,
    },

    /// Object construction
    Object(Vec<(String, Expression)>),

    /// Array construction
    Array(Vec<Expression>),

    /// Range expression (e.g., 1..5 produces [1, 2, 3, 4, 5])
    Range(Box<Expression>, Box<Expression>),

    /// Function call (e.g., DISTANCE(lat1, lon1, lat2, lon2))
    FunctionCall { name: String, args: Vec<Expression> },

    /// Subquery (FOR ... RETURN ...) wrapped in parentheses
    Subquery(Box<Query>),

    /// Ternary conditional (condition ? true_expr : false_expr)
    Ternary {
        condition: Box<Expression>,
        true_expr: Box<Expression>,
        false_expr: Box<Expression>,
    },

    /// CASE expression - SQL-style conditional
    /// Simple form: CASE expr WHEN val1 THEN res1 WHEN val2 THEN res2 ELSE default END
    /// Searched form: CASE WHEN cond1 THEN res1 WHEN cond2 THEN res2 ELSE default END
    Case {
        /// Optional operand for simple CASE (None for searched CASE)
        operand: Option<Box<Expression>>,
        /// List of (condition/value, result) pairs
        when_clauses: Vec<(Expression, Expression)>,
        /// Optional ELSE result
        else_clause: Option<Box<Expression>>,
    },

    /// Pipeline operation (value |> FUNC(args))
    /// Left value becomes first argument to right-side function call
    Pipeline {
        left: Box<Expression>,
        right: Box<Expression>,
    },

    /// Lambda expression (x -> expr) or ((a, b) -> expr)
    /// Used as arguments to higher-order functions like FILTER, MAP
    Lambda {
        params: Vec<String>,
        body: Box<Expression>,
    },

    /// Window function call with OVER clause
    /// Example: ROW_NUMBER() OVER (PARTITION BY doc.region ORDER BY doc.amount DESC)
    WindowFunctionCall {
        function: String,
        arguments: Vec<Expression>,
        over_clause: WindowSpec,
    },

    /// Template string with interpolated expressions: $"Hello ${name}!"
    /// Syntax: $"text ${expression} more text"
    TemplateString { parts: Vec<TemplateStringPart> },
}

impl Expression {
    /// Call `f` on every direct sub-expression. Subquery bodies are not
    /// descended into — they are queries, not expressions.
    pub fn for_each_child(&self, f: &mut dyn FnMut(&Expression)) {
        match self {
            Expression::Variable(_)
            | Expression::BindVariable(_)
            | Expression::Literal(_)
            | Expression::Subquery(_) => {}
            Expression::FieldAccess(base, _)
            | Expression::OptionalFieldAccess(base, _)
            | Expression::ArraySpreadAccess(base, _) => f(base),
            Expression::DynamicFieldAccess(a, b)
            | Expression::ArrayAccess(a, b)
            | Expression::Range(a, b) => {
                f(a);
                f(b);
            }
            Expression::BinaryOp { left, right, .. } => {
                f(left);
                f(right);
            }
            Expression::UnaryOp { operand, .. } => f(operand),
            Expression::Object(fields) => fields.iter().for_each(|(_, e)| f(e)),
            Expression::Array(items) => items.iter().for_each(&mut *f),
            Expression::FunctionCall { args, .. } => args.iter().for_each(&mut *f),
            Expression::Ternary {
                condition,
                true_expr,
                false_expr,
            } => {
                f(condition);
                f(true_expr);
                f(false_expr);
            }
            Expression::Case {
                operand,
                when_clauses,
                else_clause,
            } => {
                if let Some(o) = operand {
                    f(o);
                }
                for (w, t) in when_clauses {
                    f(w);
                    f(t);
                }
                if let Some(e) = else_clause {
                    f(e);
                }
            }
            Expression::Pipeline { left, right } => {
                f(left);
                f(right);
            }
            Expression::Lambda { body, .. } => f(body),
            Expression::WindowFunctionCall {
                arguments,
                over_clause,
                ..
            } => {
                arguments.iter().for_each(&mut *f);
                over_clause.partition_by.iter().for_each(&mut *f);
                over_clause.order_by.iter().for_each(|(e, _)| f(e));
            }
            Expression::TemplateString { parts } => {
                for p in parts {
                    if let TemplateStringPart::Expression(e) = p {
                        f(e);
                    }
                }
            }
        }
    }
}

/// Window specification (the OVER clause)
/// Example: OVER (PARTITION BY doc.region ORDER BY doc.date ASC)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowSpec {
    /// PARTITION BY expressions (optional) - groups rows into partitions
    pub partition_by: Vec<Expression>,
    /// ORDER BY within the window (optional) - defines row ordering within each partition
    /// Each tuple is (expression, ascending)
    pub order_by: Vec<(Expression, bool)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BinaryOperator {
    // Comparison
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    /// Vector cosine distance, or three-way compare (−1/0/1).
    Spaceship,
    /// Semantic / trigram match (`a ~ b`). Unary `~` stays bitwise NOT.
    SemanticMatch,
    In,
    NotIn,

    // Logical
    And,
    Or,

    // Arithmetic
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulus, // Adding modulo strictly if needed, but standard request is just ops. Adding LIKE/REGEX first.
    Exponent, // For POW operator ^ or ** if we support it as operator

    // String matching
    Like,
    NotLike,
    RegEx,
    NotRegEx,
    FuzzyEqual, // ~= (fuzzy string matching)

    // Bitwise
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    LeftShift,
    RightShift,

    // Null coalescing
    NullCoalesce,

    // Logical OR (||) - returns left if truthy, otherwise right
    LogicalOr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UnaryOperator {
    Not,
    Negate,
    BitwiseNot,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_expression_literal() {
        let expr = Expression::Literal(json!(42));
        assert_eq!(expr, Expression::Literal(json!(42)));
    }

    #[test]
    fn test_expression_variable() {
        let expr = Expression::Variable("doc".to_string());
        if let Expression::Variable(name) = expr {
            assert_eq!(name, "doc");
        } else {
            panic!("Expected Variable");
        }
    }

    #[test]
    fn test_expression_field_access() {
        let expr = Expression::FieldAccess(
            Box::new(Expression::Variable("doc".to_string())),
            "name".to_string(),
        );

        if let Expression::FieldAccess(base, field) = expr {
            assert_eq!(*base, Expression::Variable("doc".to_string()));
            assert_eq!(field, "name");
        } else {
            panic!("Expected FieldAccess");
        }
    }

    #[test]
    fn test_expression_binary_op() {
        let expr = Expression::BinaryOp {
            left: Box::new(Expression::Variable("a".to_string())),
            op: BinaryOperator::Add,
            right: Box::new(Expression::Literal(json!(1))),
        };

        if let Expression::BinaryOp { left, op, right } = expr {
            assert_eq!(*left, Expression::Variable("a".to_string()));
            assert_eq!(op, BinaryOperator::Add);
            assert_eq!(*right, Expression::Literal(json!(1)));
        } else {
            panic!("Expected BinaryOp");
        }
    }

    #[test]
    fn test_for_clause() {
        let clause = ForClause {
            variable: "doc".to_string(),
            collection: "users".to_string(),
            source_variable: None,
            source_expression: None,
            system_time: None,
            valid_time: None,
        };

        assert_eq!(clause.variable, "doc");
        assert_eq!(clause.collection, "users");
    }

    #[test]
    fn test_filter_clause() {
        let clause = FilterClause {
            expression: Expression::Literal(json!(true)),
        };

        assert_eq!(clause.expression, Expression::Literal(json!(true)));
    }

    #[test]
    fn test_limit_clause() {
        let clause = LimitClause {
            offset: Expression::Literal(json!(0)),
            count: Some(Expression::Literal(json!(10))),
        };

        assert_eq!(clause.offset, Expression::Literal(json!(0)));
        assert_eq!(clause.count, Some(Expression::Literal(json!(10))));

        // A standalone OFFSET has no count at all
        let unbounded = LimitClause {
            offset: Expression::Literal(json!(5)),
            count: None,
        };
        assert!(unbounded.count.is_none());
    }

    #[test]
    fn test_sort_clause() {
        let clause = SortClause {
            fields: vec![(
                Expression::FieldAccess(
                    Box::new(Expression::Variable("doc".to_string())),
                    "age".to_string(),
                ),
                true,
            )],
        };

        assert_eq!(clause.fields.len(), 1);
        assert!(clause.fields[0].1); // ascending
    }

    #[test]
    fn test_let_clause() {
        let clause = LetClause {
            variable: "x".to_string(),
            expression: Expression::Literal(json!(42)),
        };

        assert_eq!(clause.variable, "x");
    }

    #[test]
    fn test_insert_clause() {
        let clause = InsertClause {
            document: Expression::Object(vec![]),
            collection: "users".to_string(),
        };

        assert_eq!(clause.collection, "users");
    }

    #[test]
    fn test_edge_direction() {
        assert_ne!(EdgeDirection::Inbound, EdgeDirection::Outbound);
        assert_ne!(EdgeDirection::Any, EdgeDirection::Inbound);
    }

    #[test]
    fn test_binary_operators() {
        assert_eq!(BinaryOperator::Equal.clone(), BinaryOperator::Equal);
        assert_ne!(BinaryOperator::Equal, BinaryOperator::NotEqual);
        assert_ne!(BinaryOperator::Add, BinaryOperator::Subtract);
    }

    #[test]
    fn test_unary_operators() {
        assert_eq!(UnaryOperator::Not.clone(), UnaryOperator::Not);
        assert_ne!(UnaryOperator::Not, UnaryOperator::Negate);
    }

    #[test]
    fn test_expression_clone() {
        let expr = Expression::Variable("test".to_string());
        let cloned = expr.clone();
        assert_eq!(expr, cloned);
    }

    #[test]
    fn test_query_default() {
        let query = Query {
            with_clause: None,
            let_clauses: vec![],
            for_clauses: vec![],
            join_clauses: vec![],
            filter_clauses: vec![],
            sort_clause: None,
            limit_clause: None,
            return_clause: None,
            create_stream_clause: None,
            create_materialized_view_clause: None,
            refresh_materialized_view_clause: None,
            window_clause: None,
            body_clauses: vec![],
            set_operations: vec![],
        };

        assert!(query.for_clauses.is_empty());
        assert!(query.return_clause.is_none());
    }

    #[test]
    fn test_collect_clause() {
        let clause = CollectClause {
            group_vars: vec![(
                "category".to_string(),
                Expression::FieldAccess(
                    Box::new(Expression::Variable("doc".to_string())),
                    "cat".to_string(),
                ),
            )],
            into_var: Some("items".to_string()),
            keep_vars: vec![],
            count_var: Some("cnt".to_string()),
            aggregates: vec![],
        };

        assert_eq!(clause.group_vars.len(), 1);
        assert_eq!(clause.into_var, Some("items".to_string()));
        assert_eq!(clause.count_var, Some("cnt".to_string()));
    }

    #[test]
    fn test_aggregate_expr() {
        let agg = AggregateExpr {
            variable: "total".to_string(),
            function: "SUM".to_string(),
            argument: Some(Expression::FieldAccess(
                Box::new(Expression::Variable("doc".to_string())),
                "price".to_string(),
            )),
        };

        assert_eq!(agg.variable, "total");
        assert_eq!(agg.function, "SUM");
        assert!(agg.argument.is_some());
    }

    #[test]
    fn test_expression_array() {
        let expr = Expression::Array(vec![
            Expression::Literal(json!(1)),
            Expression::Literal(json!(2)),
            Expression::Literal(json!(3)),
        ]);

        if let Expression::Array(items) = expr {
            assert_eq!(items.len(), 3);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_expression_object() {
        let expr = Expression::Object(vec![
            ("name".to_string(), Expression::Literal(json!("test"))),
            ("value".to_string(), Expression::Literal(json!(42))),
        ]);

        if let Expression::Object(fields) = expr {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].0, "name");
        } else {
            panic!("Expected Object");
        }
    }

    #[test]
    fn test_expression_range() {
        let expr = Expression::Range(
            Box::new(Expression::Literal(json!(1))),
            Box::new(Expression::Literal(json!(5))),
        );

        if let Expression::Range(start, end) = expr {
            assert_eq!(*start, Expression::Literal(json!(1)));
            assert_eq!(*end, Expression::Literal(json!(5)));
        } else {
            panic!("Expected Range");
        }
    }

    #[test]
    fn test_expression_function_call() {
        let expr = Expression::FunctionCall {
            name: "LENGTH".to_string(),
            args: vec![Expression::Variable("arr".to_string())],
        };

        if let Expression::FunctionCall { name, args } = expr {
            assert_eq!(name, "LENGTH");
            assert_eq!(args.len(), 1);
        } else {
            panic!("Expected FunctionCall");
        }
    }

    #[test]
    fn test_expression_ternary() {
        let expr = Expression::Ternary {
            condition: Box::new(Expression::Variable("flag".to_string())),
            true_expr: Box::new(Expression::Literal(json!(1))),
            false_expr: Box::new(Expression::Literal(json!(0))),
        };

        if let Expression::Ternary {
            condition,
            true_expr,
            false_expr,
        } = expr
        {
            assert_eq!(*condition, Expression::Variable("flag".to_string()));
            assert_eq!(*true_expr, Expression::Literal(json!(1)));
            assert_eq!(*false_expr, Expression::Literal(json!(0)));
        } else {
            panic!("Expected Ternary");
        }
    }
}
