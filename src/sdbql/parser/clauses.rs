//! Clause parsing methods for SDBQL.
//!
//! This module contains methods for parsing SDBQL clauses such as:
//! - LET clauses
//! - FOR clauses
//! - FILTER clauses
//! - JOIN clauses
//! - INSERT/UPDATE/REMOVE/UPSERT clauses
//! - COLLECT clauses
//! - SORT/LIMIT/RETURN clauses
//! - Graph traversal clauses

use super::Parser;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::*;
use crate::sdbql::lexer::Token;
use serde_json::Value;

impl Parser {
    /// Parse LET clause with support for multiple comma-separated bindings
    /// e.g., LET a = 1, b = 2, c = 3
    pub(crate) fn parse_let_clause(&mut self) -> DbResult<Vec<LetClause>> {
        self.expect(Token::Let)?;

        let mut clauses = Vec::new();

        loop {
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

            clauses.push(LetClause {
                variable,
                expression,
            });

            // Check for comma to continue parsing more bindings
            if matches!(self.current_token(), Token::Comma) {
                self.advance(); // consume comma
            } else {
                break;
            }
        }

        Ok(clauses)
    }

    pub(crate) fn parse_create_stream_clause(&mut self) -> DbResult<CreateStreamClause> {
        self.expect(Token::Create)?;
        self.expect(Token::Stream)?;

        let name = if let Token::Identifier(n) = self.current_token() {
            let name = n.clone();
            self.advance();
            name
        } else {
            return Err(DbError::ParseError("Expected stream name".to_string()));
        };

        self.expect(Token::As)?;

        Ok(CreateStreamClause {
            name,
            if_not_exists: false,
        })
    }

    pub(crate) fn parse_create_materialized_view_clause(
        &mut self,
    ) -> DbResult<CreateMaterializedViewClause> {
        self.expect(Token::Create)?;
        self.expect(Token::Materialized)?;
        self.expect(Token::View)?;

        let name = if let Token::Identifier(n) = self.current_token() {
            let name = n.clone();
            self.advance();
            name
        } else {
            return Err(DbError::ParseError("Expected view name".to_string()));
        };

        self.expect(Token::As)?;

        // Parse the inner query - false means don't check for trailing tokens (as we might be inside a larger structure, though unlikely for MV)
        // But importantly, we want to parse the Full Query structure.
        let query = self.parse_query(false)?;

        Ok(CreateMaterializedViewClause {
            name,
            if_not_exists: false,
            query: Box::new(query),
        })
    }

    pub(crate) fn parse_refresh_materialized_view_clause(
        &mut self,
    ) -> DbResult<RefreshMaterializedViewClause> {
        self.expect(Token::Refresh)?;
        self.expect(Token::Materialized)?;
        self.expect(Token::View)?;

        let name = if let Token::Identifier(n) = self.current_token() {
            let name = n.clone();
            self.advance();
            name
        } else {
            return Err(DbError::ParseError("Expected view name".to_string()));
        };

        Ok(RefreshMaterializedViewClause { name })
    }

    pub(crate) fn parse_window_clause(&mut self) -> DbResult<WindowClause> {
        self.expect(Token::Window)?;

        let window_type = match self.current_token() {
            Token::Tumbling => {
                self.advance();
                WindowType::Tumbling
            }
            Token::Sliding => {
                self.advance();
                WindowType::Sliding
            }
            _ => {
                return Err(DbError::ParseError(
                    "Expected TUMBLING or SLIDING".to_string(),
                ))
            }
        };

        // Expect (SIZE "duration")
        self.expect(Token::LeftParen)?;
        self.expect(Token::Size)?;

        let duration = if let Token::String(s) = self.current_token() {
            let d = s.clone();
            self.advance();
            d
        } else {
            return Err(DbError::ParseError("Expected duration string".to_string()));
        };

        self.expect(Token::RightParen)?;

        Ok(WindowClause {
            window_type,
            duration,
        })
    }

    pub(crate) fn parse_filter_clause(&mut self) -> DbResult<FilterClause> {
        self.expect(Token::Filter)?;
        let expression = self.parse_expression()?;
        Ok(FilterClause { expression })
    }

    /// Parse JOIN clause: [LEFT|RIGHT|FULL [OUTER]] JOIN collection ON condition
    /// Variable is automatically derived from collection name
    pub(crate) fn parse_join_clause(&mut self) -> DbResult<JoinClause> {
        // Check for optional join type keyword
        let join_type = if matches!(self.current_token(), Token::Left) {
            self.advance(); // consume LEFT
            JoinType::Left
        } else if matches!(self.current_token(), Token::Right) {
            self.advance(); // consume RIGHT
            JoinType::Right
        } else if matches!(self.current_token(), Token::Full) {
            self.advance(); // consume FULL
                            // Check for optional OUTER keyword
            if matches!(self.current_token(), Token::Outer) {
                self.advance(); // consume OUTER
            }
            JoinType::FullOuter
        } else {
            JoinType::Inner
        };

        // Expect JOIN keyword
        self.expect(Token::Join)?;

        // Parse collection name (variable will be same as collection)
        let collection = if let Token::Identifier(name) = self.current_token() {
            let coll = name.clone();
            self.advance();
            coll
        } else {
            return Err(DbError::ParseError(
                "Expected collection name after JOIN".to_string(),
            ));
        };

        // Expect ON keyword
        self.expect(Token::On)?;

        // Parse join condition expression
        let condition = self.parse_expression()?;

        Ok(JoinClause {
            join_type,
            variable: collection.clone(), // Variable same as collection name
            collection,
            condition,
        })
    }

    pub(crate) fn parse_insert_clause(&mut self) -> DbResult<InsertClause> {
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

    pub(crate) fn parse_update_clause(&mut self) -> DbResult<UpdateClause> {
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

    pub(crate) fn parse_remove_clause(&mut self) -> DbResult<RemoveClause> {
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

    pub(crate) fn parse_upsert_clause(&mut self) -> DbResult<UpsertClause> {
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
    pub(crate) fn parse_collect_clause(&mut self) -> DbResult<CollectClause> {
        self.expect(Token::Collect)?;

        let mut group_vars = Vec::new();
        let mut into_var = None;
        let mut count_var = None;
        let mut aggregates = Vec::new();

        // Parse group variables: var = expr [, var = expr]*
        // Note: Can't use while let here - need peek-ahead logic to check for assignment token
        #[allow(clippy::while_let_loop)]
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

            // Note: Can't use while let here - need complex nested parsing logic
            #[allow(clippy::while_let_loop)]
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
    pub(crate) fn parse_graph_traversal_clause(
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
    pub(crate) fn parse_shortest_path_clause(
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

    pub(crate) fn parse_sort_clause(&mut self) -> DbResult<SortClause> {
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

    pub(crate) fn parse_limit_clause(&mut self) -> DbResult<LimitClause> {
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

    pub(crate) fn parse_return_clause(&mut self) -> DbResult<ReturnClause> {
        self.expect(Token::Return)?;
        let expression = self.parse_expression()?;
        Ok(ReturnClause { expression })
    }
}
