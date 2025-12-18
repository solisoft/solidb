use solidb::sdbql::lexer::{Lexer, Token};

#[test]
fn test_lexer_backtick_identifiers() {
    let input = "`my-collection-name`";
    let mut lexer = Lexer::new(input);
    let tokens = lexer.tokenize().unwrap();
    
    assert_eq!(tokens.len(), 2); // Identifier + EOF
    assert_eq!(tokens[0], Token::Identifier("my-collection-name".to_string()));
    assert_eq!(tokens[1], Token::Eof);
}

#[test]
fn test_lexer_backtick_with_spaces() {
    let input = "`Collection With Spaces`";
    let mut lexer = Lexer::new(input);
    let tokens = lexer.tokenize().unwrap();
    
    assert_eq!(tokens[0], Token::Identifier("Collection With Spaces".to_string()));
}

#[test]
fn test_lexer_backtick_in_query() {
    let input = "FOR doc IN `test-blob_s0` RETURN doc";
    let mut lexer = Lexer::new(input);
    let tokens = lexer.tokenize().unwrap();
    
    // Tokens: FOR, doc, IN, Identifier("test-blob_s0"), RETURN, doc, EOF
    assert_eq!(tokens[0], Token::For);
    assert_eq!(tokens[1], Token::Identifier("doc".to_string()));
    assert_eq!(tokens[2], Token::In);
    assert_eq!(tokens[3], Token::Identifier("test-blob_s0".to_string()));
    assert_eq!(tokens[4], Token::Return);
    assert_eq!(tokens[5], Token::Identifier("doc".to_string()));
}
