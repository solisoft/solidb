//! Comparison and binary operator parsing for SDBQL.
//!
//! Handles operators:
//! - Equality: `==`, `!=`
//! - Relational: `<`, `<=`, `>`, `>=`
//! - Membership: `IN`, `NOT IN`
//! - Pattern matching: `LIKE`, `NOT LIKE`, `=~`, `!~`
//! - Fuzzy matching: `~=`

use crate::ast::BinaryOperator;
use crate::error::SdbqlResult;
use crate::lexer::Token;
use crate::parser::Parser;

impl Parser {
    /// Parse a comparison operator if present.
    /// Returns Some(operator) if found, None otherwise.
    pub(super) fn parse_comparison_operator(&mut self) -> SdbqlResult<Option<BinaryOperator>> {
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
            Token::FuzzyEqual => {
                self.advance();
                Ok(Some(BinaryOperator::FuzzyEqual))
            }
            Token::Not => self.parse_negated_operator(),
            _ => Ok(None),
        }
    }

    /// Parse negated operators: NOT LIKE, NOT REGEX, NOT IN
    fn parse_negated_operator(&mut self) -> SdbqlResult<Option<BinaryOperator>> {
        match self.peek_token(1) {
            Token::Like => {
                self.advance(); // consume NOT
                self.advance(); // consume LIKE
                Ok(Some(BinaryOperator::NotLike))
            }
            Token::RegEx => {
                self.advance(); // consume NOT
                self.advance(); // consume REGEX
                Ok(Some(BinaryOperator::NotRegEx))
            }
            Token::In => {
                if self.allow_in_operator {
                    self.advance(); // consume NOT
                    self.advance(); // consume IN
                    Ok(Some(BinaryOperator::NotIn))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }
}
