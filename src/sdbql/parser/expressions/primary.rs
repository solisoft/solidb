//! Primary expression parsing for SDBQL.
//!
//! Handles:
//! - Literals: integers, floats, strings, booleans, null
//! - Variables and identifiers
//! - Function calls
//! - Bind variables (@name)
//! - Parenthesized expressions and subqueries
//! - Postfix operations: field access (.), optional chaining (?.), array indexing ([])

use crate::error::{DbError, DbResult};
use crate::sdbql::ast::Expression;
use crate::sdbql::lexer::Token;
use crate::sdbql::parser::Parser;
use serde_json::Value;

impl Parser {
    /// Parse postfix expression: field access, optional chaining, array indexing
    pub(super) fn parse_postfix_expression(&mut self) -> DbResult<Expression> {
        let mut expr = self.parse_primary_expression()?;

        loop {
            match self.current_token() {
                Token::Dot => {
                    expr = self.parse_field_access(expr)?;
                }
                Token::QuestionDot => {
                    expr = self.parse_optional_field_access(expr)?;
                }
                Token::LeftBracket => {
                    expr = self.parse_bracket_access(expr)?;
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    /// Parse field access: expr.field
    fn parse_field_access(&mut self, base: Expression) -> DbResult<Expression> {
        self.advance(); // consume '.'

        if let Some(field_name) = self.get_field_name() {
            self.advance();
            Ok(Expression::FieldAccess(Box::new(base), field_name))
        } else {
            Err(DbError::ParseError(format!(
                "Expected field name after '.', found {:?}. \
                 If the field name conflicts with a reserved word, \
                 use bracket notation: doc[\"field\"].",
                self.current_token()
            )))
        }
    }

    /// Parse optional field access: expr?.field
    fn parse_optional_field_access(&mut self, base: Expression) -> DbResult<Expression> {
        self.advance(); // consume '?.'

        if let Some(field_name) = self.get_field_name() {
            self.advance();
            Ok(Expression::OptionalFieldAccess(Box::new(base), field_name))
        } else {
            Err(DbError::ParseError(format!(
                "Expected field name after '?.', found {:?}. \
                 If the field name conflicts with a reserved word, \
                 use bracket notation: doc[\"field\"].",
                self.current_token()
            )))
        }
    }

    /// Parse bracket access: expr[index] or expr[*]
    fn parse_bracket_access(&mut self, base: Expression) -> DbResult<Expression> {
        self.advance(); // consume '['

        // Check for [*] array spread syntax
        if matches!(self.current_token(), Token::Star) {
            return self.parse_array_spread_access(base);
        }

        let index_expr = self.parse_expression()?;
        self.expect(Token::RightBracket)?;

        // Determine access type based on index expression
        Ok(match &index_expr {
            Expression::Literal(Value::Number(_)) => {
                // Numeric index: array access
                Expression::ArrayAccess(Box::new(base), Box::new(index_expr))
            }
            Expression::Literal(Value::String(s)) => {
                // String literal: static field access
                Expression::FieldAccess(Box::new(base), s.clone())
            }
            _ => {
                // Dynamic field access: doc[@field], doc[someVar], etc.
                Expression::DynamicFieldAccess(Box::new(base), Box::new(index_expr))
            }
        })
    }

    /// Parse array spread access: expr[*] or expr[*].field.path
    fn parse_array_spread_access(&mut self, base: Expression) -> DbResult<Expression> {
        self.advance(); // consume '*'
        self.expect(Token::RightBracket)?;

        // Collect subsequent dot-separated field path
        let field_path = if matches!(self.current_token(), Token::Dot) {
            let mut path = String::new();
            while matches!(self.current_token(), Token::Dot) {
                self.advance();
                if let Some(name) = Self::token_to_field_name(self.current_token()) {
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

        Ok(Expression::ArraySpreadAccess(Box::new(base), field_path))
    }

    /// Parse primary expression (highest precedence)
    pub(super) fn parse_primary_expression(&mut self) -> DbResult<Expression> {
        match self.current_token() {
            Token::Identifier(ref name) => {
                // Quantifier: NONE x IN array SATISFIES condition
                // (also accepts the parenthesized form NONE(x IN ... SATISFIES ...))
                if name.eq_ignore_ascii_case("NONE") && self.is_none_quantifier() {
                    let parenthesized = matches!(self.peek_token(1), Token::LeftParen);
                    self.advance(); // consume NONE
                    if parenthesized {
                        self.advance(); // consume (
                    }
                    return self.parse_quantifier_expression_after_keyword("NONE", parenthesized);
                }
                self.parse_identifier_expression(name.clone())
            }
            Token::Any => self.parse_quantifier_expression("ANY"),
            Token::Count => self.parse_keyword_as_function("COUNT"),
            Token::Left => self.parse_keyword_as_function_no_window(
                "LEFT",
                "Unexpected token in expression: Left",
            ),
            Token::Right => self.parse_keyword_as_function_no_window(
                "RIGHT",
                "Unexpected token in expression: Right",
            ),
            Token::Like => self.parse_keyword_as_function_no_window(
                "LIKE",
                "Unexpected token in expression: Like",
            ),
            Token::Replace => self.parse_keyword_as_function_no_window(
                "REPLACE",
                "Unexpected token in expression: Replace",
            ),
            Token::Join => self.parse_keyword_as_function_no_window(
                "JOIN",
                "Unexpected token in expression: Join",
            ),
            Token::Filter => self.parse_keyword_as_function_no_window(
                "FILTER",
                "Unexpected token in expression: Filter",
            ),
            Token::Integer(n) => self.parse_integer(*n),
            Token::Float(f) => self.parse_float(*f),
            Token::String(s) => self.parse_string(s.clone()),
            Token::True => self.parse_boolean(true),
            Token::False => self.parse_boolean(false),
            Token::Null => self.parse_null(),
            Token::BindVar(name) => self.parse_bind_variable(name.clone()),
            Token::LeftBrace => self.parse_object_expression(),
            Token::LeftBracket => self.parse_array_expression(),
            Token::LeftParen => self.parse_parenthesized_expression(),
            Token::For | Token::Let => self.parse_unparenthesized_subquery(),
            Token::Case => self.parse_case_expression(),
            Token::TemplateString(parts) => {
                let parts = parts.clone();
                self.advance();
                self.parse_template_string(parts)
            }
            _ => Err(DbError::ParseError(format!(
                "Unexpected token in expression: {:?}",
                self.current_token()
            ))),
        }
    }

    /// Detect the NONE quantifier form: `NONE x IN ...` or `NONE(x IN ...)`.
    /// A plain function call like `NONE(arr)` or `NONE(arr, x -> cond)` does
    /// not match (the token after the variable must be IN).
    fn is_none_quantifier(&self) -> bool {
        // NONE x IN ...
        if matches!(self.peek_token(1), Token::Identifier(_))
            && matches!(self.peek_token(2), Token::In)
        {
            return true;
        }
        // NONE(x IN ...)
        matches!(self.peek_token(1), Token::LeftParen)
            && matches!(self.peek_token(2), Token::Identifier(_))
            && matches!(self.peek_token(3), Token::In)
    }

    /// Parse identifier: variable or function call
    fn parse_identifier_expression(&mut self, name: String) -> DbResult<Expression> {
        // Check for lambda: x -> expr
        if matches!(self.peek_token(1), Token::Arrow) {
            return self.parse_lambda_expression();
        }

        self.advance();

        // Check if this is a function call
        if matches!(self.current_token(), Token::LeftParen) {
            self.advance(); // consume '('
            let args = self.parse_function_call_args()?;

            // Check for OVER clause - if present, this is a window function
            if matches!(self.current_token(), Token::Over) {
                return self.parse_window_function(name, args);
            }

            Ok(Expression::FunctionCall { name, args })
        } else {
            Ok(Expression::Variable(name))
        }
    }

    /// Parse integer literal
    fn parse_integer(&mut self, n: i64) -> DbResult<Expression> {
        self.advance();
        Ok(Expression::Literal(Value::Number(
            serde_json::Number::from(n),
        )))
    }

    /// Parse float literal
    fn parse_float(&mut self, f: f64) -> DbResult<Expression> {
        self.advance();
        Ok(Expression::Literal(Value::Number(
            serde_json::Number::from_f64(f).unwrap(),
        )))
    }

    /// Parse string literal
    fn parse_string(&mut self, s: String) -> DbResult<Expression> {
        self.advance();
        Ok(Expression::Literal(Value::String(s)))
    }

    /// Parse boolean literal
    fn parse_boolean(&mut self, value: bool) -> DbResult<Expression> {
        self.advance();
        Ok(Expression::Literal(Value::Bool(value)))
    }

    /// Parse null literal
    fn parse_null(&mut self) -> DbResult<Expression> {
        self.advance();
        Ok(Expression::Literal(Value::Null))
    }

    /// Parse bind variable (@name)
    fn parse_bind_variable(&mut self, name: String) -> DbResult<Expression> {
        self.advance();
        Ok(Expression::BindVariable(name))
    }

    /// Parse parenthesized expression or subquery
    fn parse_parenthesized_expression(&mut self) -> DbResult<Expression> {
        // Check for lambda: (params) -> expr
        if self.is_lambda_params() {
            return self.parse_lambda_expression();
        }

        self.advance(); // consume '('

        // Check if this is a subquery (starts with FOR or LET)
        if matches!(self.current_token(), Token::For | Token::Let) {
            let subquery = self.parse_query(false)?;
            self.expect(Token::RightParen)?;
            Ok(Expression::Subquery(Box::new(subquery)))
        } else {
            let expr = self.parse_expression()?;
            self.expect(Token::RightParen)?;
            Ok(expr)
        }
    }

    /// Parse unparenthesized subquery (FOR ... or LET ...)
    fn parse_unparenthesized_subquery(&mut self) -> DbResult<Expression> {
        let subquery = self.parse_query(false)?;
        Ok(Expression::Subquery(Box::new(subquery)))
    }
}
