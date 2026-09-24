//! TRY, DATE_PARSE, MIN_BY / MAX_BY, SET_PATH / UNSET_PATH and NUMBER_FORMAT
//! through the parser and executor, for the parts the module unit tests
//! cannot reach: TRY's lazy arguments and budget handling, and the lambda
//! form of MIN_BY / MAX_BY.

mod common;
use common::{create_test_engine, execute_single};
use serde_json::json;
use solidb::{parse, QueryExecutor};

#[test]
fn try_returns_the_value_or_the_fallback() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(execute_single(&engine, "RETURN TRY(1 + 1, 0)"), json!(2));
    assert_eq!(
        execute_single(
            &engine,
            r#"RETURN TRY(DATE_PARSE("nope", "%d/%m/%Y"), "n/a")"#
        ),
        json!("n/a")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN TRY(ASSERT(false, "boom"))"#),
        json!(null)
    );
}

#[test]
fn try_evaluates_the_fallback_only_on_error() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        execute_single(&engine, r#"RETURN TRY(1, ASSERT(false, "not lazy"))"#),
        json!(1)
    );
    // A failing fallback is not caught by the TRY it belongs to.
    let q = parse(r#"RETURN TRY(ASSERT(false), ASSERT(false, "fallback"))"#).unwrap();
    let err = QueryExecutor::new(&engine).execute(&q).unwrap_err();
    assert!(err.to_string().contains("fallback"), "{err}");
}

#[test]
fn try_keeps_one_bad_row_from_failing_the_query() {
    let (engine, _tmp) = create_test_engine();
    let rows = execute_single(
        &engine,
        r#"RETURN (FOR s IN ["01/02/2024", "garbage", "31/12/2024"]
                   RETURN TRY(DATE_PARSE(s, "%d/%m/%Y")))"#,
    );
    assert_eq!(
        rows,
        json!(["2024-02-01T00:00:00.000Z", null, "2024-12-31T00:00:00.000Z"])
    );
}

#[test]
fn try_does_not_swallow_the_row_budget() {
    let (engine, _tmp) = create_test_engine();
    let q = parse("RETURN TRY((FOR i IN 1..100 FOR j IN 1..100 RETURN 1), [])").unwrap();
    let err = QueryExecutor::new(&engine)
        .with_max_intermediate_rows(50)
        .execute(&q)
        .unwrap_err();
    assert!(err.to_string().contains("intermediate row limit"), "{err}");
}

#[test]
fn try_rejects_bad_arity() {
    let (engine, _tmp) = create_test_engine();
    let q = parse("RETURN TRY(1, 2, 3)").unwrap();
    assert!(QueryExecutor::new(&engine).execute(&q).is_err());
}

#[test]
fn min_by_and_max_by_with_a_lambda() {
    let (engine, _tmp) = create_test_engine();
    let items = r#"[{n: "a", p: 3}, {n: "b", p: 1}, {n: "c"}, {n: "d", p: 9}, {n: "e", p: 1}]"#;
    assert_eq!(
        execute_single(&engine, &format!("RETURN MIN_BY({items}, x -> x.p).n")),
        json!("b"),
        "null keys are skipped and the first of a tie wins"
    );
    assert_eq!(
        execute_single(&engine, &format!("RETURN MAX_BY({items}, x -> x.p).n")),
        json!("d")
    );
    assert_eq!(
        execute_single(
            &engine,
            r#"RETURN ([{n: "a", p: 3}, {n: "b", p: 1}] |> MAX_BY(x -> -x.p)).n"#
        ),
        json!("b")
    );
    assert_eq!(
        execute_single(&engine, "RETURN MIN_BY([], x -> x.p)"),
        json!(null)
    );
}

#[test]
fn min_by_and_max_by_with_a_path() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        execute_single(
            &engine,
            r#"RETURN MAX_BY([{n: "a", s: {v: 2}}, {n: "b", s: {v: 7}}], "s.v").n"#
        ),
        json!("b")
    );
}

#[test]
fn set_path_and_unset_path() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        execute_single(
            &engine,
            r#"LET d = {a: {b: 1}} RETURN [SET_PATH(d, "a.c.d", 2), d]"#
        ),
        json!([{"a": {"b": 1, "c": {"d": 2}}}, {"a": {"b": 1}}]),
        "the input is not modified"
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN UNSET_PATH({a: {b: 1, c: 2}}, "a.b")"#),
        json!({"a": {"c": 2}})
    );
}

#[test]
fn number_format_in_a_query() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        execute_single(&engine, r#"RETURN NUMBER_FORMAT(1234567.891, 2)"#),
        json!("1,234,567.89")
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN NUMBER_FORMAT(1234.5, 2, "de")"#),
        json!("1.234,50")
    );
}
