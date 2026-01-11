use serde_json::Value;

/// AST node for a complete SDBQL query
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
    Upsert(UpsertClause),
    Remove(RemoveClause),
    GraphTraversal(GraphTraversalClause),
    ShortestPath(ShortestPathClause),
    Collect(CollectClause),
}

/// Edge direction for graph traversals
#[derive(Debug, Clone, PartialEq)]
pub enum EdgeDirection {
    /// Follow edges where start_vertex == _from
    Outbound,
    /// Follow edges where start_vertex == _to
    Inbound,
    /// Follow edges in both directions
    Any,
}

/// FOR vertex[, edge] IN [depth..depth] OUTBOUND|INBOUND|ANY start_vertex edge_collection
#[derive(Debug, Clone, PartialEq)]
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
}

/// FOR vertex[, edge] IN SHORTEST_PATH start_vertex TO end_vertex OUTBOUND|INBOUND|ANY edge_collection
#[derive(Debug, Clone, PartialEq)]
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

/// UPSERT search INSERT insert UPDATE update IN collection
#[derive(Debug, Clone, PartialEq)]
pub struct UpsertClause {
    pub search: Expression,
    pub insert: Expression,
    pub update: Expression,
    pub collection: String,
    pub replace: bool,
}

/// REMOVE document IN collection
#[derive(Debug, Clone, PartialEq)]
pub struct RemoveClause {
    /// The document or key to remove (usually a variable like `doc` or `doc._key`)
    pub selector: Expression,
    /// The collection to remove from
    pub collection: String,
}

/// COLLECT var = expr [INTO group] [WITH COUNT INTO count] [AGGREGATE ...]
#[derive(Debug, Clone, PartialEq)]
pub struct CollectClause {
    /// Group variables: (variable_name, expression) pairs
    pub group_vars: Vec<(String, Expression)>,
    /// INTO variable (collects grouped documents into an array)
    pub into_var: Option<String>,
    /// WITH COUNT INTO variable
    pub count_var: Option<String>,
    /// AGGREGATE expressions
    pub aggregates: Vec<AggregateExpr>,
}

/// Aggregate expression: var = FUNC(expr)
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
pub struct SortClause {
    pub fields: Vec<(Expression, bool)>, // (expression, ascending)
}

/// LIMIT [offset,] count
#[derive(Debug, Clone, PartialEq)]
pub struct LimitClause {
    pub offset: Expression,
    pub count: Expression,
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
}

/// Window specification (the OVER clause)
/// Example: OVER (PARTITION BY doc.region ORDER BY doc.date ASC)
#[derive(Debug, Clone, PartialEq)]
pub struct WindowSpec {
    /// PARTITION BY expressions (optional) - groups rows into partitions
    pub partition_by: Vec<Expression>,
    /// ORDER BY within the window (optional) - defines row ordering within each partition
    /// Each tuple is (expression, ascending)
    pub order_by: Vec<(Expression, bool)>,
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
    In,

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

    // Bitwise
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    LeftShift,
    RightShift,

    // Null coalescing
    NullCoalesce,
}

#[derive(Debug, Clone, PartialEq)]
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
            count: Expression::Literal(json!(10)),
        };

        assert_eq!(clause.offset, Expression::Literal(json!(0)));
        assert_eq!(clause.count, Expression::Literal(json!(10)));
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
            let_clauses: vec![],
            for_clauses: vec![],
            filter_clauses: vec![],
            sort_clause: None,
            limit_clause: None,
            return_clause: None,
            body_clauses: vec![],
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
