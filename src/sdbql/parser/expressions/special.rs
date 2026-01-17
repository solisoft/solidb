//! Special expression constructs for SDBQL.
//!
//! Handles:
//! - Lambda expressions: x -> expr, (a, b) -> expr
//! - CASE expressions: CASE WHEN ... THEN ... ELSE ... END
//! - Window functions: FUNC() OVER (PARTITION BY ... ORDER BY ...)
//! - Quantifier expressions: ANY x IN array SATISFIES condition
//! - Template strings: $"Hello ${name}!"
//! - Object expressions: { field: value, ... }
//! - Array expressions: [elem1, elem2, ...]

use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{Expression, TemplateStringPart, WindowSpec};
use crate::sdbql::lexer::{TemplatePart, Token};
use crate::sdbql::parser::Parser;
use serde_json::Value;

impl Parser {
    // ========================================================================
    // Lambda expressions
    // ========================================================================

    /// Parse lambda expression: x -> expr or (a, b) -> expr
    pub(crate) fn parse_lambda_expression(&mut self) -> DbResult<Expression> {
        let params = self.parse_lambda_params()?;
        self.expect(Token::Arrow)?;
        let body = self.parse_expression()?;

        Ok(Expression::Lambda {
            params,
            body: Box::new(body),
        })
    }

    /// Parse lambda parameters: single identifier or parenthesized list
    fn parse_lambda_params(&mut self) -> DbResult<Vec<String>> {
        let mut params = Vec::new();

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
            params.push(name.clone());
            self.advance();
        }

