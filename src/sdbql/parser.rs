use serde_json::Value;

use super::ast::*;
use super::lexer::{Lexer, Token};
use crate::error::{DbError, DbResult};

pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
    allow_in_operator: bool,
}

impl Parser {
    pub fn new(input: &str) -> DbResult<Self> {
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize()?;

        Ok(Self {
            tokens,
            position: 0,
            allow_in_operator: true,
        })
    }

    fn current_token(&self) -> &Token {
        self.tokens.get(self.position).unwrap_or(&Token::Eof)
    }

    fn peek_token(&self, offset: usize) -> &Token {
        self.tokens
            .get(self.position + offset)
            .unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) {
        if self.position < self.tokens.len() {
            self.position += 1;
        }
    }

    fn expect(&mut self, expected: Token) -> DbResult<()> {
        if self.current_token() == &expected {
            self.advance();
            Ok(())
        } else {
            Err(DbError::ParseError(format!(
                "Expected {:?}, got {:?}",
                expected,
                self.current_token()
            )))
        }
    }

    pub fn parse(&mut self) -> DbResult<Query> {
        self.parse_query(true)
    }

    /// Parse a query, optionally checking for trailing tokens (false for subqueries)
    fn parse_query(&mut self, check_trailing: bool) -> DbResult<Query> {
        // Parse initial LET clauses (before any FOR - these are evaluated once)
        let mut let_clauses = Vec::new();
        while matches!(self.current_token(), Token::Let) {
            let_clauses.push(self.parse_let_clause()?);
        }

        // Parse body clauses (FOR, LET, FILTER) preserving order for correlated subqueries
        let mut body_clauses = Vec::new();
        let mut for_clauses = Vec::new();
        let mut filter_clauses = Vec::new();

        // Parse FOR, FILTER, INSERT, and additional LET clauses (they can be interleaved in SDBQL)
        loop {
            if matches!(self.current_token(), Token::For) {
                // Try to parse as graph traversal first, fallback to regular FOR
                match self.try_parse_for_or_graph()? {
                    ForOrGraph::For(for_clause) => {
                        for_clauses.push(for_clause.clone());
                        body_clauses.push(BodyClause::For(for_clause));
                    }
                    ForOrGraph::GraphTraversal(gt_clause) => {
                        body_clauses.push(BodyClause::GraphTraversal(gt_clause));
                    }
                    ForOrGraph::ShortestPath(sp_clause) => {
                        body_clauses.push(BodyClause::ShortestPath(sp_clause));
                    }
                }
            } else if matches!(self.current_token(), Token::Filter) {
                let filter_clause = self.parse_filter_clause()?;
                filter_clauses.push(filter_clause.clone());
                body_clauses.push(BodyClause::Filter(filter_clause));
            } else if matches!(self.current_token(), Token::Insert) {
                let insert_clause = self.parse_insert_clause()?;
                body_clauses.push(BodyClause::Insert(insert_clause));
            } else if matches!(self.current_token(), Token::Update) {
                let update_clause = self.parse_update_clause()?;
                body_clauses.push(BodyClause::Update(update_clause));
            } else if matches!(self.current_token(), Token::Remove) {
                let remove_clause = self.parse_remove_clause()?;
                body_clauses.push(BodyClause::Remove(remove_clause));
            } else if matches!(self.current_token(), Token::Upsert) {
                let upsert_clause = self.parse_upsert_clause()?;
                body_clauses.push(BodyClause::Upsert(upsert_clause));
            } else if matches!(self.current_token(), Token::Let) {
                let let_clause = self.parse_let_clause()?;
                // LET after FOR goes to body_clauses (correlated), not let_clauses
                body_clauses.push(BodyClause::Let(let_clause));
            } else if matches!(self.current_token(), Token::Collect) {
                let collect_clause = self.parse_collect_clause()?;
                body_clauses.push(BodyClause::Collect(collect_clause));
            } else {
                break;
            }
        }

        // FOR clause is now optional - allow simple RETURN queries
        // e.g., RETURN 1 + 1, RETURN MERGE({a: 1}, {b: 2})

        let sort_clause = if matches!(self.current_token(), Token::Sort) {
            Some(self.parse_sort_clause()?)
        } else {
            None
        };

        let limit_clause = if matches!(self.current_token(), Token::Limit) {
            Some(self.parse_limit_clause()?)
        } else {
            None
        };

        // RETURN clause is optional - mutations (INSERT/UPDATE/REMOVE) don't require it
        let return_clause = if matches!(self.current_token(), Token::Return) {
            Some(self.parse_return_clause()?)
        } else {
            None
        };

        // Only validate at top-level, not for subqueries
        if check_trailing {
            // Validate that we have a valid query structure
            // A query must have either:
            // 1. A RETURN clause (with or without FOR)
            // 2. A mutation (INSERT/UPDATE/REMOVE)
            // FOR without RETURN or mutation is invalid
            let has_mutation = body_clauses.iter().any(|c| {
                matches!(
                    c,
                    BodyClause::Insert(_) | BodyClause::Update(_) | BodyClause::Remove(_)
                )
            });

            if return_clause.is_none() && !has_mutation {
                // Check if there are unexpected tokens
                if !matches!(self.current_token(), Token::Eof) {
                    return Err(DbError::ParseError(format!(
                        "Unexpected token: {:?}. Expected FOR, LET, RETURN, INSERT, UPDATE, or REMOVE",
                        self.current_token()
                    )));
                }
                return Err(DbError::ParseError(
                    "Invalid query: missing RETURN clause or mutation (INSERT/UPDATE/REMOVE)"
                        .to_string(),
                ));
            }

            // Check for trailing tokens after a valid query
            if !matches!(self.current_token(), Token::Eof) {
                return Err(DbError::ParseError(format!(
                    "Unexpected token after query: {:?}",
                    self.current_token()
                )));
            }
        }

        Ok(Query {
            let_clauses,
            for_clauses,
            filter_clauses,
            sort_clause,
            limit_clause,
            return_clause,
            body_clauses,
        })
    }
}

