use crate::error::{DbError, DbResult};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    For,
    In,
    Filter,
    Sort,
    Limit,
    Return,
    Let,
    Asc,
    Desc,
    And,
    Or,
    Not,
    True,
    False,
    Null,
    Insert,
    Into,
    Update,
    With,
    Remove,

    // Graph traversal keywords
    Outbound,
    Inbound,
    Any,
    ShortestPath,
    Graph,
    To,

    // Identifiers and literals
    Identifier(String),
    BindVar(String), // @variable for bind parameters
    Integer(i64),
    Float(f64),
    String(String),

    // Operators
    Equal,         // ==
    Assign,        // =
    NotEqual,      // !=
    LessThan,      // <
    LessThanEq,    // <=
    GreaterThan,   // >
    GreaterThanEq, // >=
    Plus,          // +
    Minus,         // -
    Star,          // *
    Slash,         // /

    // Delimiters
    Dot,          // .
    DotDot,       // .. (range operator)
    Comma,        // ,
    LeftBrace,    // {
    RightBrace,   // }
    LeftBracket,  // [
    RightBracket, // ]
    LeftParen,    // (
    RightParen,   // )
    Colon,        // :

    // Special
    Eof,
}

pub struct Lexer {
    input: Vec<char>,
    position: usize,
    current_char: Option<char>,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        let chars: Vec<char> = input.chars().collect();
        let current_char = chars.get(0).copied();

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



    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current_char {
            if ch.is_whitespace() {
                self.advance();
            } else {
                break;
            }
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
                // Check if this is a decimal point or start of range operator (..)
                let next = self.input.get(self.position + 1).copied();
                if next == Some('.') {
                    // This is a range operator (..), stop reading number
                    break;
                }
                // It's a decimal point
                has_dot = true;
                num_str.push(ch);
                self.advance();
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
                self.advance(); // Skip closing quote
                return Ok(Token::String(string));
            } else if ch == '\\' {
                self.advance();
                if let Some(escaped) = self.current_char {
                    string.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '\\' => '\\',
                        '"' => '"',
                        '\'' => '\'',
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

        // Check for keywords
        match ident.to_uppercase().as_str() {
            "FOR" => Token::For,
            "IN" => Token::In,
            "FILTER" => Token::Filter,
            "SORT" => Token::Sort,
            "LIMIT" => Token::Limit,
            "RETURN" => Token::Return,
            "LET" => Token::Let,
            "ASC" => Token::Asc,
            "DESC" => Token::Desc,
            "AND" => Token::And,
            "OR" => Token::Or,
            "NOT" => Token::Not,
            "TRUE" => Token::True,
            "FALSE" => Token::False,
            "NULL" => Token::Null,
            "INSERT" => Token::Insert,
            "INTO" => Token::Into,
            "UPDATE" => Token::Update,
            "WITH" => Token::With,
            "REMOVE" => Token::Remove,
            // Graph traversal keywords
            "OUTBOUND" => Token::Outbound,
            "INBOUND" => Token::Inbound,
            "ANY" => Token::Any,
            "SHORTEST_PATH" => Token::ShortestPath,
            "GRAPH" => Token::Graph,
            "TO" => Token::To,
            _ => Token::Identifier(ident),
        }
    }

    fn read_bind_var(&mut self) -> DbResult<Token> {
        self.advance(); // Skip '@'

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
            return Err(DbError::ParseError(
                "Expected bind variable name after '@'".to_string(),
            ));
        }

        Ok(Token::BindVar(name))
    }

    pub fn next_token(&mut self) -> DbResult<Token> {
        self.skip_whitespace();

        let token = match self.current_char {
            None => Token::Eof,

            Some(ch) if ch.is_numeric() => {
                return self.read_number();
            }

            Some('"') | Some('\'') => {
                return self.read_string();
            }

            Some(ch) if ch.is_alphabetic() || ch == '_' => {
                return Ok(self.read_identifier());
            }

            Some('@') => {
                return self.read_bind_var();
            }

            Some('=') => {
                self.advance();
                if self.current_char == Some('=') {
                    self.advance();
                    Token::Equal
                } else {
                    Token::Assign
                }
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
            Some('.') => {
                self.advance();
                if self.current_char == Some('.') {
                    self.advance();
                    Token::DotDot
                } else {
                    Token::Dot
                }
            }
            Some(',') => {
                self.advance();
                Token::Comma
            }
            Some('{') => {
                self.advance();
                Token::LeftBrace
            }
            Some('}') => {
                self.advance();
                Token::RightBrace
            }
            Some('[') => {
                self.advance();
                Token::LeftBracket
            }
            Some(']') => {
                self.advance();
                Token::RightBracket
            }
            Some('(') => {
                self.advance();
                Token::LeftParen
            }
            Some(')') => {
                self.advance();
                Token::RightParen
            }
            Some(':') => {
                self.advance();
                Token::Colon
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
