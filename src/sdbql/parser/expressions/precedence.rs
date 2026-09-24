//! Operator precedence chain for SDBQL expression parsing.
//!
//! Precedence (lowest to highest):
//! 1. Ternary: `?:`
//! 2. Null coalesce: `??`
//! 3. Logical OR: `||`
//! 4. Pipeline: `|>`
//! 5. Boolean OR: `OR`
//! 6. Boolean AND: `AND`
//! 7. Bitwise OR: `|`
//! 8. Bitwise XOR: `^`
//! 9. Bitwise AND: `&`
//! 10. Comparison: `==`, `!=`, `<`, `<=`, `>`, `>=`, `IN`, `LIKE`, etc.
//! 11. Range: `..`
//! 12. Shift: `<<`, `>>`
//! 13. Additive: `+`, `-`
//! 14. Multiplicative: `*`, `/`, `%`
//! 15. Unary: `!`, `-`, `~`
//! 16. Postfix: `.`, `?.`, `[]`
//! 17. Primary: literals, variables, function calls, etc.

use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{ArrayQuantifier, BinaryOperator, Expression, UnaryOperator};
use crate::sdbql::lexer::Token;
use crate::sdbql::parser::Parser;

impl Parser {
    /// Parse ternary expression: condition ? true_expr : false_expr
    /// Lowest precedence, right-associative
    pub(super) fn parse_ternary_expression(&mut self) -> DbResult<Expression> {
        let condition = self.parse_null_coalesce_expression()?;

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

    /// Parse null coalescing expression: left ?? right
    /// Returns left if left is not null, otherwise evaluates and returns right
    fn parse_null_coalesce_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_logical_or_expression()?;

        while matches!(self.current_token(), Token::NullCoalesce) {
            self.advance(); // consume ??
            let right = self.parse_logical_or_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op: BinaryOperator::NullCoalesce,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Parse logical OR expression: left || right
    fn parse_logical_or_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_pipeline_expression()?;

        while matches!(self.current_token(), Token::DoublePipe) {
            self.advance(); // consume ||
            let right = self.parse_pipeline_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op: BinaryOperator::LogicalOr,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Parse pipeline expression: expr |> FUNC(args) |> FUNC2(args)
    fn parse_pipeline_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_or_expression()?;

        while matches!(self.current_token(), Token::PipeRight) {
            self.advance(); // consume |>

            let func_name = self.parse_pipeline_function_name()?;
            self.expect(Token::LeftParen)?;
            let args = self.parse_function_call_args()?;

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
            Token::Filter => "FILTER".to_string(),
            Token::Sort => "SORT".to_string(),
            Token::Count => "COUNT".to_string(),
            Token::Any => "ANY".to_string(),
            Token::Return => "RETURN".to_string(),
            Token::In => "IN".to_string(),
            Token::Replace => "REPLACE".to_string(),
            Token::Like => "LIKE".to_string(),
            Token::Left => "LEFT".to_string(),
            Token::Right => "RIGHT".to_string(),
            Token::Join => "JOIN".to_string(),
            _ => {
                return Err(DbError::ParseError(format!(
                    "Expected function name after |>, got {:?}",
                    self.current_token()
                )));
            }
        };
        self.advance();
        Ok(name.to_uppercase())
    }

    /// Parse boolean OR expression
    pub(super) fn parse_or_expression(&mut self) -> DbResult<Expression> {
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

    /// Parse boolean AND expression
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

    /// Parse bitwise OR expression
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

    /// Parse bitwise XOR expression
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

    /// Parse bitwise AND expression
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

    /// Parse comparison expression
    pub(super) fn parse_comparison_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_range_expression()?;

        loop {
            // Array comparison: `arr ANY == x`, `arr ALL IN y`, `arr NONE > 1`,
            // `arr AT LEAST (n) == x`.
            if let Some(quantifier) = self.parse_array_quantifier()? {
                let Some(op) = self.parse_comparison_operator()? else {
                    return Err(DbError::ParseError(format!(
                        "Expected a comparison operator after the array quantifier, found {:?}",
                        self.current_token()
                    )));
                };
                if !matches!(
                    op,
                    BinaryOperator::Equal
                        | BinaryOperator::NotEqual
                        | BinaryOperator::LessThan
                        | BinaryOperator::LessThanOrEqual
                        | BinaryOperator::GreaterThan
                        | BinaryOperator::GreaterThanOrEqual
                        | BinaryOperator::In
                        | BinaryOperator::NotIn
                ) {
                    return Err(DbError::ParseError(format!(
                        "Array comparison supports ==, !=, <, <=, >, >=, IN and NOT IN, not {:?}",
                        op
                    )));
                }
                let right = self.parse_range_expression()?;
                left = Expression::ArrayComparison {
                    quantifier,
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                };
                continue;
            }

            let Some(op) = self.parse_comparison_operator()? else {
                break;
            };
            let right = self.parse_range_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Consume an array-comparison quantifier (`ANY`, `ALL`, `NONE`,
    /// `AT LEAST (n)`) if one follows an operand.
    ///
    /// `ANY` / `ALL` / `NONE` only count when a comparison operator comes
    /// next, so a traversal's `... TO @end ANY edges` and other uses of those
    /// words are left alone. `AT LEAST` is unambiguous on its own.
    fn parse_array_quantifier(&mut self) -> DbResult<Option<ArrayQuantifier>> {
        let simple = match self.current_token() {
            Token::Any => Some(ArrayQuantifier::Any),
            Token::Identifier(n) if n.eq_ignore_ascii_case("ALL") => Some(ArrayQuantifier::All),
            Token::Identifier(n) if n.eq_ignore_ascii_case("NONE") => Some(ArrayQuantifier::None),
            _ => None,
        };
        if let Some(quantifier) = simple {
            if self.comparison_operator_at(1) {
                self.advance();
                return Ok(Some(quantifier));
            }
            return Ok(None);
        }

        if self.ident_eq("AT")
            && matches!(self.peek_token(1), Token::Identifier(n) if n.eq_ignore_ascii_case("LEAST"))
        {
            self.advance(); // AT
            self.advance(); // LEAST
            self.expect(Token::LeftParen)?;
            let count = self.with_in_allowed(|p| p.parse_expression())?;
            self.expect(Token::RightParen)?;
            return Ok(Some(ArrayQuantifier::AtLeast(Box::new(count))));
        }
        Ok(None)
    }

    /// True when the token at `offset` starts a comparison operator that
    /// `parse_comparison_operator` would accept here.
    fn comparison_operator_at(&self, offset: usize) -> bool {
        match self.peek_token(offset) {
            Token::Equal
            | Token::NotEqual
            | Token::LessThan
            | Token::LessThanEq
            | Token::GreaterThan
            | Token::GreaterThanEq => true,
            Token::In => self.allow_in_operator,
            Token::Not => {
                self.allow_in_operator && matches!(self.peek_token(offset + 1), Token::In)
            }
            _ => false,
        }
    }

    /// Parse range expressions (e.g., 1..5 produces [1, 2, 3, 4, 5])
    pub(super) fn parse_range_expression(&mut self) -> DbResult<Expression> {
        let left = self.parse_shift_expression()?;

        if matches!(self.current_token(), Token::DotDot) {
            self.advance(); // consume '..'
            let right = self.parse_shift_expression()?;
            Ok(Expression::Range(Box::new(left), Box::new(right)))
        } else {
            Ok(left)
        }
    }

    /// Parse shift expression
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

    /// Parse additive expression (+, -)
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

    /// Parse multiplicative expression (*, /, %)
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

    /// Parse unary expression (!, -, ~)
    ///
    /// Chained unary operators recurse *here*, without going back through
    /// `parse_expression`, so this is the one recursive production the
    /// SEC-130 depth guard did not reach: `RETURN` followed by 200k `-`
    /// characters (200 KB, well under the body limit) recursed 200k frames
    /// and overflowed the stack. A Rust stack overflow is not catchable — it
    /// aborts the whole process, taking every other connection with it. The
    /// depth accounting below puts these frames on the same budget as every
    /// other nesting construct.
    pub(super) fn parse_unary_expression(&mut self) -> DbResult<Expression> {
        let op = match self.current_token() {
            Token::Not => UnaryOperator::Not,
            Token::Minus => UnaryOperator::Negate,
            Token::Tilde => UnaryOperator::BitwiseNot,
            _ => return self.parse_postfix_expression(),
        };

        self.advance();
        self.check_depth()?;
        let operand = self.parse_unary_expression();
        // Decrement before propagating: an early `?` here would leak depth
        // and make later sibling expressions in the same query fail.
        self.leave_depth();

        Ok(Expression::UnaryOp {
            op,
            operand: Box::new(operand?),
        })
    }
}
