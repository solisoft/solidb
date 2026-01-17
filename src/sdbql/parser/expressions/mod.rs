//! Expression parsing methods for SDBQL.
//!
//! This module contains methods for parsing SDBQL expressions, organized into:
//! - `precedence`: Operator precedence chain (ternary → logical → bitwise → comparison → arithmetic)
//! - `operators`: Comparison and binary operator parsing
//! - `primary`: Primary expressions, postfix operations, field access
//! - `special`: Special constructs (CASE, lambda, window, template, object, array)

mod operators;
mod precedence;
mod primary;
mod special;

use crate::error::{DbError, DbResult};
use crate::sdbql::ast::Expression;
use crate::sdbql::lexer::Token;
use crate::sdbql::parser::Parser;

impl Parser {
    /// Entry point for expression parsing
    pub(crate) fn parse_expression(&mut self) -> DbResult<Expression> {
        self.parse_ternary_expression()
    }
}

// ============================================================================
// Helper functions for code deduplication
// ============================================================================

impl Parser {
    /// Parse function call arguments: (arg1, arg2, ...)
    /// Assumes the opening '(' has already been consumed.
    /// Returns the list of argument expressions.
    pub(super) fn parse_function_call_args(&mut self) -> DbResult<Vec<Expression>> {
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
        Ok(args)
    }

    /// Convert a token to a field name string if it can be used as a field name.
    /// Handles both identifiers and keywords that can be used as field names.
    pub(super) fn token_to_field_name(token: &Token) -> Option<String> {
        match token {
            Token::Identifier(name) => Some(name.clone()),
            // Keywords that can be used as field names
            Token::Sort => Some("sort".to_string()),
            Token::Count => Some("count".to_string()),
            Token::Filter => Some("filter".to_string()),
            Token::Return => Some("return".to_string()),
            Token::In => Some("in".to_string()),
            Token::For => Some("for".to_string()),
            Token::Let => Some("let".to_string()),
            Token::Limit => Some("limit".to_string()),
            Token::Partition => Some("partition".to_string()),
            Token::Over => Some("over".to_string()),
            Token::Case => Some("case".to_string()),
            Token::When => Some("when".to_string()),
            Token::Then => Some("then".to_string()),
            Token::Else => Some("else".to_string()),
            Token::End => Some("end".to_string()),
            Token::True => Some("true".to_string()),
            Token::False => Some("false".to_string()),
            Token::Null => Some("null".to_string()),
            Token::And => Some("and".to_string()),
            Token::Or => Some("or".to_string()),
            Token::Not => Some("not".to_string()),
            Token::Like => Some("like".to_string()),
            Token::Insert => Some("insert".to_string()),
            Token::Update => Some("update".to_string()),
            Token::Remove => Some("remove".to_string()),
            Token::Upsert => Some("upsert".to_string()),
            Token::Into => Some("into".to_string()),
            Token::With => Some("with".to_string()),
            Token::Collect => Some("collect".to_string()),
            Token::Aggregate => Some("aggregate".to_string()),
            Token::Asc => Some("asc".to_string()),
            Token::Desc => Some("desc".to_string()),
            _ => None,
        }
    }

    /// Get field name from current token - handles both identifiers and keywords
    pub(super) fn get_field_name(&self) -> Option<String> {
        Self::token_to_field_name(self.current_token())
    }

    /// Parse a keyword token as a function call.
    /// Used for tokens like COUNT, LEFT, RIGHT that can be both keywords and functions.
    /// Returns the function call expression if followed by '(', otherwise returns an error.
    pub(super) fn parse_keyword_as_function(&mut self, name: &str) -> DbResult<Expression> {
        self.advance(); // consume the keyword token

        if matches!(self.current_token(), Token::LeftParen) {
            self.advance(); // consume '('
            let args = self.parse_function_call_args()?;

            // Check for OVER clause - if present, this is a window function
            if matches!(self.current_token(), Token::Over) {
                return self.parse_window_function(name.to_string(), args);
            }

            Ok(Expression::FunctionCall {
                name: name.to_string(),
                args,
            })
        } else {
            Err(DbError::ParseError(format!(
                "Expected '(' after {}",
                name
            )))
        }
    }

    /// Parse a keyword token as a function call without window function support.
    /// Used for LEFT/RIGHT which don't support OVER clauses.
    pub(super) fn parse_keyword_as_function_no_window(
        &mut self,
        name: &str,
        error_msg: &str,
    ) -> DbResult<Expression> {
        self.advance(); // consume the keyword token

        if matches!(self.current_token(), Token::LeftParen) {
            self.advance(); // consume '('
            let args = self.parse_function_call_args()?;

            Ok(Expression::FunctionCall {
                name: name.to_string(),
                args,
            })
        } else {
            Err(DbError::ParseError(error_msg.to_string()))
        }
    }
}
