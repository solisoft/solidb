use crate::error::{DbError, DbResult};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // SQL Keywords
    Select,
    From,
    Where,
    Insert,
    Into,
    Values,
    Update,
    Set,
    Delete,
    Create,
    Drop,
    Table,
    
    // Clauses
    OrderBy,
    Order,
    By,
    GroupBy,
    Group,
    Having,
    Limit,
    Offset,
    As,
    
    // Joins
    Join,
    Left,
    Right,
    Inner,
    Outer,
    On,
    
    // Logical
    And,
    Or,
    Not,
    
    // Comparison
    Is,
    Null,
    Between,
    Like,
    In,
    
    // Aggregates
    Count,
    Sum,
    Avg,
    Min,
    Max,
    
    // Boolean
    True,
    False,
    
    // Sort direction
    Asc,
    Desc,
    
    // Literals and identifiers
    Identifier(String),
    Integer(i64),
    Float(f64),
    String(String),
    Placeholder(String), // ? or :name for bind parameters
    
    // Operators
    Equal,         // =
    NotEqual,      // != or <>
    LessThan,      // <
    LessThanEq,    // <=
    GreaterThan,   // >
    GreaterThanEq, // >=
    Plus,          // +
    Minus,         // -
    Star,          // *
    Slash,         // /
    Percent,       // %
    
    // Delimiters
    Comma,        // ,
    Dot,          // .
    LeftParen,    // (
    RightParen,   // )
    Semicolon,    // ;
    
    // Special
    Eof,
}

pub struct SqlLexer {
    input: Vec<char>,
    position: usize,
    current_char: Option<char>,
}

impl SqlLexer {
    pub fn new(input: &str) -> Self {
        let chars: Vec<char> = input.chars().collect();
        let current_char = chars.first().copied();
        
        Self {
            input: chars,
            position: 0,
            current_char,
        }
    }
    
    fn advance(&mut self) {
        self.position += 1;
        self.current_char = self.input.get(self.position).copied();
    }
    
    fn peek(&self) -> Option<char> {
        self.input.get(self.position + 1).copied()
    }
    
    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current_char {
            if ch.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }
    
    fn skip_line_comment(&mut self) {
        // Skip -- comments
        while let Some(ch) = self.current_char {
            if ch == '\n' {
                self.advance();
                break;
            }
            self.advance();
        }
    }
    
    fn skip_block_comment(&mut self) {
        // Skip /* */ comments
        self.advance(); // skip *
        self.advance(); // skip /
        while let Some(ch) = self.current_char {
            if ch == '*' && self.peek() == Some('/') {
                self.advance();
                self.advance();
                break;
            }
            self.advance();
        }
    }
    