/// Result of parsing a FOR clause - could be regular FOR or graph traversal
enum ForOrGraph {
    For(ForClause),
    GraphTraversal(GraphTraversalClause),
    ShortestPath(ShortestPathClause),
}

impl Parser {
    /// Try to parse FOR as either regular FOR or graph traversal
    /// Syntax detection:
    /// - Regular FOR: FOR v IN collection
    /// - Graph: FOR v[, e] IN [depth..depth] OUTBOUND|INBOUND|ANY start edge_coll
    /// - Shortest Path: FOR v[, e] IN SHORTEST_PATH start TO end OUTBOUND|... edge_coll
    fn try_parse_for_or_graph(&mut self) -> DbResult<ForOrGraph> {
        self.expect(Token::For)?;

        // Parse first variable
        let first_var = if let Token::Identifier(name) = self.current_token() {
            let var = name.clone();
            self.advance();
            var
        } else {
            return Err(DbError::ParseError(
                "Expected variable name after FOR".to_string(),
            ));
        };

        // Check for optional second variable (edge variable for graph traversals)
        let second_var = if matches!(self.current_token(), Token::Comma) {
            self.advance(); // consume comma
            if let Token::Identifier(name) = self.current_token() {
                let var = name.clone();
                self.advance();
                Some(var)
            } else {
                return Err(DbError::ParseError(
                    "Expected variable name after comma".to_string(),
                ));
            }
        } else {
            None
        };

        self.expect(Token::In)?;

        // Now detect what type of FOR this is
        // If we see SHORTEST_PATH, it's a shortest path query
        if matches!(self.current_token(), Token::ShortestPath) {
            let sp_clause = self.parse_shortest_path_clause(first_var, second_var)?;
            return Ok(ForOrGraph::ShortestPath(sp_clause));
        }

        // If we see a number (depth) or OUTBOUND/INBOUND/ANY, it's a graph traversal
        // If we see a number (depth) or OUTBOUND/INBOUND/ANY, it's a graph traversal
        let is_graph = if matches!(
            self.current_token(),
            Token::Outbound | Token::Inbound | Token::Any
        ) {
            true
        } else if matches!(self.current_token(), Token::Integer(_) | Token::Float(_)) {
            // Check if this is a graph traversal depth or just a range expression
            // Graph traversal: [min..max] OUTBOUND... or [depth] OUTBOUND...
            // Range expression: min..max ...

            // Look ahead to see if we find OUTBOUND/INBOUND/ANY
            if matches!(
                self.peek_token(1),
                Token::Outbound | Token::Inbound | Token::Any
            ) {
                // Case: 1 OUTBOUND ...
                true
            } else if matches!(self.peek_token(1), Token::DotDot) {
                // Case: 1..
                if matches!(self.peek_token(2), Token::Integer(_) | Token::Float(_)) {
                    // Case: 1..2
                    if matches!(
                        self.peek_token(3),
                        Token::Outbound | Token::Inbound | Token::Any
                    ) {
                        // Case: 1..2 OUTBOUND ...
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        if is_graph {
            let gt_clause = self.parse_graph_traversal_clause(first_var, second_var)?;
            return Ok(ForOrGraph::GraphTraversal(gt_clause));
        }

        // Otherwise it's a regular FOR clause
        // If we had a second variable, that's an error for regular FOR
        if second_var.is_some() {
            return Err(DbError::ParseError(
                "Second variable only allowed in graph traversals".to_string(),
            ));
        }

        // Check if the source is an identifier (collection/variable) or an expression
        if let Token::Identifier(name) = self.current_token() {
            let n = name.clone();
            self.advance();

            Ok(ForOrGraph::For(ForClause {
                variable: first_var,
                collection: n.clone(),
                source_variable: Some(n),
                source_expression: None,
            }))
        } else {
            // Parse as expression (e.g., 1..5, [1, 2, 3], etc.)
            let expr = self.parse_expression()?;
            Ok(ForOrGraph::For(ForClause {
                variable: first_var,
                collection: String::new(),
                source_variable: None,
                source_expression: Some(expr),
            }))
        }
    }
}

impl Parser {
    fn parse_let_clause(&mut self) -> DbResult<LetClause> {
        self.expect(Token::Let)?;

        let variable = if let Token::Identifier(name) = self.current_token() {
            let var = name.clone();
            self.advance();
            var
        } else {
            return Err(DbError::ParseError(
                "Expected variable name after LET".to_string(),
            ));
        };

        self.expect(Token::Assign)?;

        let expression = self.parse_expression()?;

        Ok(LetClause {
            variable,
            expression,
        })
    }

    fn parse_filter_clause(&mut self) -> DbResult<FilterClause> {
        self.expect(Token::Filter)?;
        let expression = self.parse_expression()?;
        Ok(FilterClause { expression })
    }

    fn parse_insert_clause(&mut self) -> DbResult<InsertClause> {
        self.expect(Token::Insert)?;
        let document = self.parse_expression()?;
        self.expect(Token::Into)?;

        let collection = if let Token::Identifier(name) = self.current_token() {
            let coll = name.clone();
            self.advance();
            coll
        } else {
            return Err(DbError::ParseError(
                "Expected collection name after INTO".to_string(),
            ));
        };

        Ok(InsertClause {
            document,
            collection,
        })
    }

    fn parse_update_clause(&mut self) -> DbResult<UpdateClause> {
        self.expect(Token::Update)?;

        // Parse the document selector (usually a variable like `doc` or `doc._key`)
        let selector = self.parse_expression()?;

        // Expect WITH keyword
        self.expect(Token::With)?;

        // Parse the changes (object expression)
        // Disable IN operator to avoid consuming the 'IN' keyword of the clause
        self.allow_in_operator = false;
        let changes_result = self.parse_expression();
        self.allow_in_operator = true;
        let changes = changes_result?;

        // Expect IN keyword
        self.expect(Token::In)?;

        // Parse collection name
        let collection = if let Token::Identifier(name) = self.current_token() {
            let coll = name.clone();
            self.advance();
            coll
        } else {
            return Err(DbError::ParseError(
                "Expected collection name after IN".to_string(),
            ));
        };

        Ok(UpdateClause {
            selector,
            changes,
            collection,
        })
    }

    fn parse_remove_clause(&mut self) -> DbResult<RemoveClause> {
        self.expect(Token::Remove)?;

        // Parse the document selector (usually a variable like `doc` or `doc._key`)
        // Disable IN operator to avoid consuming the 'IN' keyword of the clause
        self.allow_in_operator = false;
        let selector_result = self.parse_expression();
        self.allow_in_operator = true;
        let selector = selector_result?;

        // Expect IN keyword
        self.expect(Token::In)?;

        // Parse collection name
        let collection = if let Token::Identifier(name) = self.current_token() {
            let coll = name.clone();
            self.advance();
            coll
        } else {
            return Err(DbError::ParseError(
                "Expected collection name after IN".to_string(),
            ));
        };

        Ok(RemoveClause {
            selector,
            collection,
        })
    }

    fn parse_upsert_clause(&mut self) -> DbResult<UpsertClause> {
        self.expect(Token::Upsert)?;

        // Parse search expression
        // Disable IN operator to avoid consuming 'IN' keyword
        self.allow_in_operator = false;
        let search = self.parse_expression()?;
        self.allow_in_operator = true;

        self.expect(Token::Insert)?;

        self.allow_in_operator = false;
        let insert = self.parse_expression()?;
        self.allow_in_operator = true;

        // Expect UPDATE or REPLACE
        let replace = if matches!(self.current_token(), Token::Replace) {
            self.advance();
            true
        } else {
            self.expect(Token::Update)?;
            false
        };

        self.allow_in_operator = false;
        let update = self.parse_expression()?;
        self.allow_in_operator = true;

        self.expect(Token::In)?;

        let collection = if let Token::Identifier(name) = self.current_token() {
            let coll = name.clone();
            self.advance();
            coll
        } else {
            return Err(DbError::ParseError(
                "Expected collection name after IN".to_string(),
            ));
        };

        Ok(UpsertClause {
            search,
            insert,
            update,
            collection,
            replace,
        })
    }

    /// Parse COLLECT clause: COLLECT var = expr [, var = expr]* [INTO var] [WITH COUNT INTO var] [AGGREGATE var = FUNC(expr), ...]
    fn parse_collect_clause(&mut self) -> DbResult<CollectClause> {
        self.expect(Token::Collect)?;

        let mut group_vars = Vec::new();
        let mut into_var = None;
        let mut count_var = None;
        let mut aggregates = Vec::new();

        // Parse group variables: var = expr [, var = expr]*
        loop {
            // Check if we have a variable name followed by =
            // Need to peek ahead to not consume tokens meant for other clauses
            if let Token::Identifier(var_name) = self.current_token() {
                // Peek: check if next token is = (assignment)
                if let Some(next) = self.tokens.get(self.position + 1) {
                    if !matches!(next, Token::Assign) {
                        // Not a group variable assignment, stop parsing group vars
                        break;
                    }
                } else {
                    break;
                }

                let name = var_name.clone();
                self.advance(); // consume identifier
                self.advance(); // consume =

                // Parse the grouping expression
                let expr = self.parse_expression()?;
                group_vars.push((name, expr));

                // Check for comma for more group variables
                if matches!(self.current_token(), Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Parse optional INTO var
        if matches!(self.current_token(), Token::Into) {
            self.advance(); // consume INTO
            if let Token::Identifier(var_name) = self.current_token() {
                into_var = Some(var_name.clone());
                self.advance();
            } else {
                return Err(DbError::ParseError(
                    "Expected variable name after INTO".to_string(),
                ));
            }
        }

        // Parse optional WITH COUNT INTO var
        if matches!(self.current_token(), Token::With) {
            self.advance(); // consume WITH
            if !matches!(self.current_token(), Token::Count) {
                return Err(DbError::ParseError(
                    "Expected COUNT after WITH in COLLECT".to_string(),
                ));
            }
            self.advance(); // consume COUNT

            if !matches!(self.current_token(), Token::Into) {
                return Err(DbError::ParseError(
                    "Expected INTO after WITH COUNT".to_string(),
                ));
            }
            self.advance(); // consume INTO

            if let Token::Identifier(var_name) = self.current_token() {
                count_var = Some(var_name.clone());
                self.advance();
            } else {
                return Err(DbError::ParseError(
                    "Expected variable name after WITH COUNT INTO".to_string(),
                ));
            }
        }

        // Parse optional AGGREGATE var = FUNC(expr) [, ...]
        if matches!(self.current_token(), Token::Aggregate) {
            self.advance(); // consume AGGREGATE

            loop {
                // Parse var = FUNC(expr)
                if let Token::Identifier(var_name) = self.current_token() {
                    let var = var_name.clone();
                    self.advance();

                    self.expect(Token::Assign)?;

                    // Parse function call: FUNC(expr)
                    if let Token::Identifier(func_name) = self.current_token() {
                        let func = func_name.to_uppercase();
                        self.advance();

                        self.expect(Token::LeftParen)?;

                        // Parse optional argument
                        let arg = if matches!(self.current_token(), Token::RightParen) {
                            None
                        } else {
                            Some(self.parse_expression()?)
                        };

                        self.expect(Token::RightParen)?;

                        aggregates.push(AggregateExpr {
                            variable: var,
                            function: func,
                            argument: arg,
                        });
                    } else {
                        return Err(DbError::ParseError(
                            "Expected aggregate function name".to_string(),
                        ));
                    }

                    // Check for comma for more aggregates
                    if matches!(self.current_token(), Token::Comma) {
                        self.advance();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
        }

        Ok(CollectClause {
            group_vars,
            into_var,
            count_var,
            aggregates,
        })
    }

    /// Parse graph traversal: FOR v[, e] IN [min..max] OUTBOUND|INBOUND|ANY start_vertex edge_collection
    fn parse_graph_traversal_clause(
        &mut self,
        vertex_var: String,
        edge_var: Option<String>,
    ) -> DbResult<GraphTraversalClause> {
        // Already consumed FOR v[, e] IN

        // Parse optional depth range (e.g., 1..3)
        let (min_depth, max_depth) = if let Token::Integer(n) = self.current_token() {
            let min = *n as usize;
            self.advance();
            if matches!(self.current_token(), Token::DotDot) {
                self.advance();
                if let Token::Integer(m) = self.current_token() {
                    let max = *m as usize;
                    self.advance();
                    (min, max)
                } else {
                    return Err(DbError::ParseError(
                        "Expected integer after '..' in depth range".to_string(),
                    ));
                }
            } else {
                // Single depth value means min and max are the same
                (min, min)
            }
        } else {
            // Default depth is 1..1
            (1, 1)
        };

        // Parse direction (OUTBOUND, INBOUND, ANY)
        let direction = match self.current_token() {
            Token::Outbound => {
                self.advance();
                EdgeDirection::Outbound
            }
            Token::Inbound => {
                self.advance();
                EdgeDirection::Inbound
            }
            Token::Any => {
                self.advance();
                EdgeDirection::Any
            }
            _ => {
                return Err(DbError::ParseError(
                    "Expected OUTBOUND, INBOUND, or ANY after depth range".to_string(),
                ));
            }
        };

        // Parse start vertex (string literal or bind variable)
        let start_vertex = self.parse_expression()?;

        // Parse edge collection
        let edge_collection = if let Token::Identifier(name) = self.current_token() {
            let coll = name.clone();
            self.advance();
            coll
        } else {
            return Err(DbError::ParseError(
                "Expected edge collection name".to_string(),
            ));
        };

        Ok(GraphTraversalClause {
            vertex_var,
            edge_var,
            direction,
            start_vertex,
            edge_collection,
            min_depth,
            max_depth,
        })
    }

    /// Parse shortest path: FOR v[, e] IN SHORTEST_PATH start_vertex TO end_vertex OUTBOUND|INBOUND|ANY edge_collection
    fn parse_shortest_path_clause(
        &mut self,
        vertex_var: String,
        edge_var: Option<String>,
    ) -> DbResult<ShortestPathClause> {
        // Consume SHORTEST_PATH
        self.expect(Token::ShortestPath)?;

        // Parse start vertex
        let start_vertex = self.parse_expression()?;

        // Expect TO
        self.expect(Token::To)?;

        // Parse end vertex
        let end_vertex = self.parse_expression()?;

        // Parse direction (OUTBOUND, INBOUND, ANY)
        let direction = match self.current_token() {
            Token::Outbound => {
                self.advance();
                EdgeDirection::Outbound
            }
            Token::Inbound => {
                self.advance();
                EdgeDirection::Inbound
            }
            Token::Any => {
                self.advance();
                EdgeDirection::Any
            }
            _ => {
                return Err(DbError::ParseError(
                    "Expected OUTBOUND, INBOUND, or ANY after target vertex".to_string(),
                ));
            }
        };

        // Parse edge collection
        let edge_collection = if let Token::Identifier(name) = self.current_token() {
            let coll = name.clone();
            self.advance();
            coll
        } else {
            return Err(DbError::ParseError(
                "Expected edge collection name".to_string(),
            ));
        };

        Ok(ShortestPathClause {
            vertex_var,
            edge_var,
            start_vertex,
            end_vertex,
            direction,
            edge_collection,
        })
    }

    fn parse_sort_clause(&mut self) -> DbResult<SortClause> {
        self.expect(Token::Sort)?;

        let mut fields = Vec::new();

        loop {
            // Parse expression (could be field path, function call like BM25(...), etc.)
            let expression = self.parse_expression()?;

            let ascending = match self.current_token() {
                Token::Desc => {
                    self.advance();
                    false
                }
                Token::Asc => {
                    self.advance();
                    true
                }
                _ => true, // Default to ascending
            };

            fields.push((expression, ascending));

            if matches!(self.current_token(), Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }

        Ok(SortClause { fields })
    }

    fn parse_limit_clause(&mut self) -> DbResult<LimitClause> {
        self.expect(Token::Limit)?;

        let first = self.parse_expression()?;

        // Check for offset, count syntax
        if matches!(self.current_token(), Token::Comma) {
            self.advance();

            let count = self.parse_expression()?;

            Ok(LimitClause {
                offset: first,
                count,
            })
        } else {
            Ok(LimitClause {
                offset: Expression::Literal(Value::Number(serde_json::Number::from(0))),
                count: first,
            })
        }
    }

    fn parse_return_clause(&mut self) -> DbResult<ReturnClause> {
        self.expect(Token::Return)?;
        let expression = self.parse_expression()?;
        Ok(ReturnClause { expression })
    }

    fn parse_expression(&mut self) -> DbResult<Expression> {
        self.parse_ternary_expression()
    }

    /// Parse ternary expression: condition ? true_expr : false_expr
    /// Lowest precedence, right-associative
    fn parse_ternary_expression(&mut self) -> DbResult<Expression> {
        let condition = self.parse_pipeline_expression()?;

        if matches!(self.current_token(), Token::Question) {
            self.advance(); // consume '?'
            let true_expr = self.parse_ternary_expression()?; // right-associative
            self.expect(Token::Colon)?;
            let false_expr = self.parse_ternary_expression()?;
            Ok(Expression::Ternary {
                condition: Box::new(condition),
                true_expr: Box::new(true_expr),
                false_expr: Box::new(false_expr),
            })
        } else {
            Ok(condition)
        }
    }

    /// Parse pipeline expression: expr |> FUNC(args) |> FUNC2(args)
    /// Left-associative, precedence between ternary and OR
    fn parse_pipeline_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_or_expression()?;

        while matches!(self.current_token(), Token::PipeRight) {
            self.advance(); // consume |>

            // Parse function name - can be an identifier or a keyword that doubles as a function
            let func_name = self.parse_pipeline_function_name()?;

            // Expect opening paren
            self.expect(Token::LeftParen)?;

            // Parse arguments
            let mut args = Vec::new();
            while !matches!(self.current_token(), Token::RightParen | Token::Eof) {
                args.push(self.parse_expression()?);
                if matches!(self.current_token(), Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(Token::RightParen)?;

            let right = Expression::FunctionCall {
                name: func_name,
                args,
            };

            left = Expression::Pipeline {
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Parse function name in pipeline context - allows keywords that double as functions
    fn parse_pipeline_function_name(&mut self) -> DbResult<String> {
        let name = match self.current_token() {
            Token::Identifier(name) => name.clone(),
            // Keywords that can also be function names
            Token::Filter => "FILTER".to_string(),
            Token::Sort => "SORT".to_string(),
            Token::Count => "COUNT".to_string(),
            Token::Any => "ANY".to_string(),
            Token::Return => "RETURN".to_string(),
            Token::In => "IN".to_string(),
            _ => {
                return Err(DbError::ParseError(format!(
                    "Expected function name after |>, got {:?}",
                    self.current_token()
                )));
            }
        };
        self.advance();
        Ok(name)
    }

    /// Parse lambda expression: x -> expr or (a, b) -> expr
    fn parse_lambda_expression(&mut self) -> DbResult<Expression> {
        let mut params = Vec::new();

        // Handle parenthesized parameters: (a, b) -> expr
        if matches!(self.current_token(), Token::LeftParen) {
            self.advance(); // consume (
            while !matches!(self.current_token(), Token::RightParen | Token::Eof) {
                if let Token::Identifier(name) = self.current_token() {
                    params.push(name.clone());
                    self.advance();
                } else {
                    return Err(DbError::ParseError(
                        "Expected parameter name in lambda".to_string(),
                    ));
                }
                if matches!(self.current_token(), Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(Token::RightParen)?;
        } else if let Token::Identifier(name) = self.current_token() {
            // Single parameter without parentheses: x -> expr
            params.push(name.clone());
            self.advance();
        }

        self.expect(Token::Arrow)?;
        let body = self.parse_expression()?;

        Ok(Expression::Lambda {
            params,
            body: Box::new(body),
        })
    }

    /// Check if the current position looks like the start of a lambda: (params) ->
    fn is_lambda_params(&self) -> bool {
        // We're at '(' - scan ahead to see if it's (ident, ident, ...) ->
        let mut pos = self.position + 1;
        let mut depth = 1;

        while let Some(tok) = self.tokens.get(pos) {
            match tok {
                Token::LeftParen => depth += 1,
                Token::RightParen => {
                    depth -= 1;
                    if depth == 0 {
                        // Check if next token is Arrow
                        return matches!(self.tokens.get(pos + 1), Some(Token::Arrow));
                    }
                }
                Token::Comma | Token::Identifier(_) => {}
                _ => return false, // Not a simple param list
            }
            pos += 1;
        }
        false
    }

    fn parse_or_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_and_expression()?;

        while matches!(self.current_token(), Token::Or) {
            self.advance();
            let right = self.parse_and_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op: BinaryOperator::Or,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_and_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_bitwise_or_expression()?;

        while matches!(self.current_token(), Token::And) {
            self.advance();
            let right = self.parse_bitwise_or_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op: BinaryOperator::And,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_bitwise_or_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_bitwise_xor_expression()?;

        while matches!(self.current_token(), Token::Pipe) {
            self.advance();
            let right = self.parse_bitwise_xor_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op: BinaryOperator::BitwiseOr,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_bitwise_xor_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_bitwise_and_expression()?;

        while matches!(self.current_token(), Token::Caret) {
            self.advance();
            let right = self.parse_bitwise_and_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op: BinaryOperator::BitwiseXor,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_bitwise_and_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_comparison_expression()?;

        while matches!(self.current_token(), Token::Ampersand) {
            self.advance();
            let right = self.parse_comparison_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op: BinaryOperator::BitwiseAnd,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_comparison_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_range_expression()?;

        while let Some(op) = self.parse_comparison_operator()? {
            let right = self.parse_range_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_comparison_operator(&mut self) -> DbResult<Option<BinaryOperator>> {
        match self.current_token() {
            Token::Equal => {
                self.advance();
                Ok(Some(BinaryOperator::Equal))
            }
            Token::NotEqual => {
                self.advance();
                Ok(Some(BinaryOperator::NotEqual))
            }
            Token::LessThan => {
                self.advance();
                Ok(Some(BinaryOperator::LessThan))
            }
            Token::LessThanEq => {
                self.advance();
                Ok(Some(BinaryOperator::LessThanOrEqual))
            }
            Token::GreaterThan => {
                self.advance();
                Ok(Some(BinaryOperator::GreaterThan))
            }
            Token::GreaterThanEq => {
                self.advance();
                Ok(Some(BinaryOperator::GreaterThanOrEqual))
            }
            Token::In => {
                if self.allow_in_operator {
                    self.advance();
                    Ok(Some(BinaryOperator::In))
                } else {
                    Ok(None)
                }
            }
            Token::Like => {
                self.advance();
                Ok(Some(BinaryOperator::Like))
            }
            Token::RegEx => {
                self.advance();
                Ok(Some(BinaryOperator::RegEx))
            }
            Token::NotRegEx => {
                self.advance();
                Ok(Some(BinaryOperator::NotRegEx))
            }
            Token::Not => {
                // Check for NOT LIKE
                if matches!(self.peek_token(1), Token::Like) {
                    self.advance(); // consume NOT
                    self.advance(); // consume LIKE
                    Ok(Some(BinaryOperator::NotLike))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    /// Parse range expressions (e.g., 1..5 produces [1, 2, 3, 4, 5])
    fn parse_range_expression(&mut self) -> DbResult<Expression> {
        let left = self.parse_shift_expression()?;

        if matches!(self.current_token(), Token::DotDot) {
            self.advance(); // consume '..'
            let right = self.parse_shift_expression()?;
            Ok(Expression::Range(Box::new(left), Box::new(right)))
        } else {
            Ok(left)
        }
    }

    fn parse_shift_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_additive_expression()?;

        while matches!(self.current_token(), Token::LeftShift | Token::RightShift) {
            let op = match self.current_token() {
                Token::LeftShift => BinaryOperator::LeftShift,
                Token::RightShift => BinaryOperator::RightShift,
                _ => unreachable!(),
            };
            self.advance();
            let right = self.parse_additive_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_additive_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_multiplicative_expression()?;

        while matches!(self.current_token(), Token::Plus | Token::Minus) {
            let op = match self.current_token() {
                Token::Plus => BinaryOperator::Add,
                Token::Minus => BinaryOperator::Subtract,
                _ => unreachable!(),
            };
            self.advance();
            let right = self.parse_multiplicative_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_multiplicative_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_unary_expression()?;

        while matches!(
            self.current_token(),
            Token::Star | Token::Slash | Token::Percent
        ) {
            let op = match self.current_token() {
                Token::Star => BinaryOperator::Multiply,
                Token::Slash => BinaryOperator::Divide,
                Token::Percent => BinaryOperator::Modulus,
                _ => unreachable!(),
            };
            self.advance();
            let right = self.parse_unary_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_unary_expression(&mut self) -> DbResult<Expression> {
        match self.current_token() {
            Token::Not => {
                self.advance();
                let operand = self.parse_unary_expression()?;
                Ok(Expression::UnaryOp {
                    op: UnaryOperator::Not,
                    operand: Box::new(operand),
                })
            }
            Token::Minus => {
                self.advance();
                let operand = self.parse_unary_expression()?;
                Ok(Expression::UnaryOp {
                    op: UnaryOperator::Negate,
                    operand: Box::new(operand),
                })
            }
            Token::Tilde => {
                self.advance();
                let operand = self.parse_unary_expression()?;
                Ok(Expression::UnaryOp {
                    op: UnaryOperator::BitwiseNot,
                    operand: Box::new(operand),
                })
            }
            _ => self.parse_postfix_expression(),
        }
    }

    fn parse_postfix_expression(&mut self) -> DbResult<Expression> {
        let mut expr = self.parse_primary_expression()?;

        // Handle field access and array indexing
        loop {
            match self.current_token() {
                Token::Dot => {
                    self.advance();
                    if let Token::Identifier(field) = self.current_token() {
                        let field_name = field.clone();
                        self.advance();
                        expr = Expression::FieldAccess(Box::new(expr), field_name);
                    } else {
                        return Err(DbError::ParseError(
                            "Expected field name after '.'".to_string(),
                        ));
                    }
                }
                Token::LeftBracket => {
                    self.advance();

                    // Check for [*] array spread syntax
                    if matches!(self.current_token(), Token::Star) {
                        self.advance(); // consume '*'
                        self.expect(Token::RightBracket)?;

                        // Collect subsequent dot-separated field path
                        let field_path = if matches!(self.current_token(), Token::Dot) {
                            let mut path = String::new();
                            while matches!(self.current_token(), Token::Dot) {
                                self.advance();
                                if let Token::Identifier(name) = self.current_token() {
                                    let name = name.clone();
                                    if !path.is_empty() {
                                        path.push('.');
                                    }
                                    path.push_str(&name);
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                            if path.is_empty() {
                                None
                            } else {
                                Some(path)
                            }
                        } else {
                            None
                        };

                        expr = Expression::ArraySpreadAccess(Box::new(expr), field_path);
                        continue; // Continue loop for chained [*]
                    }

                    let index_expr = self.parse_expression()?;
                    self.expect(Token::RightBracket)?;

                    // Handle different index types:
                    // - Number literal: array index (arr[0])
                    // - String literal: static field access (doc["name"])
                    // - Bind variable or other expression: dynamic field/array access
                    match &index_expr {
                        Expression::Literal(Value::Number(_)) => {
                            // Numeric index: array access
                            expr = Expression::ArrayAccess(Box::new(expr), Box::new(index_expr));
                        }
                        Expression::Literal(Value::String(s)) => {
                            // String literal: static field access
                            expr = Expression::FieldAccess(Box::new(expr), s.clone());
                        }
                        _ => {
                            // Dynamic field access: doc[@field], doc[someVar], etc.
                            expr = Expression::DynamicFieldAccess(
                                Box::new(expr),
                                Box::new(index_expr),
                            );
                        }
                    }
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    fn parse_primary_expression(&mut self) -> DbResult<Expression> {
        match self.current_token() {
            Token::Identifier(name) => {
                // Check for lambda: x -> expr
                if matches!(self.peek_token(1), Token::Arrow) {
                    return self.parse_lambda_expression();
                }

                let name = name.clone();
                self.advance();

                // Check if this is a function call
                if matches!(self.current_token(), Token::LeftParen) {
                    self.advance(); // consume '('
                    let mut args = Vec::new();

                    // Parse arguments
                    while !matches!(self.current_token(), Token::RightParen | Token::Eof) {
                        args.push(self.parse_expression()?);

                        if matches!(self.current_token(), Token::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }

                    self.expect(Token::RightParen)?;

                    Ok(Expression::FunctionCall { name, args })
                } else {
                    Ok(Expression::Variable(name))
                }
            }

            Token::Integer(n) => {
                let num = *n;
                self.advance();
                Ok(Expression::Literal(Value::Number(
                    serde_json::Number::from(num),
                )))
            }

            Token::Float(f) => {
                let num = *f;
                self.advance();
                Ok(Expression::Literal(Value::Number(
                    serde_json::Number::from_f64(num).unwrap(),
                )))
            }

            Token::String(s) => {
                let string = s.clone();
                self.advance();
                Ok(Expression::Literal(Value::String(string)))
            }

            Token::True => {
                self.advance();
                Ok(Expression::Literal(Value::Bool(true)))
            }

            Token::False => {
                self.advance();
                Ok(Expression::Literal(Value::Bool(false)))
            }

            Token::Null => {
                self.advance();
                Ok(Expression::Literal(Value::Null))
            }

            Token::BindVar(name) => {
                let var_name = name.clone();
                self.advance();
                Ok(Expression::BindVariable(var_name))
            }

            Token::LeftBrace => self.parse_object_expression(),

            Token::LeftBracket => self.parse_array_expression(),

            Token::LeftParen => {
                // Check for lambda: (params) -> expr
                if self.is_lambda_params() {
                    return self.parse_lambda_expression();
                }

                self.advance();
                // Check if this is a subquery (starts with FOR or LET)
                if matches!(self.current_token(), Token::For | Token::Let) {
                    let subquery = self.parse_query(false)?; // Don't check trailing for subqueries
                    self.expect(Token::RightParen)?;
                    Ok(Expression::Subquery(Box::new(subquery)))
                } else {
                    let expr = self.parse_expression()?;
                    self.expect(Token::RightParen)?;
                    Ok(expr)
                }
            }

            // Allow unparenthesized subqueries in expression position
            // This enables: FIRST(FOR x IN col RETURN x) or LET x = FOR ...
            Token::For | Token::Let => {
                let subquery = self.parse_query(false)?;
                Ok(Expression::Subquery(Box::new(subquery)))
            }

            _ => Err(DbError::ParseError(format!(
                "Unexpected token in expression: {:?}",
                self.current_token()
            ))),
        }
    }

    fn parse_object_expression(&mut self) -> DbResult<Expression> {
        self.expect(Token::LeftBrace)?;

        let mut fields = Vec::new();

        while !matches!(self.current_token(), Token::RightBrace | Token::Eof) {
            let key = if let Token::Identifier(name) = self.current_token() {
                let k = name.clone();
                self.advance();
                k
            } else if let Token::String(s) = self.current_token() {
                let k = s.clone();
                self.advance();
                k
            } else {
                return Err(DbError::ParseError(
                    "Expected field name in object".to_string(),
                ));
            };

            // Support shorthand syntax: { city } means { city: city }
            let value = if matches!(self.current_token(), Token::Colon) {
                self.advance(); // consume :
                self.parse_expression()?
            } else {
                // Shorthand: key becomes both the field name and variable reference
                Expression::Variable(key.clone())
            };

            fields.push((key, value));

            if matches!(self.current_token(), Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(Token::RightBrace)?;

        Ok(Expression::Object(fields))
    }

    fn parse_array_expression(&mut self) -> DbResult<Expression> {
        self.expect(Token::LeftBracket)?;

        let mut elements = Vec::new();

        while !matches!(self.current_token(), Token::RightBracket | Token::Eof) {
            elements.push(self.parse_expression()?);

            if matches!(self.current_token(), Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(Token::RightBracket)?;

        Ok(Expression::Array(elements))
    }
}

pub fn parse(input: &str) -> DbResult<Query> {
    let mut parser = Parser::new(input)?;
    parser.parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_for_return() {
        let query = parse("FOR doc IN users RETURN doc").unwrap();
        assert_eq!(query.for_clauses.len(), 1);
        assert!(query.return_clause.is_some());
    }

    #[test]
    fn test_parse_for_filter_return() {
        let query = parse("FOR doc IN users FILTER doc.age > 18 RETURN doc").unwrap();
        assert_eq!(query.filter_clauses.len(), 1);
        assert!(query.return_clause.is_some());
    }

    #[test]
    fn test_parse_for_sort_limit_return() {
        let query = parse("FOR doc IN users SORT doc.name ASC LIMIT 10 RETURN doc").unwrap();
        assert!(query.sort_clause.is_some());
        assert!(query.limit_clause.is_some());
    }

    #[test]
    fn test_parse_insert() {
        let query = parse("INSERT { name: \"Alice\" } INTO users").unwrap();
        assert!(query
            .body_clauses
            .iter()
            .any(|c| matches!(c, BodyClause::Insert(_))));
    }

    #[test]
    fn test_parse_update() {
        let query = parse("FOR doc IN users UPDATE doc WITH { active: true } IN users").unwrap();
        assert!(query
            .body_clauses
            .iter()
            .any(|c| matches!(c, BodyClause::Update(_))));
    }

    #[test]
    fn test_parse_remove() {
        let query = parse("FOR doc IN users REMOVE doc IN users").unwrap();
        assert!(query
            .body_clauses
            .iter()
            .any(|c| matches!(c, BodyClause::Remove(_))));
    }

    #[test]
    fn test_parse_collect() {
        let query = parse("FOR doc IN users COLLECT city = doc.city RETURN city").unwrap();
        assert!(query
            .body_clauses
            .iter()
            .any(|c| matches!(c, BodyClause::Collect(_))));
    }

    #[test]
    fn test_parse_let_clause() {
        let query = parse("LET x = 5 RETURN x").unwrap();
        assert_eq!(query.let_clauses.len(), 1);
    }

    #[test]
    fn test_parse_return_arithmetic() {
        let query = parse("RETURN 1 + 2 * 3").unwrap();
        assert!(query.return_clause.is_some());
        let ret = query.return_clause.unwrap();
        assert!(matches!(ret.expression, Expression::BinaryOp { .. }));
    }

    #[test]
    fn test_parse_error_incomplete() {
        let result = parse("FOR doc IN");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_error_invalid_token() {
        let result = parse("FOR 123 IN users");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_sort_desc() {
        let query = parse("FOR doc IN users SORT doc.age DESC RETURN doc").unwrap();
        let sort = query.sort_clause.unwrap();
        assert_eq!(sort.fields.len(), 1);
        assert!(!sort.fields[0].1);
    }

    #[test]
    fn test_parse_multiple_filters() {
        let query =
            parse("FOR doc IN users FILTER doc.age > 18 FILTER doc.active RETURN doc").unwrap();
        assert_eq!(query.filter_clauses.len(), 2);
    }

    #[test]
    fn test_parse_nested_for() {
        let query = parse("FOR a IN users FOR b IN orders RETURN { user: a, order: b }").unwrap();
        assert_eq!(query.for_clauses.len(), 2);
    }
}
