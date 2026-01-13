#[cfg(test)]
mod tests {
    use crate::sdbql::parser::Parser;

    #[test]
    fn test_any_syntax() {
        let query = "FOR doc IN collection FILTER ANY member IN doc.members RETURN doc";
        let mut parser = Parser::new(query).unwrap();
        let result = parser.parse();
        assert!(result.is_ok(), "Failed to parse ANY syntax: {:?}", result.err());
    }

    #[test]
    fn test_any_satisfies_syntax() {
        let query = "FOR doc IN collection FILTER ANY member IN doc.members SATISFIES member.age > 10 RETURN doc";
        let mut parser = Parser::new(query).unwrap();
        let result = parser.parse();
        assert!(result.is_ok(), "Failed to parse ANY ... SATISFIES syntax: {:?}", result.err());
    }
}
