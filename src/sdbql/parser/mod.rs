//! Parser module for SDBQL query language.
//!
//! This module contains the parser for SDBQL (SoliDB Query Language), which converts
//! tokenized input into an Abstract Syntax Tree (AST).

mod clauses;
mod expressions;
#[cfg(test)]
mod tests;

use super::ast::*;
use super::lexer::{Lexer, Token};
use crate::error::{DbError, DbResult};

/// Parser for SDBQL queries
pub struct Parser {
    pub(crate) tokens: Vec<Token>,
    pub(crate) position: usize,
    pub(crate) allow_in_operator: bool,
}

impl Parser {
    /// Create a new parser from an input string
    pub fn new(input: &str) -> DbResult<Self> {
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize()?;

        Ok(Self {
            tokens,
            position: 0,
            allow_in_operator: true,
        })
    }

    /// Get the current token
    pub(crate) fn current_token(&self) -> &Token {
        self.tokens.get(self.position).unwrap_or(&Token::Eof)
    }

    /// Peek at a token at a given offset from the current position
    pub(crate) fn peek_token(&self, offset: usize) -> &Token {
        self.tokens
            .get(self.position + offset)
            .unwrap_or(&Token::Eof)
    }

    /// Advance to the next token
    pub(crate) fn advance(&mut self) {
        if self.position < self.tokens.len() {
            self.position += 1;
        }
    }

    /// Expect a specific token and advance, or return an error
    pub(crate) fn expect(&mut self, expected: Token) -> DbResult<()> {
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

    /// Parse a complete query
    pub fn parse(&mut self) -> DbResult<Query> {
        self.parse_query(true)
    }

    /// Parse a query, optionally checking for trailing tokens (false for subqueries)
    pub(crate) fn parse_query(&mut self, check_trailing: bool) -> DbResult<Query> {
        // Parse optional CREATE STREAM or CREATE MATERIALIZED VIEW
        let (create_stream_clause, create_mv_clause) =
            if matches!(self.current_token(), Token::Create) {
                if matches!(self.peek_token(1), Token::Stream) {
                    (Some(self.parse_create_stream_clause()?), None)
                } else if matches!(self.peek_token(1), Token::Materialized) {
                    (None, Some(self.parse_create_materialized_view_clause()?))
                } else {
                    return Err(DbError::ParseError(
                        "Expected STREAM or MATERIALIZED VIEW after CREATE".to_string(),
                    ));
                }
            } else {
                (None, None)
            };

        // Parse optional REFRESH MATERIALIZED VIEW
        let refresh_mv_clause = if matches!(self.current_token(), Token::Refresh) {
            Some(self.parse_refresh_materialized_view_clause()?)
        } else {
            None
        };

        // Parse initial LET clauses (before any FOR - these are evaluated once)
        // Supports multiple comma-separated bindings: LET a = 1, b = 2, c = 3
        let mut let_clauses = Vec::new();
        while matches!(self.current_token(), Token::Let) {
            let_clauses.extend(self.parse_let_clause()?);
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
                let let_clauses_parsed = self.parse_let_clause()?;
                // LET after FOR goes to body_clauses (correlated), not let_clauses
                for let_clause in let_clauses_parsed {
                    body_clauses.push(BodyClause::Let(let_clause));
                }
            } else if matches!(self.current_token(), Token::Collect) {
                let collect_clause = self.parse_collect_clause()?;
                body_clauses.push(BodyClause::Collect(collect_clause));
            } else if matches!(self.current_token(), Token::Join)
                || (matches!(self.current_token(), Token::Left)
                    && matches!(self.peek_token(1), Token::Join))
                || (matches!(self.current_token(), Token::Right)
                    && matches!(self.peek_token(1), Token::Join))
                || (matches!(self.current_token(), Token::Full)
                    && (matches!(self.peek_token(1), Token::Join)
                        || matches!(self.peek_token(1), Token::Outer)))
            {
                let join_clause = self.parse_join_clause()?;
                body_clauses.push(BodyClause::Join(join_clause));
            } else if matches!(self.current_token(), Token::Window) {
                // WINDOW clause inside body (stream processing)
                let window_clause = self.parse_window_clause()?;
                // Note: We might want to store this in body_clauses to preserve order relative to filters
                body_clauses.push(BodyClause::Window(window_clause));
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

            if return_clause.is_none()
                && !has_mutation
                && create_stream_clause.is_none()
                && create_mv_clause.is_none()
                && refresh_mv_clause.is_none()
            {
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

        // Extract window clause for top-level access if present
        let window_clause = body_clauses.iter().find_map(|c| match c {
            BodyClause::Window(w) => Some(w.clone()),
            _ => None,
        });

        // Extract join clauses for top-level access
        let join_clauses: Vec<JoinClause> = body_clauses
            .iter()
            .filter_map(|c| match c {
                BodyClause::Join(j) => Some(j.clone()),
                _ => None,
            })
            .collect();

        Ok(Query {
            create_stream_clause,
            create_materialized_view_clause: create_mv_clause,
            refresh_materialized_view_clause: refresh_mv_clause,
            let_clauses,
            for_clauses,
            join_clauses,
            filter_clauses,
            sort_clause,
            limit_clause,
            return_clause,
            window_clause,
            body_clauses,
        })
    }
}

/// Result of parsing a FOR clause - could be regular FOR or graph traversal
pub(crate) enum ForOrGraph {
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
    pub(crate) fn try_parse_for_or_graph(&mut self) -> DbResult<ForOrGraph> {
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

/// Parse an SDBQL query string into an AST
pub fn parse(input: &str) -> DbResult<Query> {
    let mut parser = Parser::new(input)?;
    parser.parse()
}
