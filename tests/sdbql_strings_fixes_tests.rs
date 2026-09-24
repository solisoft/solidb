//! End-to-end checks for the string / crypto / JSON / type-check fixes from
//! the SDBQL functions audit (D1–D4, S3, X10, X12, X13) and the functions
//! added with them. Unit tests next to the code cover the edge cases; these
//! make sure each name is reachable through the query engine's dispatch.

mod common;
use common::{create_test_engine, execute_query_expect_err, execute_single};
use serde_json::json;

#[test]
fn highlight_empty_term_and_escaping() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        execute_single(&engine, r#"RETURN HIGHLIGHT("a<b>", "")"#),
        json!("a&lt;b&gt;")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN HIGHLIGHT("x & fox", "fox")"#),
        json!("x &amp; <b>fox</b>")
    );
}

#[test]
fn argon2_verify_param_policy() {
    let (engine, _tmp) = create_test_engine();
    let err = execute_query_expect_err(
        &engine,
        r#"RETURN ARGON2_VERIFY("$argon2id$v=19$m=131072,t=2,p=1$c29tZXNhbHQ$iWh06vD8Fy27wf9npn6FXWiCX4K6pW6Ue1Bnzz07Z8A", "pw")"#,
    );
    assert!(err.contains("policy"), "{}", err);
}

#[test]
fn decode_uri_multibyte_and_plus() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        execute_single(&engine, r#"RETURN DECODE_URI("%aé")"#),
        json!("%aé")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN DECODE_URI_COMPONENT("a+b")"#),
        json!("a+b")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN URL_DECODE("a+b")"#),
        json!("a b")
    );
}

#[test]
fn replacements_empty_search_and_single_pass() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        // REPLACE is also a statement keyword, so exercise the empty-search
        // rule through REGEX_REPLACE here (REPLACE is unit-tested).
        execute_single(&engine, r#"RETURN REGEX_REPLACE("abc", "", "x")"#),
        json!("abc")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN SUBSTITUTE("ab", {a: "b", b: "c"})"#),
        json!("bc")
    );
    assert_eq!(
        execute_single(
            &engine,
            r#"RETURN SUBSTITUTE("a-b_c", ["-", "_"], ["+", "="])"#
        ),
        json!("a+b=c")
    );
}

#[test]
fn like_function_null() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        execute_single(&engine, r#"RETURN LIKE(null, "%")"#),
        json!(null)
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN LIKE("a.c", "a.c")"#),
        json!(true)
    );
}

#[test]
fn split_concat_title_case() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        execute_single(&engine, r#"RETURN SPLIT("foo-bar-baz", "-", 1)"#),
        json!(["foo"])
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN SPLIT("a-b_c", ["-", "_"])"#),
        json!(["a", "b", "c"])
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN CONCAT_SEPARATOR(",", "a", null, "b")"#),
        json!("a,b")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN CONCAT("n=", 1 + 1)"#),
        json!("n=2")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN TITLE_CASE("hello  world")"#),
        json!("Hello  World")
    );
}

#[test]
fn new_string_functions_are_dispatched() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        execute_single(&engine, r#"RETURN REGEX_MATCH("abc", "^a")"#),
        json!(true)
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN ELLIPSIS("Hello World", 8)"#),
        json!("Hello...")
    );
    assert_eq!(
        execute_single(&engine, r##"RETURN MASK("secret", 0, -2, "#")"##),
        json!("####et")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN LTRIM("xxabc", "x")"#),
        json!("abc")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN REPEAT("ab", 3, "-")"#),
        json!("ab-ab-ab")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN SUBSTRING_BYTES("hello", 1, 3)"#),
        json!("ell")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN IPV4_TO_NUMBER("127.0.0.1")"#),
        json!(2130706433u32)
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN IPV4_FROM_NUMBER(2130706433)"#),
        json!("127.0.0.1")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN IS_IPV4("10.0.0.1")"#),
        json!(true)
    );
    let sim = execute_single(
        &engine,
        r#"RETURN NGRAM_SIMILARITY("quick fox", "quick foxx", 2)"#,
    );
    assert!((sim.as_f64().unwrap() - 8.0 / 9.0).abs() < 1e-9);
    let pos = execute_single(
        &engine,
        r#"RETURN NGRAM_POSITIONAL_SIMILARITY("quick fox", "quick foxx", 3)"#,
    );
    assert!((pos.as_f64().unwrap() - 0.875).abs() < 1e-9);
    assert_eq!(
        execute_single(&engine, r#"RETURN FIND_FIRST("foobarbaz", "ba", 0, 3)"#),
        json!(-1)
    );
}

#[test]
fn hashes_json_and_date_predicates() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        execute_single(&engine, r#"RETURN SHA1("foobar")"#),
        json!("8843d7f92416211de9ebb963ff4ce28125932878")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN CRC32("foobar")"#),
        json!("D5F5C7F")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN FNV64("foobar")"#),
        json!("85944171F73967E8")
    );
    assert_eq!(execute_single(&engine, r#"RETURN MD5(null)"#), json!(null));
    assert_eq!(
        execute_single(&engine, r#"RETURN MD5(1)"#),
        execute_single(&engine, r#"RETURN MD5("1")"#)
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN JSON_PARSE("{not json")"#),
        json!(null)
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN JSON_PARSE(null)"#),
        json!(null)
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN IS_DATETIME("2024-01-15T10:30:00Z")"#),
        json!(true)
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN IS_DATESTRING("2024-01-15")"#),
        json!(true)
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN IS_DATE(1700000000)"#),
        json!(false)
    );
}
