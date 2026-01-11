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
    Upsert,
    With,
    Remove,
    Replace,

    // Aggregation keywords
    Collect,
    Aggregate,
    Count,

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
    Percent,       // %
    Like,          // LIKE
    RegEx,         // =~
    NotRegEx,      // !~

    // Bitwise operators
    Ampersand,  // &
    Pipe,       // |
    Caret,      // ^
    Tilde,      // ~
    LeftShift,  // <<
    RightShift, // >>

    // Pipeline operators
    PipeRight,    // |> (pipeline)
    Arrow,        // -> (lambda)
    NullCoalesce, // ?? (null coalescing)

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
    Question,     // ?

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

    fn read_quoted_identifier(&mut self) -> DbResult<Token> {
        self.advance(); // Skip opening backtick

        let mut ident = String::new();

        while let Some(ch) = self.current_char {
            if ch == '`' {
                self.advance(); // Skip closing backtick
                return Ok(Token::Identifier(ident));
            }
            ident.push(ch);
            self.advance();
        }

        Err(DbError::ParseError(
            "Unterminated quoted identifier".to_string(),
        ))
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
            "LIKE" => Token::Like,
            "TRUE" => Token::True,
            "FALSE" => Token::False,
            "NULL" => Token::Null,
            "INSERT" => Token::Insert,
            "INTO" => Token::Into,
            "UPDATE" => Token::Update,
            "UPSERT" => Token::Upsert,
            "WITH" => Token::With,
            "REMOVE" => Token::Remove,
            "REPLACE" => Token::Replace,
            // Graph traversal keywords
            "OUTBOUND" => Token::Outbound,
            "INBOUND" => Token::Inbound,
            "ANY" => Token::Any,
            "SHORTEST_PATH" => Token::ShortestPath,
            "GRAPH" => Token::Graph,
            "TO" => Token::To,
            // Aggregation keywords
            "COLLECT" => Token::Collect,
            "AGGREGATE" => Token::Aggregate,
            "COUNT" => Token::Count,
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

            Some('`') => {
                return self.read_quoted_identifier();
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
                } else if self.current_char == Some('~') {
                    self.advance();
                    Token::RegEx
                } else {
                    Token::Assign
                }
            }

            Some('!') => {
                self.advance();
                if self.current_char == Some('=') {
                    self.advance();
                    Token::NotEqual
                } else if self.current_char == Some('~') {
                    self.advance();
                    Token::NotRegEx
                } else {
                    Token::Not
                }
            }

            Some('<') => {
                self.advance();
                if self.current_char == Some('=') {
                    self.advance();
                    Token::LessThanEq
                } else if self.current_char == Some('<') {
                    self.advance();
                    Token::LeftShift
                } else {
                    Token::LessThan
                }
            }

            Some('>') => {
                self.advance();
                if self.current_char == Some('=') {
                    self.advance();
                    Token::GreaterThanEq
                } else if self.current_char == Some('>') {
                    self.advance();
                    Token::RightShift
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
                if self.current_char == Some('>') {
                    self.advance();
                    Token::Arrow
                } else {
                    Token::Minus
                }
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
            Some('&') => {
                self.advance();
                Token::Ampersand
            }
            Some('|') => {
                self.advance();
                if self.current_char == Some('>') {
                    self.advance();
                    Token::PipeRight
                } else {
                    Token::Pipe
                }
            }
            Some('^') => {
                self.advance();
                Token::Caret
            }
            Some('~') => {
                self.advance();
                Token::Tilde
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
            Some('?') => {
                self.advance();
                if self.current_char == Some('?') {
                    self.advance();
                    Token::NullCoalesce
                } else {
                    Token::Question
                }
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
        Lexer::new(input).tokenize().unwrap()
    }

    #[test]
    fn test_keywords() {
        let tokens = tokenize("FOR IN FILTER SORT LIMIT RETURN LET ASC DESC");
        assert_eq!(tokens[0], Token::For);
        assert_eq!(tokens[1], Token::In);
        assert_eq!(tokens[2], Token::Filter);
        assert_eq!(tokens[3], Token::Sort);
        assert_eq!(tokens[4], Token::Limit);
        assert_eq!(tokens[5], Token::Return);
        assert_eq!(tokens[6], Token::Let);
        assert_eq!(tokens[7], Token::Asc);
        assert_eq!(tokens[8], Token::Desc);
    }

    #[test]
    fn test_keywords_case_insensitive() {
        assert_eq!(tokenize("for")[0], Token::For);
        assert_eq!(tokenize("FOR")[0], Token::For);
        assert_eq!(tokenize("For")[0], Token::For);
    }

    #[test]
    fn test_boolean_null() {
        assert_eq!(tokenize("TRUE")[0], Token::True);
        assert_eq!(tokenize("FALSE")[0], Token::False);
        assert_eq!(tokenize("NULL")[0], Token::Null);
    }

    #[test]
    fn test_logical_operators() {
        assert_eq!(tokenize("AND")[0], Token::And);
        assert_eq!(tokenize("OR")[0], Token::Or);
        assert_eq!(tokenize("NOT")[0], Token::Not);
    }

    #[test]
    fn test_graph_keywords() {
        assert_eq!(tokenize("OUTBOUND")[0], Token::Outbound);
        assert_eq!(tokenize("INBOUND")[0], Token::Inbound);
        assert_eq!(tokenize("ANY")[0], Token::Any);
        assert_eq!(tokenize("GRAPH")[0], Token::Graph);
    }

    #[test]
    fn test_identifiers() {
        assert_eq!(tokenize("myVar")[0], Token::Identifier("myVar".to_string()));
        assert_eq!(
            tokenize("_private")[0],
            Token::Identifier("_private".to_string())
        );
        assert_eq!(
            tokenize("var123")[0],
            Token::Identifier("var123".to_string())
        );
    }

    #[test]
    fn test_quoted_identifier() {
        assert_eq!(
            tokenize("`my field`")[0],
            Token::Identifier("my field".to_string())
        );
    }

    #[test]
    fn test_bind_variables() {
        assert_eq!(tokenize("@name")[0], Token::BindVar("name".to_string()));
        assert_eq!(tokenize("@_id")[0], Token::BindVar("_id".to_string()));
        assert_eq!(tokenize("@var123")[0], Token::BindVar("var123".to_string()));
    }

    #[test]
    fn test_integers() {
        assert_eq!(tokenize("123")[0], Token::Integer(123));
        assert_eq!(tokenize("0")[0], Token::Integer(0));
        assert_eq!(tokenize("999999")[0], Token::Integer(999999));
    }

    #[test]
    fn test_floats() {
        assert_eq!(tokenize("3.14")[0], Token::Float(3.14));
        assert_eq!(tokenize("0.5")[0], Token::Float(0.5));
        assert_eq!(tokenize("100.0")[0], Token::Float(100.0));
    }

    #[test]
    fn test_strings() {
        assert_eq!(tokenize("\"hello\"")[0], Token::String("hello".to_string()));
        assert_eq!(tokenize("'world'")[0], Token::String("world".to_string()));
        assert_eq!(tokenize("\"\"")[0], Token::String("".to_string()));
    }

    #[test]
    fn test_string_escapes() {
        assert_eq!(
            tokenize("\"hello\\nworld\"")[0],
            Token::String("hello\nworld".to_string())
        );
        assert_eq!(
            tokenize("\"tab\\there\"")[0],
            Token::String("tab\there".to_string())
        );
        assert_eq!(
            tokenize("\"quote\\\"here\"")[0],
            Token::String("quote\"here".to_string())
        );
    }

    #[test]
    fn test_comparison_operators() {
        assert_eq!(tokenize("==")[0], Token::Equal);
        assert_eq!(tokenize("!=")[0], Token::NotEqual);
        assert_eq!(tokenize("<")[0], Token::LessThan);
        assert_eq!(tokenize("<=")[0], Token::LessThanEq);
        assert_eq!(tokenize(">")[0], Token::GreaterThan);
        assert_eq!(tokenize(">=")[0], Token::GreaterThanEq);
        assert_eq!(tokenize("=")[0], Token::Assign);
    }

    #[test]
    fn test_regex_operators() {
        assert_eq!(tokenize("=~")[0], Token::RegEx);
        assert_eq!(tokenize("!~")[0], Token::NotRegEx);
    }

    #[test]
    fn test_arithmetic_operators() {
        assert_eq!(tokenize("+")[0], Token::Plus);
        assert_eq!(tokenize("-")[0], Token::Minus);
        assert_eq!(tokenize("*")[0], Token::Star);
        assert_eq!(tokenize("/")[0], Token::Slash);
        assert_eq!(tokenize("%")[0], Token::Percent);
    }

    #[test]
    fn test_delimiters() {
        assert_eq!(tokenize(".")[0], Token::Dot);
        assert_eq!(tokenize("..")[0], Token::DotDot);
        assert_eq!(tokenize(",")[0], Token::Comma);
        assert_eq!(tokenize("{")[0], Token::LeftBrace);
        assert_eq!(tokenize("}")[0], Token::RightBrace);
        assert_eq!(tokenize("[")[0], Token::LeftBracket);
        assert_eq!(tokenize("]")[0], Token::RightBracket);
        assert_eq!(tokenize("(")[0], Token::LeftParen);
        assert_eq!(tokenize(")")[0], Token::RightParen);
        assert_eq!(tokenize(":")[0], Token::Colon);
        assert_eq!(tokenize("?")[0], Token::Question);
        assert_eq!(tokenize("??")[0], Token::NullCoalesce);
    }

    #[test]
    fn test_eof() {
        let tokens = tokenize("");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Eof);
    }

    #[test]
    fn test_whitespace_handling() {
        let tokens = tokenize("  FOR   IN  ");
        assert_eq!(tokens[0], Token::For);
        assert_eq!(tokens[1], Token::In);
    }

    #[test]
    fn test_complete_query() {
        let query = "FOR doc IN users FILTER doc.age > 18 RETURN doc";
        let tokens = tokenize(query);

        assert_eq!(tokens[0], Token::For);
        assert_eq!(tokens[1], Token::Identifier("doc".to_string()));
        assert_eq!(tokens[2], Token::In);
        assert_eq!(tokens[3], Token::Identifier("users".to_string()));
        assert_eq!(tokens[4], Token::Filter);
    }

    #[test]
    fn test_number_before_range() {
        // "1..10" should tokenize as Integer(1), DotDot, Integer(10)
        let tokens = tokenize("1..10");
        assert_eq!(tokens[0], Token::Integer(1));
        assert_eq!(tokens[1], Token::DotDot);
        assert_eq!(tokens[2], Token::Integer(10));
    }

    #[test]
    fn test_error_unterminated_string() {
        let result = Lexer::new("\"unterminated").tokenize();
        assert!(result.is_err());
    }

    #[test]
    fn test_error_empty_bind_var() {
        let result = Lexer::new("@").tokenize();
        assert!(result.is_err());
    }

    #[test]
    fn test_error_unexpected_char() {
        let result = Lexer::new("$").tokenize();
        assert!(result.is_err());
    }
}
