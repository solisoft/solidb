use serde_json::Value;

/// AST node for a complete AQL query
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    /// LET clauses for variable bindings (executed first, before any FOR)
    pub let_clauses: Vec<LetClause>,
    /// Multiple FOR clauses for JOINs (nested loops)
    pub for_clauses: Vec<ForClause>,
    /// Multiple FILTER clauses (can reference any FOR variable)
    pub filter_clauses: Vec<FilterClause>,
    pub sort_clause: Option<SortClause>,
    pub limit_clause: Option<LimitClause>,
    /// RETURN clause is optional - queries with only mutations (INSERT/UPDATE/REMOVE) don't need it
    pub return_clause: Option<ReturnClause>,
    /// Ordered body clauses (FOR, LET, FILTER) preserving declaration order
    /// This enables correlated subqueries where LET can reference outer FOR variables
    pub body_clauses: Vec<BodyClause>,
}

/// A clause that can appear in the query body (preserves order for correlated subqueries)
#[derive(Debug, Clone, PartialEq)]
pub enum BodyClause {
    For(ForClause),
    Let(LetClause),
    Filter(FilterClause),
    Insert(InsertClause),
    Update(UpdateClause),
    Remove(RemoveClause),
}

/// LET variable = expression (can be a subquery)
#[derive(Debug, Clone, PartialEq)]
pub struct LetClause {
    pub variable: String,
    pub expression: Expression,
}

/// FOR variable IN collection/expression
#[derive(Debug, Clone, PartialEq)]
pub struct ForClause {
    pub variable: String,
    pub collection: String,
    /// Optional: iterate over a variable (e.g., FOR x IN someLetVar)
    pub source_variable: Option<String>,
    /// Optional: iterate over an expression (e.g., FOR i IN 1..5)
    pub source_expression: Option<Expression>,
}

/// FILTER expression
#[derive(Debug, Clone, PartialEq)]
pub struct FilterClause {
    pub expression: Expression,
}

/// INSERT document INTO collection
#[derive(Debug, Clone, PartialEq)]
pub struct InsertClause {
    pub document: Expression,
    pub collection: String,
}

/// UPDATE document WITH changes IN collection
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateClause {
    /// The document or key to update (usually a variable like `doc` or `doc._key`)
    pub selector: Expression,
    /// The changes to apply (object expression)
    pub changes: Expression,
    /// The collection to update in
    pub collection: String,
}

/// REMOVE document IN collection
#[derive(Debug, Clone, PartialEq)]
pub struct RemoveClause {
    /// The document or key to remove (usually a variable like `doc` or `doc._key`)
    pub selector: Expression,
    /// The collection to remove from
    pub collection: String,
}

/// SORT expression [ASC|DESC]
/// Supports both field-based sorting (SORT doc.age) and function-based sorting (SORT BM25(doc.content, "query"))
#[derive(Debug, Clone, PartialEq)]
pub struct SortClause {
    pub expression: Expression,
    pub ascending: bool,
}

/// LIMIT [offset,] count
#[derive(Debug, Clone, PartialEq)]
pub struct LimitClause {
    pub offset: usize,
    pub count: usize,
}

/// RETURN expression
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnClause {
    pub expression: Expression,
}

/// Expression types
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    /// Variable reference (e.g., doc)
    Variable(String),

    /// Bind variable reference (e.g., @name) - for parameterized queries
    BindVariable(String),

    /// Field access (e.g., doc.name)
    FieldAccess(Box<Expression>, String),

    /// Dynamic field access (e.g., doc[@fieldName] or doc["name"])
    DynamicFieldAccess(Box<Expression>, Box<Expression>),

    /// Array element access (e.g., arr[0], arr[i])
    ArrayAccess(Box<Expression>, Box<Expression>),

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
    FunctionCall {
        name: String,
        args: Vec<Expression>,
    },

    /// Subquery (FOR ... RETURN ...) wrapped in parentheses
    Subquery(Box<Query>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOperator {
    // Comparison
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,

    // Logical
    And,
    Or,

    // Arithmetic
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOperator {
    Not,
    Negate,
}
