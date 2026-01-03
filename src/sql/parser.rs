use crate::error::{DbError, DbResult};
use super::lexer::{SqlLexer, Token};

/// SQL Abstract Syntax Tree types

#[derive(Debug, Clone, PartialEq)]
pub enum SqlStatement {
    Select(SelectStatement),
    Insert(InsertStatement),
    Update(UpdateStatement),
    Delete(DeleteStatement),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectStatement {
    pub columns: Vec<SelectColumn>,
    pub from: String,
    pub from_alias: Option<String>,
    pub joins: Vec<JoinClause>,
    pub where_clause: Option<SqlExpr>,
    pub group_by: Vec<String>,
    pub having: Option<SqlExpr>,
    pub order_by: Vec<OrderByItem>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JoinType {
    Inner,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JoinClause {
    pub join_type: JoinType,
    pub table: String,
    pub alias: Option<String>,
    pub on_condition: SqlExpr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectColumn {
    Star,
    Column {
        name: String,
        alias: Option<String>,
    },
    Function {
        name: String,
        args: Vec<SqlExpr>,
        alias: Option<String>,
    },
    Expression {
        expr: SqlExpr,
        alias: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderByItem {
    pub column: String,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertStatement {
    pub table: String,
    pub columns: Option<Vec<String>>,
    pub values: Vec<Vec<SqlExpr>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateStatement {
    pub table: String,
    pub assignments: Vec<(String, SqlExpr)>,
    pub where_clause: Option<SqlExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteStatement {
    pub table: String,
    pub where_clause: Option<SqlExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SqlExpr {
    Column(String),
    QualifiedColumn { table: String, column: String },
    Integer(i64),
    Float(f64),
    String(String),
    Boolean(bool),
    Null,
    Placeholder(String),
    
    // Binary operations
    BinaryOp {
        left: Box<SqlExpr>,
        op: BinaryOp,
        right: Box<SqlExpr>,
    },
    
    // Unary operations
    Not(Box<SqlExpr>),
    IsNull(Box<SqlExpr>),
    IsNotNull(Box<SqlExpr>),
    
    // Special
    Between {
        expr: Box<SqlExpr>,
        low: Box<SqlExpr>,
        high: Box<SqlExpr>,
    },
    InList {
        expr: Box<SqlExpr>,
        list: Vec<SqlExpr>,
    },
    
    // Function call
    Function {
        name: String,
        args: Vec<SqlExpr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
    Plus,
    Minus,
    Multiply,
    Divide,
    Modulo,
    Like,
}

/// SQL Parser
pub struct SqlParser {
    tokens: Vec<Token>,
    position: usize,
}

impl SqlParser {
    pub fn new(input: &str) -> DbResult<Self> {
        let mut lexer = SqlLexer::new(input);
        let tokens = lexer.tokenize()?;
        
        Ok(Self {
            tokens,
            position: 0,
        })
    }
    
    fn current_token(&self) -> &Token {
        self.tokens.get(self.position).unwrap_or(&Token::Eof)
    }
    
    fn peek_token(&self, offset: usize) -> &Token {
        self.tokens.get(self.position + offset).unwrap_or(&Token::Eof)
    }
    
    fn advance(&mut self) {
        if self.position < self.tokens.len() {
            self.position += 1;
        }
    }
    
    fn expect(&mut self, expected: Token) -> DbResult<()> {
        if *self.current_token() == expected {
            self.advance();
            Ok(())
        } else {
            Err(DbError::ParseError(format!(
                "Expected {:?}, found {:?}",
                expected,
                self.current_token()
            )))
        }
    }
    
    fn expect_identifier(&mut self) -> DbResult<String> {
        match self.current_token().clone() {
            Token::Identifier(name) => {
                self.advance();
                Ok(name)
            }
            other => Err(DbError::ParseError(format!(
                "Expected identifier, found {:?}",
                other
            ))),
        }
    }
    
    pub fn parse(&mut self) -> DbResult<SqlStatement> {
        let stmt = match self.current_token() {
            Token::Select => self.parse_select()?,
            Token::Insert => self.parse_insert()?,
            Token::Update => self.parse_update()?,
            Token::Delete => self.parse_delete()?,
            other => {
                return Err(DbError::ParseError(format!(
                    "Unexpected token at start of statement: {:?}",
                    other
                )));
            }
        };
        
        // Optional semicolon at end
        if *self.current_token() == Token::Semicolon {
            self.advance();
        }
        
        Ok(stmt)
    }
    
    fn parse_select(&mut self) -> DbResult<SqlStatement> {
        self.expect(Token::Select)?;
        
        // Parse columns
        let columns = self.parse_select_columns()?;
        
        // FROM clause
        self.expect(Token::From)?;
        let from = self.expect_identifier()?;
        
        // Optional alias
        let from_alias = if *self.current_token() == Token::As {
            self.advance();
            Some(self.expect_identifier()?)
        } else if let Token::Identifier(_) = self.current_token() {
            // Implicit alias without AS - but be careful not to consume JOIN keywords
            if !matches!(
                self.current_token(),
                Token::Where | Token::Order | Token::Group | Token::Limit | Token::Eof | Token::Semicolon |
                Token::Join | Token::Left | Token::Right | Token::Inner
            ) {
                Some(self.expect_identifier()?)
            } else {
                None
            }
        } else {
            None
        };
        
        // Parse JOIN clauses
        let joins = self.parse_join_clauses()?;
        
        // WHERE clause
        let where_clause = if *self.current_token() == Token::Where {
            self.advance();
            Some(self.parse_expression()?)
        } else {
            None
        };
        
        // GROUP BY clause
        let group_by = if *self.current_token() == Token::Group {
            self.advance();
            self.expect(Token::By)?;
            self.parse_identifier_list()?
        } else {
            Vec::new()
        };
        
        // HAVING clause
        let having = if *self.current_token() == Token::Having {
            self.advance();
            Some(self.parse_expression()?)
        } else {
            None
        };
        
        // ORDER BY clause
        let order_by = if *self.current_token() == Token::Order {
            self.advance();
            self.expect(Token::By)?;
            self.parse_order_by_list()?
        } else {
            Vec::new()
        };
        
        // LIMIT clause
        let limit = if *self.current_token() == Token::Limit {
            self.advance();
            match self.current_token() {
                Token::Integer(n) => {
                    let n = *n;
                    self.advance();
                    Some(n)
                }
                _ => return Err(DbError::ParseError("Expected integer after LIMIT".to_string())),
            }
        } else {
            None
        };
        
        // OFFSET clause
        let offset = if *self.current_token() == Token::Offset {
            self.advance();
            match self.current_token() {
                Token::Integer(n) => {
                    let n = *n;
                    self.advance();
                    Some(n)
                }
                _ => return Err(DbError::ParseError("Expected integer after OFFSET".to_string())),
            }
        } else {
            None
        };
        
        Ok(SqlStatement::Select(SelectStatement {
            columns,
            from,
            from_alias,
            joins,
            where_clause,
            group_by,
            having,
            order_by,
            limit,
            offset,
        }))
    }
    
    fn parse_join_clauses(&mut self) -> DbResult<Vec<JoinClause>> {
        let mut joins = Vec::new();
        
        loop {
            // Check for JOIN keyword (with optional INNER/LEFT/RIGHT prefix)
            let join_type = match self.current_token() {
                Token::Join => {
                    self.advance();
                    JoinType::Inner // Default to INNER
                }
                Token::Inner => {
                    self.advance();
                    self.expect(Token::Join)?;
                    JoinType::Inner
                }
                Token::Left => {
                    self.advance();
                    // Optional OUTER keyword
                    if *self.current_token() == Token::Outer {
                        self.advance();
                    }
                    self.expect(Token::Join)?;
                    JoinType::Left
                }
                Token::Right => {
                    self.advance();
                    // Optional OUTER keyword
                    if *self.current_token() == Token::Outer {
                        self.advance();
                    }
                    self.expect(Token::Join)?;
                    JoinType::Right
                }
                _ => break, // No more joins
            };
            
            // Table name
            let table = self.expect_identifier()?;
            
            // Optional alias
            let alias = if *self.current_token() == Token::As {
                self.advance();
                Some(self.expect_identifier()?)
            } else if let Token::Identifier(_) = self.current_token() {
                if !matches!(
                    self.current_token(),
                    Token::On | Token::Where | Token::Order | Token::Group | Token::Limit |
                    Token::Join | Token::Left | Token::Right | Token::Inner
                ) {
                    Some(self.expect_identifier()?)
                } else {
                    None
                }
            } else {
                None
            };
            
            // ON condition
            self.expect(Token::On)?;
            let on_condition = self.parse_expression()?;
            
            joins.push(JoinClause {
                join_type,
                table,
                alias,
                on_condition,
            });
        }
        
        Ok(joins)
    }
    
    fn parse_select_columns(&mut self) -> DbResult<Vec<SelectColumn>> {
        let mut columns = Vec::new();
        
        loop {
            let col = self.parse_select_column()?;
            columns.push(col);
            
            if *self.current_token() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        
        Ok(columns)
    }
    
    fn parse_select_column(&mut self) -> DbResult<SelectColumn> {
        // Check for *
        if *self.current_token() == Token::Star {
            self.advance();
            return Ok(SelectColumn::Star);
        }
        
        // Check for aggregate functions
        let func_name = match self.current_token() {
            Token::Count => Some("COUNT"),
            Token::Sum => Some("SUM"),
            Token::Avg => Some("AVG"),
            Token::Min => Some("MIN"),
            Token::Max => Some("MAX"),
            _ => None,
        };
        
        if let Some(name) = func_name {
            self.advance();
            self.expect(Token::LeftParen)?;
            
            let args = if *self.current_token() == Token::Star {
                self.advance();
                vec![SqlExpr::Column("*".to_string())]
            } else {
                self.parse_expression_list()?
            };
            
            self.expect(Token::RightParen)?;
            
            let alias = self.parse_optional_alias()?;
            
            return Ok(SelectColumn::Function {
                name: name.to_string(),
                args,
                alias,
            });
        }
        
        // Regular column or expression
        let name = self.expect_identifier()?;
        
        // Check for table.column
        let column_name = if *self.current_token() == Token::Dot {
            self.advance();
            let col = self.expect_identifier()?;
            format!("{}.{}", name, col)
        } else {
            name
        };
        
        let alias = self.parse_optional_alias()?;
        
        Ok(SelectColumn::Column {
            name: column_name,
            alias,
        })
    }
    
    fn parse_optional_alias(&mut self) -> DbResult<Option<String>> {
        if *self.current_token() == Token::As {
            self.advance();
            Ok(Some(self.expect_identifier()?))
        } else if let Token::Identifier(name) = self.current_token() {
            // Check if this could be an implicit alias (not a keyword)
            if !matches!(
                self.peek_token(0),
                Token::From | Token::Where | Token::Order | Token::Group | 
                Token::Limit | Token::Comma | Token::Eof | Token::Semicolon
            ) {
                let alias = name.clone();
                self.advance();
                Ok(Some(alias))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
    
    fn parse_identifier_list(&mut self) -> DbResult<Vec<String>> {
        let mut list = Vec::new();
        
        loop {
            let mut name = self.expect_identifier()?;
            
            // Handle qualified names (table.column)
            if *self.current_token() == Token::Dot {
                self.advance();
                let col = self.expect_identifier()?;
                name = format!("{}.{}", name, col);
            }
            
            list.push(name);
            
            if *self.current_token() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        
        Ok(list)
    }
    
    fn parse_order_by_list(&mut self) -> DbResult<Vec<OrderByItem>> {
        let mut items = Vec::new();
        
        loop {
            let column = self.expect_identifier()?;
            
            let descending = if *self.current_token() == Token::Desc {
                self.advance();
                true
            } else if *self.current_token() == Token::Asc {
                self.advance();
                false
            } else {
                false
            };
            
            items.push(OrderByItem { column, descending });
            
            if *self.current_token() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        
        Ok(items)
    }
    
    fn parse_expression_list(&mut self) -> DbResult<Vec<SqlExpr>> {
        let mut exprs = Vec::new();
        
        loop {
            exprs.push(self.parse_expression()?);
            
            if *self.current_token() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        
        Ok(exprs)
    }
    
    fn parse_expression(&mut self) -> DbResult<SqlExpr> {
        self.parse_or_expression()
    }
    
    fn parse_or_expression(&mut self) -> DbResult<SqlExpr> {
        let mut left = self.parse_and_expression()?;
        
        while *self.current_token() == Token::Or {
            self.advance();
            let right = self.parse_and_expression()?;
            left = SqlExpr::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::Or,
                right: Box::new(right),
            };
        }
        
        Ok(left)
    }
    
    fn parse_and_expression(&mut self) -> DbResult<SqlExpr> {
        let mut left = self.parse_not_expression()?;
        
        while *self.current_token() == Token::And {
            self.advance();
            let right = self.parse_not_expression()?;
            left = SqlExpr::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::And,
                right: Box::new(right),
            };
        }
        
        Ok(left)
    }
    
    fn parse_not_expression(&mut self) -> DbResult<SqlExpr> {
        if *self.current_token() == Token::Not {
            self.advance();
            let expr = self.parse_not_expression()?;
            return Ok(SqlExpr::Not(Box::new(expr)));
        }
        
        self.parse_comparison_expression()
    }
    
    fn parse_comparison_expression(&mut self) -> DbResult<SqlExpr> {
        let left = self.parse_additive_expression()?;
        
        // IS NULL / IS NOT NULL
        if *self.current_token() == Token::Is {
            self.advance();
            
            if *self.current_token() == Token::Not {
                self.advance();
                self.expect(Token::Null)?;
                return Ok(SqlExpr::IsNotNull(Box::new(left)));
            } else {
                self.expect(Token::Null)?;
                return Ok(SqlExpr::IsNull(Box::new(left)));
            }
        }
        
        // BETWEEN
        if *self.current_token() == Token::Between {
            self.advance();
            let low = self.parse_additive_expression()?;
            self.expect(Token::And)?;
            let high = self.parse_additive_expression()?;
            return Ok(SqlExpr::Between {
                expr: Box::new(left),
                low: Box::new(low),
                high: Box::new(high),
            });
        }
        
        // IN
        if *self.current_token() == Token::In {
            self.advance();
            self.expect(Token::LeftParen)?;
            let list = self.parse_expression_list()?;
            self.expect(Token::RightParen)?;
            return Ok(SqlExpr::InList {
                expr: Box::new(left),
                list,
            });
        }
        
        // LIKE
        if *self.current_token() == Token::Like {
            self.advance();
            let right = self.parse_additive_expression()?;
            return Ok(SqlExpr::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::Like,
                right: Box::new(right),
            });
        }
        
        // Regular comparison operators
        let op = match self.current_token() {
            Token::Equal => Some(BinaryOp::Eq),
            Token::NotEqual => Some(BinaryOp::NotEq),
            Token::LessThan => Some(BinaryOp::Lt),
            Token::LessThanEq => Some(BinaryOp::LtEq),
            Token::GreaterThan => Some(BinaryOp::Gt),
            Token::GreaterThanEq => Some(BinaryOp::GtEq),
            _ => None,
        };
        
        if let Some(op) = op {
            self.advance();
            let right = self.parse_additive_expression()?;
            return Ok(SqlExpr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            });
        }
        
        Ok(left)
    }
    
    fn parse_additive_expression(&mut self) -> DbResult<SqlExpr> {
        let mut left = self.parse_multiplicative_expression()?;
        
        loop {
            let op = match self.current_token() {
                Token::Plus => Some(BinaryOp::Plus),
                Token::Minus => Some(BinaryOp::Minus),
                _ => None,
            };
            
            if let Some(op) = op {
                self.advance();
                let right = self.parse_multiplicative_expression()?;
                left = SqlExpr::BinaryOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        
        Ok(left)
    }
    
    fn parse_multiplicative_expression(&mut self) -> DbResult<SqlExpr> {
        let mut left = self.parse_primary_expression()?;
        
        loop {
            let op = match self.current_token() {
                Token::Star => Some(BinaryOp::Multiply),
                Token::Slash => Some(BinaryOp::Divide),
                Token::Percent => Some(BinaryOp::Modulo),
                _ => None,
            };
            
            if let Some(op) = op {
                self.advance();
                let right = self.parse_primary_expression()?;
                left = SqlExpr::BinaryOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        
        Ok(left)
    }
    
    fn parse_primary_expression(&mut self) -> DbResult<SqlExpr> {
        match self.current_token().clone() {
            Token::Integer(n) => {
                self.advance();
                Ok(SqlExpr::Integer(n))
            }
            Token::Float(n) => {
                self.advance();
                Ok(SqlExpr::Float(n))
            }
            Token::String(s) => {
                self.advance();
                Ok(SqlExpr::String(s))
            }
            Token::True => {
                self.advance();
                Ok(SqlExpr::Boolean(true))
            }
            Token::False => {
                self.advance();
                Ok(SqlExpr::Boolean(false))
            }
            Token::Null => {
                self.advance();
                Ok(SqlExpr::Null)
            }
            Token::Placeholder(name) => {
                self.advance();
                Ok(SqlExpr::Placeholder(name))
            }
            Token::LeftParen => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(Token::RightParen)?;
                Ok(expr)
            }
            Token::Identifier(name) => {
                self.advance();
                
                // Check for function call
                if *self.current_token() == Token::LeftParen {
                    self.advance();
                    let args = if *self.current_token() == Token::RightParen {
                        Vec::new()
                    } else {
                        self.parse_expression_list()?
                    };
                    self.expect(Token::RightParen)?;
                    return Ok(SqlExpr::Function { name, args });
                }
                
                // Check for qualified column (table.column)
                if *self.current_token() == Token::Dot {
                    self.advance();
                    let column = self.expect_identifier()?;
                    return Ok(SqlExpr::QualifiedColumn {
                        table: name,
                        column,
                    });
                }
                
                Ok(SqlExpr::Column(name))
            }
            // Aggregate functions
            Token::Count | Token::Sum | Token::Avg | Token::Min | Token::Max => {
                let name = match self.current_token() {
                    Token::Count => "COUNT",
                    Token::Sum => "SUM",
                    Token::Avg => "AVG",
                    Token::Min => "MIN",
                    Token::Max => "MAX",
                    _ => unreachable!(),
                }.to_string();
                self.advance();
                
                self.expect(Token::LeftParen)?;
                let args = if *self.current_token() == Token::Star {
                    self.advance();
                    vec![SqlExpr::Column("*".to_string())]
                } else if *self.current_token() == Token::RightParen {
                    Vec::new()
                } else {
                    self.parse_expression_list()?
                };
                self.expect(Token::RightParen)?;
                
                Ok(SqlExpr::Function { name, args })
            }
            other => Err(DbError::ParseError(format!(
                "Unexpected token in expression: {:?}",
                other
            ))),
        }
    }
    
    fn parse_insert(&mut self) -> DbResult<SqlStatement> {
        self.expect(Token::Insert)?;
        self.expect(Token::Into)?;
        
        let table = self.expect_identifier()?;
        
        // Optional column list
        let columns = if *self.current_token() == Token::LeftParen {
            self.advance();
            let cols = self.parse_identifier_list()?;
            self.expect(Token::RightParen)?;
            Some(cols)
        } else {
            None
        };
        
        self.expect(Token::Values)?;
        
        // Parse value lists
        let mut values = Vec::new();
        loop {
            self.expect(Token::LeftParen)?;
            let row = self.parse_expression_list()?;
            self.expect(Token::RightParen)?;
            values.push(row);
            
            if *self.current_token() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        
        Ok(SqlStatement::Insert(InsertStatement {
            table,
            columns,
            values,
        }))
    }
    
    fn parse_update(&mut self) -> DbResult<SqlStatement> {
        self.expect(Token::Update)?;
        
        let table = self.expect_identifier()?;
        
        self.expect(Token::Set)?;
        
        // Parse assignments
        let mut assignments = Vec::new();
        loop {
            let column = self.expect_identifier()?;
            self.expect(Token::Equal)?;
            let value = self.parse_expression()?;
            assignments.push((column, value));
            
            if *self.current_token() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        
        // Optional WHERE clause
        let where_clause = if *self.current_token() == Token::Where {
            self.advance();
            Some(self.parse_expression()?)
        } else {
            None
        };
        
        Ok(SqlStatement::Update(UpdateStatement {
            table,
            assignments,
            where_clause,
        }))
    }
    
    fn parse_delete(&mut self) -> DbResult<SqlStatement> {
        self.expect(Token::Delete)?;
        self.expect(Token::From)?;
        
        let table = self.expect_identifier()?;
        
        // Optional WHERE clause
        let where_clause = if *self.current_token() == Token::Where {
            self.advance();
            Some(self.parse_expression()?)
        } else {
            None
        };
        
        Ok(SqlStatement::Delete(DeleteStatement {
            table,
            where_clause,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn parse(input: &str) -> SqlStatement {
        SqlParser::new(input).unwrap().parse().unwrap()
    }
    
    #[test]
    fn test_simple_select() {
        let stmt = parse("SELECT * FROM users");
        if let SqlStatement::Select(s) = stmt {
            assert_eq!(s.columns, vec![SelectColumn::Star]);
            assert_eq!(s.from, "users");
        } else {
            panic!("Expected SELECT statement");
        }
    }
    
    #[test]
    fn test_select_columns() {
        let stmt = parse("SELECT name, age FROM users");
        if let SqlStatement::Select(s) = stmt {
            assert_eq!(s.columns.len(), 2);
        } else {
            panic!("Expected SELECT statement");
        }
    }
    
    #[test]
    fn test_select_with_where() {
        let stmt = parse("SELECT * FROM users WHERE age > 18");
        if let SqlStatement::Select(s) = stmt {
            assert!(s.where_clause.is_some());
        } else {
            panic!("Expected SELECT statement");
        }
    }
    
    #[test]
    fn test_select_with_order_by() {
        let stmt = parse("SELECT * FROM users ORDER BY name ASC");
        if let SqlStatement::Select(s) = stmt {
            assert_eq!(s.order_by.len(), 1);
            assert_eq!(s.order_by[0].column, "name");
            assert!(!s.order_by[0].descending);
        } else {
            panic!("Expected SELECT statement");
        }
    }
    
    #[test]
    fn test_select_with_limit_offset() {
        let stmt = parse("SELECT * FROM users LIMIT 10 OFFSET 5");
        if let SqlStatement::Select(s) = stmt {
            assert_eq!(s.limit, Some(10));
            assert_eq!(s.offset, Some(5));
        } else {
            panic!("Expected SELECT statement");
        }
    }
    
    #[test]
    fn test_insert() {
        let stmt = parse("INSERT INTO users (name, age) VALUES ('Alice', 30)");
        if let SqlStatement::Insert(s) = stmt {
            assert_eq!(s.table, "users");
            assert_eq!(s.columns, Some(vec!["name".to_string(), "age".to_string()]));
            assert_eq!(s.values.len(), 1);
        } else {
            panic!("Expected INSERT statement");
        }
    }
    
    #[test]
    fn test_update() {
        let stmt = parse("UPDATE users SET age = 31 WHERE name = 'Alice'");
        if let SqlStatement::Update(s) = stmt {
            assert_eq!(s.table, "users");
            assert_eq!(s.assignments.len(), 1);
            assert!(s.where_clause.is_some());
        } else {
            panic!("Expected UPDATE statement");
        }
    }
    
    #[test]
    fn test_delete() {
        let stmt = parse("DELETE FROM users WHERE age < 18");
        if let SqlStatement::Delete(s) = stmt {
            assert_eq!(s.table, "users");
            assert!(s.where_clause.is_some());
        } else {
            panic!("Expected DELETE statement");
        }
    }
    
    #[test]
    fn test_complex_where() {
        let stmt = parse("SELECT * FROM users WHERE age > 18 AND status = 'active' OR role = 'admin'");
        if let SqlStatement::Select(s) = stmt {
            assert!(s.where_clause.is_some());
        } else {
            panic!("Expected SELECT statement");
        }
    }
    
    #[test]
    fn test_like() {
        let stmt = parse("SELECT * FROM users WHERE name LIKE 'A%'");
        if let SqlStatement::Select(s) = stmt {
            if let Some(SqlExpr::BinaryOp { op, .. }) = s.where_clause {
                assert_eq!(op, BinaryOp::Like);
            } else {
                panic!("Expected LIKE expression");
            }
        } else {
            panic!("Expected SELECT statement");
        }
    }
    
    #[test]
    fn test_in_list() {
        let stmt = parse("SELECT * FROM users WHERE status IN ('active', 'pending')");
        if let SqlStatement::Select(s) = stmt {
            if let Some(SqlExpr::InList { list, .. }) = s.where_clause {
                assert_eq!(list.len(), 2);
            } else {
                panic!("Expected IN list expression");
            }
        } else {
            panic!("Expected SELECT statement");
        }
    }
    
    #[test]
    fn test_is_null() {
        let stmt = parse("SELECT * FROM users WHERE email IS NULL");
        if let SqlStatement::Select(s) = stmt {
            assert!(matches!(s.where_clause, Some(SqlExpr::IsNull(_))));
        } else {
            panic!("Expected SELECT statement");
        }
    }
    
    #[test]
    fn test_aggregate_function() {
        let stmt = parse("SELECT COUNT(*) FROM users");
        if let SqlStatement::Select(s) = stmt {
            if let SelectColumn::Function { name, .. } = &s.columns[0] {
                assert_eq!(name, "COUNT");
            } else {
                panic!("Expected function column");
            }
        } else {
            panic!("Expected SELECT statement");
        }
    }
}