        Ok(params)
    }

    /// Check if the current position looks like the start of a lambda: (params) ->
    pub(crate) fn is_lambda_params(&self) -> bool {
        let mut pos = self.position + 1;
        let mut depth = 1;

        while let Some(tok) = self.tokens.get(pos) {
            match tok {
                Token::LeftParen => depth += 1,
                Token::RightParen => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(self.tokens.get(pos + 1), Some(Token::Arrow));
                    }
                }
                Token::Comma | Token::Identifier(_) => {}
                _ => return false,
            }
            pos += 1;
        }
        false
    }

    // ========================================================================
    // CASE expressions
    // ========================================================================

    /// Parse CASE expression
    /// Simple form: CASE expr WHEN val1 THEN res1 WHEN val2 THEN res2 ELSE default END
    /// Searched form: CASE WHEN cond1 THEN res1 WHEN cond2 THEN res2 ELSE default END
    pub(super) fn parse_case_expression(&mut self) -> DbResult<Expression> {
        self.advance(); // consume CASE

        // Check if this is a simple or searched CASE
        let operand = if !matches!(self.current_token(), Token::When) {
            Some(Box::new(self.parse_expression()?))
        } else {
            None
        };

        // Parse WHEN clauses
        let mut when_clauses = Vec::new();
        while matches!(self.current_token(), Token::When) {
            self.advance(); // consume WHEN

            let condition = self.parse_expression()?;

            if !matches!(self.current_token(), Token::Then) {
                return Err(DbError::ParseError(
                    "Expected THEN after WHEN condition".to_string(),
                ));
            }
            self.advance(); // consume THEN

            let result = self.parse_expression()?;
            when_clauses.push((condition, result));
        }

        if when_clauses.is_empty() {
            return Err(DbError::ParseError(
                "CASE expression requires at least one WHEN clause".to_string(),
            ));
        }

        // Parse optional ELSE clause
        let else_clause = if matches!(self.current_token(), Token::Else) {
            self.advance(); // consume ELSE
            Some(Box::new(self.parse_expression()?))
        } else {
            None
        };

        // Expect END
        if !matches!(self.current_token(), Token::End) {
            return Err(DbError::ParseError(
                "Expected END to close CASE expression".to_string(),
            ));
        }
        self.advance(); // consume END

        Ok(Expression::Case {
            operand,
            when_clauses,
            else_clause,
        })
    }

    // ========================================================================
    // Window functions
    // ========================================================================

    /// Parse window function after detecting OVER keyword
    pub(super) fn parse_window_function(
        &mut self,
        function: String,
        arguments: Vec<Expression>,
    ) -> DbResult<Expression> {
        self.expect(Token::Over)?;
        self.expect(Token::LeftParen)?;

        let over_clause = self.parse_window_spec()?;

        self.expect(Token::RightParen)?;

        Ok(Expression::WindowFunctionCall {
            function: function.to_uppercase(),
            arguments,
            over_clause,
        })
    }

    /// Parse window specification inside OVER(...)
    fn parse_window_spec(&mut self) -> DbResult<WindowSpec> {
        let partition_by = self.parse_partition_by()?;
        let order_by = self.parse_order_by()?;

        Ok(WindowSpec {
            partition_by,
            order_by,
        })
    }

    /// Parse PARTITION BY clause in window spec
    fn parse_partition_by(&mut self) -> DbResult<Vec<Expression>> {
        let mut partition_by = Vec::new();

        if !matches!(self.current_token(), Token::Partition) {
            return Ok(partition_by);
        }

        self.advance(); // consume PARTITION

        // Expect BY keyword
        match self.current_token() {
            Token::Identifier(s) if s.to_uppercase() == "BY" => {
                self.advance();
            }
            _ => {
                return Err(DbError::ParseError(
                    "Expected BY after PARTITION".to_string(),
                ));
            }
        }

        // Parse partition expressions
        loop {
            partition_by.push(self.parse_expression()?);
            if matches!(self.current_token(), Token::Comma) {
                // Check if next is ORDER or end of spec
                if matches!(self.peek_token(1), Token::Sort) {
                    break;
                }
                if let Token::Identifier(s) = self.peek_token(1) {
                    if s.to_uppercase() == "ORDER" {
                        break;
                    }
                }
                self.advance();
            } else {
                break;
            }
        }

        Ok(partition_by)
    }

    /// Parse ORDER BY clause in window spec
    fn parse_order_by(&mut self) -> DbResult<Vec<(Expression, bool)>> {
        let mut order_by = Vec::new();

        if !matches!(self.current_token(), Token::Sort) {
            return Ok(order_by);
        }

        self.advance(); // consume ORDER (Sort token)

        // Expect BY keyword
        match self.current_token() {
            Token::Identifier(s) if s.to_uppercase() == "BY" => {
                self.advance();
            }
            _ => {
                return Err(DbError::ParseError("Expected BY after ORDER".to_string()));
            }
        }

        // Parse order expressions with optional ASC/DESC
        loop {
            let expr = self.parse_expression()?;
            let ascending = match self.current_token() {
                Token::Desc => {
                    self.advance();
                    false
                }
                Token::Asc => {
                    self.advance();
                    true
                }
                _ => true,
            };
            order_by.push((expr, ascending));

            if matches!(self.current_token(), Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }

        Ok(order_by)
    }

    // ========================================================================
    // Quantifier expressions
    // ========================================================================

    /// Parse quantifier expression: ANY x IN array SATISFIES condition
    pub(super) fn parse_quantifier_expression(&mut self, name: &str) -> DbResult<Expression> {
        self.advance(); // consume ANY/SOME/ALL

        let variable = if let Token::Identifier(v) = self.current_token() {
            v.clone()
        } else {
            return Err(DbError::ParseError(
                "Expected variable name after quantifier".to_string(),
            ));
        };
        self.advance();

        self.expect(Token::In)?;

        let array_expr = self.parse_expression()?;

        let condition = if matches!(self.current_token(), Token::Satisfies) {
            self.advance();
            self.parse_expression()?
        } else {
            Expression::Literal(Value::Bool(true))
        };

        // Construct desugared ANY(array, x -> condition)
        let lambda = Expression::Lambda {
            params: vec![variable],
            body: Box::new(condition),
        };

        Ok(Expression::FunctionCall {
            name: name.to_string(),
            args: vec![array_expr, lambda],
        })
    }

    // ========================================================================
    // Template strings
    // ========================================================================

    /// Parse template string parts from lexer into AST nodes
    pub(super) fn parse_template_string(
        &mut self,
        parts: Vec<TemplatePart>,
    ) -> DbResult<Expression> {
        let mut parsed_parts = Vec::new();

        for part in parts {
            match part {
                TemplatePart::Literal(s) => {
                    parsed_parts.push(TemplateStringPart::Literal(s));
                }
                TemplatePart::Expression(expr_str) => {
                    let mut expr_parser = Parser::new(&expr_str)?;
                    let expr = expr_parser.parse_expression()?;
                    parsed_parts.push(TemplateStringPart::Expression(Box::new(expr)));
                }
            }
        }

        Ok(Expression::TemplateString {
            parts: parsed_parts,
        })
    }

    // ========================================================================
    // Object and array expressions
    // ========================================================================

    /// Parse object expression: { field: value, ... }
    pub(crate) fn parse_object_expression(&mut self) -> DbResult<Expression> {
        self.expect(Token::LeftBrace)?;

        let mut fields = Vec::new();

        while !matches!(self.current_token(), Token::RightBrace | Token::Eof) {
            let key = self.parse_object_key()?;

            // Support shorthand syntax: { city } means { city: city }
            let value = if matches!(self.current_token(), Token::Colon) {
                self.advance();
                self.parse_expression()?
            } else {
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

    /// Parse object key: identifier, string, or keyword
    fn parse_object_key(&mut self) -> DbResult<String> {
        if let Token::Identifier(name) = self.current_token() {
            let k = name.clone();
            self.advance();
            return Ok(k);
        }

        if let Token::String(s) = self.current_token() {
            let k = s.clone();
            self.advance();
            return Ok(k);
        }

        // Handle keyword tokens that can be used as field names
        let keyword_name = match self.current_token() {
            Token::Sort => Some("order"),
            Token::Count => Some("count"),
            Token::Filter => Some("filter"),
            Token::Return => Some("return"),
            Token::In => Some("in"),
            Token::For => Some("for"),
            Token::Let => Some("let"),
            Token::Limit => Some("limit"),
            Token::Partition => Some("partition"),
            Token::Over => Some("over"),
            Token::Case => Some("case"),
            Token::When => Some("when"),
            Token::Then => Some("then"),
            Token::Else => Some("else"),
            Token::End => Some("end"),
            _ => None,
        };

        if let Some(name) = keyword_name {
            self.advance();
            Ok(name.to_string())
        } else {
            Err(DbError::ParseError(
                "Expected field name in object".to_string(),
            ))
        }
    }

    /// Parse array expression: [elem1, elem2, ...]
    pub(crate) fn parse_array_expression(&mut self) -> DbResult<Expression> {
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