    fn read_number(&mut self) -> DbResult<Token> {
        let mut num_str = String::new();
        let mut has_dot = false;
        
        while let Some(ch) = self.current_char {
            if ch.is_numeric() {
                num_str.push(ch);
                self.advance();
            } else if ch == '.' && !has_dot {
                // Check if next char is numeric (decimal point vs end of statement)
                if let Some(next) = self.peek() {
                    if next.is_numeric() {
                        has_dot = true;
                        num_str.push(ch);
                        self.advance();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        
        if has_dot {
            num_str
                .parse::<f64>()
                .map(Token::Float)
                .map_err(|_| DbError::ParseError(format!("Invalid float number: {}", num_str)))
        } else {
            num_str
                .parse::<i64>()
                .map(Token::Integer)
                .map_err(|_| DbError::ParseError(format!("Invalid integer number: {}", num_str)))
        }
    }
    
    fn read_string(&mut self) -> DbResult<Token> {
        let quote = self.current_char.unwrap();
        self.advance(); // Skip opening quote
        
        let mut string = String::new();
        
        while let Some(ch) = self.current_char {
            if ch == quote {
                // Check for escaped quote (doubled)
                if self.peek() == Some(quote) {
                    string.push(quote);
                    self.advance();
                    self.advance();
                } else {
                    self.advance(); // Skip closing quote
                    return Ok(Token::String(string));
                }
            } else if ch == '\\' {
                self.advance();
                if let Some(escaped) = self.current_char {
                    string.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '\\' => '\\',
                        '\'' => '\'',
                        '"' => '"',
                        _ => escaped,
                    });
                    self.advance();
                }
            } else {
                string.push(ch);
                self.advance();
            }
        }
        
        Err(DbError::ParseError("Unterminated string".to_string()))
    }
    
    fn read_identifier(&mut self) -> Token {
        let mut ident = String::new();
        
        while let Some(ch) = self.current_char {
            if ch.is_alphanumeric() || ch == '_' {
                ident.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        
        // Check for keywords (case-insensitive)
        match ident.to_uppercase().as_str() {
            "SELECT" => Token::Select,
            "FROM" => Token::From,
            "WHERE" => Token::Where,
            "INSERT" => Token::Insert,
            "INTO" => Token::Into,
            "VALUES" => Token::Values,
            "UPDATE" => Token::Update,
            "SET" => Token::Set,
            "DELETE" => Token::Delete,
            "CREATE" => Token::Create,
            "DROP" => Token::Drop,
            "TABLE" => Token::Table,
            "ORDER" => Token::Order,
            "BY" => Token::By,
            "GROUP" => Token::Group,
            "HAVING" => Token::Having,
            "LIMIT" => Token::Limit,
            "OFFSET" => Token::Offset,
            "AS" => Token::As,
            "JOIN" => Token::Join,
            "LEFT" => Token::Left,
            "RIGHT" => Token::Right,
            "INNER" => Token::Inner,
            "OUTER" => Token::Outer,
            "ON" => Token::On,
            "AND" => Token::And,
            "OR" => Token::Or,
            "NOT" => Token::Not,
            "IS" => Token::Is,
            "NULL" => Token::Null,
            "BETWEEN" => Token::Between,
            "LIKE" => Token::Like,
            "IN" => Token::In,
            "COUNT" => Token::Count,
            "SUM" => Token::Sum,
            "AVG" => Token::Avg,
            "MIN" => Token::Min,
            "MAX" => Token::Max,
            "TRUE" => Token::True,
            "FALSE" => Token::False,
            "ASC" => Token::Asc,
            "DESC" => Token::Desc,
            _ => Token::Identifier(ident),
        }
    }
    
    fn read_quoted_identifier(&mut self) -> DbResult<Token> {
        let quote = self.current_char.unwrap(); // " or `
        self.advance(); // Skip opening quote
        
        let mut ident = String::new();
        let closing = if quote == '[' { ']' } else { quote };
        
        while let Some(ch) = self.current_char {
            if ch == closing {
                self.advance();
                return Ok(Token::Identifier(ident));
            }
            ident.push(ch);
            self.advance();
        }
        
        Err(DbError::ParseError("Unterminated quoted identifier".to_string()))
    }
    
    fn read_placeholder(&mut self) -> DbResult<Token> {
        if self.current_char == Some('?') {
            self.advance();
            return Ok(Token::Placeholder("?".to_string()));
        }
        
        // :name style
        self.advance(); // skip :
        let mut name = String::new();
        
        while let Some(ch) = self.current_char {
            if ch.is_alphanumeric() || ch == '_' {
                name.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        
        if name.is_empty() {
            return Err(DbError::ParseError("Expected placeholder name after ':'".to_string()));
        }
        
        Ok(Token::Placeholder(name))
    }
    
    pub fn next_token(&mut self) -> DbResult<Token> {
        loop {
            self.skip_whitespace();
            
            match self.current_char {
                None => return Ok(Token::Eof),
                
                // Comments
                Some('-') if self.peek() == Some('-') => {
                    self.skip_line_comment();
                    continue;
                }
                Some('/') if self.peek() == Some('*') => {
                    self.skip_block_comment();
                    continue;
                }
                
                _ => break,
            }
        }
        
        let token = match self.current_char {
            None => Token::Eof,
            
            Some(ch) if ch.is_numeric() => {
                return self.read_number();
            }
            
            Some('\'') | Some('"') => {
                return self.read_string();
            }
            
            Some('`') | Some('[') => {
                return self.read_quoted_identifier();
            }
            
            Some(ch) if ch.is_alphabetic() || ch == '_' => {
                return Ok(self.read_identifier());
            }
            
            Some('?') | Some(':') => {
                return self.read_placeholder();
            }
            
            Some('=') => {
                self.advance();
                Token::Equal
            }
            
            Some('!') => {
                self.advance();
                if self.current_char == Some('=') {
                    self.advance();
                    Token::NotEqual
                } else {
                    Token::Not
                }
            }
            
            Some('<') => {
                self.advance();
                if self.current_char == Some('=') {
                    self.advance();
                    Token::LessThanEq
                } else if self.current_char == Some('>') {
                    self.advance();
                    Token::NotEqual // <>
                } else {
                    Token::LessThan
                }
            }
            
            Some('>') => {
                self.advance();
                if self.current_char == Some('=') {
                    self.advance();
                    Token::GreaterThanEq
                } else {
                    Token::GreaterThan
                }
            }
            
            Some('+') => {
                self.advance();
                Token::Plus
            }
            Some('-') => {
                self.advance();
                Token::Minus
            }
            Some('*') => {
                self.advance();
                Token::Star
            }
            Some('/') => {
                self.advance();
                Token::Slash
            }
            Some('%') => {
                self.advance();
                Token::Percent
            }
            Some(',') => {
                self.advance();
                Token::Comma
            }
            Some('.') => {
                self.advance();
                Token::Dot
            }
            Some('(') => {
                self.advance();
                Token::LeftParen
            }
            Some(')') => {
                self.advance();
                Token::RightParen
            }
            Some(';') => {
                self.advance();
                Token::Semicolon
            }
            
            Some(ch) => {
                return Err(DbError::ParseError(format!("Unexpected character: {}", ch)));
            }
        };
        
        Ok(token)
    }
    
    pub fn tokenize(&mut self) -> DbResult<Vec<Token>> {
        let mut tokens = Vec::new();
        
        loop {
            let token = self.next_token()?;
            if token == Token::Eof {
                tokens.push(token);
                break;
            }
            tokens.push(token);
        }
        
        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn tokenize(input: &str) -> Vec<Token> {
        SqlLexer::new(input).tokenize().unwrap()
    }
    
    #[test]
    fn test_select_keywords() {
        let tokens = tokenize("SELECT FROM WHERE");
        assert_eq!(tokens[0], Token::Select);
        assert_eq!(tokens[1], Token::From);
        assert_eq!(tokens[2], Token::Where);
    }
    
    #[test]
    fn test_case_insensitive() {
        assert_eq!(tokenize("select")[0], Token::Select);
        assert_eq!(tokenize("SELECT")[0], Token::Select);
        assert_eq!(tokenize("Select")[0], Token::Select);
    }
    
    #[test]
    fn test_identifiers() {
        assert_eq!(tokenize("users")[0], Token::Identifier("users".to_string()));
        assert_eq!(tokenize("my_table")[0], Token::Identifier("my_table".to_string()));
    }
    
    #[test]
    fn test_strings() {
        assert_eq!(tokenize("'hello'")[0], Token::String("hello".to_string()));
    }
    
    #[test]
    fn test_numbers() {
        assert_eq!(tokenize("123")[0], Token::Integer(123));
        assert_eq!(tokenize("3.14")[0], Token::Float(3.14));
    }
    
    #[test]
    fn test_operators() {
        assert_eq!(tokenize("=")[0], Token::Equal);
        assert_eq!(tokenize("!=")[0], Token::NotEqual);
        assert_eq!(tokenize("<>")[0], Token::NotEqual);
        assert_eq!(tokenize("<")[0], Token::LessThan);
        assert_eq!(tokenize("<=")[0], Token::LessThanEq);
        assert_eq!(tokenize(">")[0], Token::GreaterThan);
        assert_eq!(tokenize(">=")[0], Token::GreaterThanEq);
    }
    
    #[test]
    fn test_simple_select() {
        let tokens = tokenize("SELECT * FROM users WHERE age > 18");
        assert_eq!(tokens[0], Token::Select);
        assert_eq!(tokens[1], Token::Star);
        assert_eq!(tokens[2], Token::From);
        assert_eq!(tokens[3], Token::Identifier("users".to_string()));
        assert_eq!(tokens[4], Token::Where);
        assert_eq!(tokens[5], Token::Identifier("age".to_string()));
        assert_eq!(tokens[6], Token::GreaterThan);
        assert_eq!(tokens[7], Token::Integer(18));
    }
    
    #[test]
    fn test_comments() {
        let tokens = tokenize("SELECT -- this is a comment\n* FROM users");
        assert_eq!(tokens[0], Token::Select);
        assert_eq!(tokens[1], Token::Star);
        assert_eq!(tokens[2], Token::From);
    }
}
