use serde_json::Value;

use crate::error::{DbError, DbResult};
use super::ast::*;
use super::lexer::{Lexer, Token};

pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    pub fn new(input: &str) -> DbResult<Self> {
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize()?;

        Ok(Self {
            tokens,
            position: 0,
        })
    }

    fn current_token(&self) -> &Token {
        self.tokens.get(self.position).unwrap_or(&Token::Eof)
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

        // Parse FOR, FILTER, INSERT, and additional LET clauses (they can be interleaved in AQL)
        loop {
            if matches!(self.current_token(), Token::For) {
                let for_clause = self.parse_for_clause()?;
                for_clauses.push(for_clause.clone());
                body_clauses.push(BodyClause::For(for_clause));
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
            } else if matches!(self.current_token(), Token::Let) {
                let let_clause = self.parse_let_clause()?;
                // LET after FOR goes to body_clauses (correlated), not let_clauses
                body_clauses.push(BodyClause::Let(let_clause));
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
            let has_mutation = body_clauses.iter().any(|c| matches!(c,
                BodyClause::Insert(_) | BodyClause::Update(_) | BodyClause::Remove(_)));

            if return_clause.is_none() && !has_mutation {
                // Check if there are unexpected tokens
                if !matches!(self.current_token(), Token::Eof) {
                    return Err(DbError::ParseError(format!(
                        "Unexpected token: {:?}. Expected FOR, LET, RETURN, INSERT, UPDATE, or REMOVE",
                        self.current_token()
                    )));
                }
                return Err(DbError::ParseError(
                    "Invalid query: missing RETURN clause or mutation (INSERT/UPDATE/REMOVE)".to_string()
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

    fn parse_let_clause(&mut self) -> DbResult<LetClause> {
        self.expect(Token::Let)?;

        let variable = if let Token::Identifier(name) = self.current_token() {
            let var = name.clone();
            self.advance();
            var
        } else {
            return Err(DbError::ParseError("Expected variable name after LET".to_string()));
        };

        self.expect(Token::Assign)?;

        let expression = self.parse_expression()?;

        Ok(LetClause { variable, expression })
    }

    fn parse_for_clause(&mut self) -> DbResult<ForClause> {
        self.expect(Token::For)?;

        let variable = if let Token::Identifier(name) = self.current_token() {
            let var = name.clone();
            self.advance();
            var
        } else {
            return Err(DbError::ParseError("Expected variable name after FOR".to_string()));
        };

        self.expect(Token::In)?;

        // Check if the source is an identifier (collection/variable) or an expression (e.g., range)
        if let Token::Identifier(name) = self.current_token() {
            let n = name.clone();
            self.advance();

            // We'll determine at execution time whether this is a collection or a LET variable
            Ok(ForClause {
                variable,
                collection: n.clone(),
                source_variable: Some(n),
                source_expression: None,
            })
        } else {
            // Parse as expression (e.g., 1..5, [1, 2, 3], etc.)
            let expr = self.parse_expression()?;
            Ok(ForClause {
                variable,
                collection: String::new(), // No collection for expression sources
                source_variable: None,
                source_expression: Some(expr),
            })
        }
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
            return Err(DbError::ParseError("Expected collection name after INTO".to_string()));
        };

        Ok(InsertClause { document, collection })
    }

    fn parse_update_clause(&mut self) -> DbResult<UpdateClause> {
        self.expect(Token::Update)?;

        // Parse the document selector (usually a variable like `doc` or `doc._key`)
        let selector = self.parse_expression()?;

        // Expect WITH keyword
        self.expect(Token::With)?;

        // Parse the changes (object expression)
        let changes = self.parse_expression()?;

        // Expect IN keyword
        self.expect(Token::In)?;

        // Parse collection name
        let collection = if let Token::Identifier(name) = self.current_token() {
            let coll = name.clone();
            self.advance();
            coll
        } else {
            return Err(DbError::ParseError("Expected collection name after IN".to_string()));
        };

        Ok(UpdateClause { selector, changes, collection })
    }

    fn parse_remove_clause(&mut self) -> DbResult<RemoveClause> {
        self.expect(Token::Remove)?;

        // Parse the document selector (usually a variable like `doc` or `doc._key`)
        let selector = self.parse_expression()?;

        // Expect IN keyword
        self.expect(Token::In)?;

        // Parse collection name
        let collection = if let Token::Identifier(name) = self.current_token() {
            let coll = name.clone();
            self.advance();
            coll
        } else {
            return Err(DbError::ParseError("Expected collection name after IN".to_string()));
        };

        Ok(RemoveClause { selector, collection })
    }

    fn parse_sort_clause(&mut self) -> DbResult<SortClause> {
        self.expect(Token::Sort)?;

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

        Ok(SortClause { expression, ascending })
    }

    fn parse_limit_clause(&mut self) -> DbResult<LimitClause> {
        self.expect(Token::Limit)?;

        let first = if let Token::Number(n) = self.current_token() {
            let num = *n as usize;
            self.advance();
            num
        } else {
            return Err(DbError::ParseError("Expected number after LIMIT".to_string()));
        };

        // Check for offset, count syntax
        if matches!(self.current_token(), Token::Comma) {
            self.advance();

            let count = if let Token::Number(n) = self.current_token() {
                let num = *n as usize;
                self.advance();
                num
            } else {
                return Err(DbError::ParseError("Expected count after comma in LIMIT".to_string()));
            };

            Ok(LimitClause { offset: first, count })
        } else {
            Ok(LimitClause { offset: 0, count: first })
        }
    }

    fn parse_return_clause(&mut self) -> DbResult<ReturnClause> {
        self.expect(Token::Return)?;
        let expression = self.parse_expression()?;
        Ok(ReturnClause { expression })
    }

    fn parse_expression(&mut self) -> DbResult<Expression> {
        self.parse_or_expression()
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
        let mut left = self.parse_comparison_expression()?;

        while matches!(self.current_token(), Token::And) {
            self.advance();
            let right = self.parse_comparison_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op: BinaryOperator::And,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_comparison_expression(&mut self) -> DbResult<Expression> {
        let mut left = self.parse_range_expression()?;

        while let Some(op) = self.parse_comparison_operator() {
            self.advance();
            let right = self.parse_range_expression()?;
            left = Expression::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_comparison_operator(&self) -> Option<BinaryOperator> {
        match self.current_token() {
            Token::Equal => Some(BinaryOperator::Equal),
            Token::NotEqual => Some(BinaryOperator::NotEqual),
            Token::LessThan => Some(BinaryOperator::LessThan),
            Token::LessThanEq => Some(BinaryOperator::LessThanOrEqual),
            Token::GreaterThan => Some(BinaryOperator::GreaterThan),
            Token::GreaterThanEq => Some(BinaryOperator::GreaterThanOrEqual),
            _ => None,
        }
    }

    /// Parse range expressions (e.g., 1..5 produces [1, 2, 3, 4, 5])
    fn parse_range_expression(&mut self) -> DbResult<Expression> {
        let left = self.parse_additive_expression()?;

        if matches!(self.current_token(), Token::DotDot) {
            self.advance(); // consume '..'
            let right = self.parse_additive_expression()?;
            Ok(Expression::Range(Box::new(left), Box::new(right)))
        } else {
            Ok(left)
        }
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

        while matches!(self.current_token(), Token::Star | Token::Slash) {
            let op = match self.current_token() {
                Token::Star => BinaryOperator::Multiply,
                Token::Slash => BinaryOperator::Divide,
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
                        return Err(DbError::ParseError("Expected field name after '.'".to_string()));
                    }
                }
                Token::LeftBracket => {
                    self.advance();
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
                            expr = Expression::DynamicFieldAccess(Box::new(expr), Box::new(index_expr));
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

            Token::Number(n) => {
                let num = *n;
                self.advance();
                Ok(Expression::Literal(Value::Number(
                    serde_json::Number::from_f64(num).unwrap()
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

            Token::LeftBrace => {
                self.parse_object_expression()
            }

            Token::LeftBracket => {
                self.parse_array_expression()
            }

            Token::LeftParen => {
                self.advance();
                // Check if this is a subquery (starts with FOR or LET)
                if matches!(self.current_token(), Token::For | Token::Let) {
                    let subquery = self.parse_query(false)?;  // Don't check trailing for subqueries
                    self.expect(Token::RightParen)?;
                    Ok(Expression::Subquery(Box::new(subquery)))
                } else {
                    let expr = self.parse_expression()?;
                    self.expect(Token::RightParen)?;
                    Ok(expr)
                }
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
                return Err(DbError::ParseError("Expected field name in object".to_string()));
            };

            self.expect(Token::Colon)?;
            let value = self.parse_expression()?;

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

    fn parse_field_path(&mut self) -> DbResult<String> {
        let mut path = String::new();

        if let Token::Identifier(name) = self.current_token() {
            path.push_str(name);
            self.advance();
        } else {
            return Err(DbError::ParseError("Expected field path".to_string()));
        }

        while matches!(self.current_token(), Token::Dot) {
            self.advance();

            if let Token::Identifier(field) = self.current_token() {
                path.push('.');
                path.push_str(field);
                self.advance();
            } else {
                return Err(DbError::ParseError("Expected field name after '.'".to_string()));
            }
        }

        Ok(path)
    }
}

pub fn parse(input: &str) -> DbResult<Query> {
    let mut parser = Parser::new(input)?;
    parser.parse()
}
