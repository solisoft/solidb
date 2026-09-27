//! Developer-helper functions through the parser and executor, for the parts
//! the module unit tests cannot reach: TRY's lazy arguments and budget
//! handling, and the lambda forms (MIN_BY / MAX_BY, KEY_BY / COUNT_BY,
//! MAP_VALUES / MAP_KEYS / FILTER_KEYS). Also a smoke test that every
//! function added with DATE_SERIES parses and runs by name.

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

#[test]
fn key_by_and_count_by_take_a_lambda() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        execute_single(
            &engine,
            r#"LET m = KEY_BY([{id: 1, n: "a"}, {id: 2, n: "b"}], u -> CONCAT("u", u.id))
               RETURN m.u2.n"#
        ),
        json!("b")
    );
    assert_eq!(
        execute_single(
            &engine,
            r#"RETURN ["apple", "avocado", "banana"] |> COUNT_BY(w -> LEFT(w, 1))"#
        ),
        json!({"a": 2, "b": 1})
    );
}

#[test]
fn object_lambdas() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        execute_single(&engine, r#"RETURN MAP_VALUES({a: 1, b: 2}, v -> v * 10)"#),
        json!({"a": 10, "b": 20})
    );
    assert_eq!(
        execute_single(
            &engine,
            r#"RETURN MAP_VALUES({a: 1, b: 2}, (v, k) -> CONCAT(k, v))"#
        ),
        json!({"a": "a1", "b": "b2"})
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN MAP_KEYS({a: 1, b: 2}, k -> UPPER(k))"#),
        json!({"A": 1, "B": 2})
    );
    assert_eq!(
        execute_single(
            &engine,
            r#"RETURN {_key: "x", _rev: "1", name: "n", n: 3} |> FILTER_KEYS(k -> !STARTS_WITH(k, "_"))"#
        ),
        json!({"name": "n", "n": 3})
    );
    assert_eq!(
        execute_single(
            &engine,
            r#"LET min = 2 RETURN FILTER_KEYS({a: 1, b: 2, c: 3}, (k, v) -> v >= min)"#
        ),
        json!({"b": 2, "c": 3})
    );
    assert_eq!(
        execute_single(&engine, r#"RETURN MAP_VALUES(null, v -> v)"#),
        json!(null)
    );
    let q = parse(r#"RETURN MAP_VALUES([1, 2], v -> v)"#).unwrap();
    assert!(QueryExecutor::new(&engine).execute(&q).is_err());
}

#[test]
fn every_new_function_runs_by_name() {
    let (engine, _tmp) = create_test_engine();
    let cases = [
        (
            r#"RETURN LENGTH(DATE_SERIES("2024-01-01", "2024-01-07", "day"))"#,
            json!(7),
        ),
        (
            r#"RETURN DATE_END_OF("2024-02-10", "month")"#,
            json!("2024-02-29T23:59:59.999Z"),
        ),
        (r#"RETURN UNACCENT("Élodie")"#, json!("Elodie")),
        (
            r#"RETURN DIFF({a: 1}, {a: 2})"#,
            json!({"a": {"old": 1, "new": 2}}),
        ),
        (r#"RETURN ROUND(2.5, 0, "half_even")"#, json!(2.0)),
        (
            r#"RETURN IS_IBAN("FR7630006000011234567890189")"#,
            json!(true),
        ),
        (r#"RETURN LUHN("79927398713")"#, json!(true)),
        (r#"RETURN IS_SIREN("732829320")"#, json!(true)),
        (r#"RETURN IS_SIRET("73282932000074")"#, json!(true)),
        (
            r#"RETURN PARSE_URL("https://x.io/p?a=1").params.a"#,
            json!("1"),
        ),
        (r#"RETURN QUERY_STRING({a: 1})"#, json!("a=1")),
        (r#"RETURN HUMAN_BYTES(2048, true)"#, json!("2 KiB")),
        (r#"RETURN SPLIT_PART("a-b-c", "-", 2)"#, json!("b")),
        (r#"RETURN MODE([1, 2, 2])"#, json!(2)),
        (r#"RETURN PAIRWISE([1, 2, 3])"#, json!([[1, 2], [2, 3]])),
        (
            r#"RETURN TRANSPOSE([[1, 2], [3, 4]])"#,
            json!([[1, 3], [2, 4]]),
        ),
        (r#"RETURN LENGTH(SHUFFLE([1, 2, 3]))"#, json!(3)),
        (r#"RETURN GCD(12, 18)"#, json!(6)),
        (r#"RETURN LCM(4, 6)"#, json!(12)),
        (r#"RETURN HYPOT(3, 4)"#, json!(5.0)),
        (r#"RETURN CBRT(8)"#, json!(2.0)),
        (r#"RETURN KEY_BY([{k: "a"}], "k").a.k"#, json!("a")),
        (r#"RETURN COUNT_BY(["x", "x"])"#, json!({"x": 2})),
        (r#"RETURN mode([3, 3])"#, json!(3)),
    ];
    for (q, want) in cases {
        assert_eq!(execute_single(&engine, q), want, "{q}");
    }
}

#[test]
fn date_series_fills_the_days_without_rows() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("orders".to_string(), None)
        .unwrap();
    let orders = engine.get_collection("orders").unwrap();
    for at in [
        "2024-03-01T09:00:00Z",
        "2024-03-01T15:00:00Z",
        "2024-03-03T10:00:00Z",
    ] {
        orders.insert(json!({"at": at})).unwrap();
    }
    let q = parse(
        r#"LET counts = COUNT_BY((FOR o IN orders RETURN o), o -> DATE_TRUNC(o.at, "day"))
           FOR d IN DATE_SERIES("2024-03-01", "2024-03-04", "day")
             RETURN {d: LEFT(d, 10), n: counts[d] || 0}"#,
    )
    .unwrap();
    let rows = QueryExecutor::new(&engine).execute(&q).unwrap();
    assert_eq!(
        rows,
        vec![
            json!({"d": "2024-03-01", "n": 2}),
            json!({"d": "2024-03-02", "n": 0}),
            json!({"d": "2024-03-03", "n": 1}),
            json!({"d": "2024-03-04", "n": 0}),
        ]
    );
}

/// `FOR x IN <identifier>…` used to take the identifier as the whole source,
/// so a function call, attribute path, index, range, pipeline or `??` after
/// `IN` failed to parse. A bare identifier is still a collection or variable.
#[test]
fn for_accepts_expression_sources_that_start_with_an_identifier() {
    let (engine, _tmp) = create_test_engine();
    let cases = [
        (
            r#"FOR d IN DATE_SERIES("2024-01-01", "2024-01-03", "day") RETURN LEFT(d, 10)"#,
            json!(["2024-01-01", "2024-01-02", "2024-01-03"]),
        ),
        (
            r#"LET doc = {tags: ["a", "b"]} FOR t IN doc.tags RETURN t"#,
            json!(["a", "b"]),
        ),
        (
            r#"LET doc = {tags: ["a"]} FOR t IN doc?.tags RETURN t"#,
            json!(["a"]),
        ),
        (
            r#"LET rows = [[1, 2], [3]] FOR x IN rows[0] RETURN x"#,
            json!([1, 2]),
        ),
        (r#"LET n = 3 FOR i IN n..5 RETURN i"#, json!([3, 4, 5])),
        (
            r#"LET l = [3, 1, 2] FOR x IN l |> SORTED() RETURN x"#,
            json!([1, 2, 3]),
        ),
        (r#"LET m = null FOR x IN m ?? [9] RETURN x"#, json!([9])),
        (r#"LET m = null FOR x IN m || [7] RETURN x"#, json!([7])),
        (r#"LET l = [1, 2] FOR x IN l RETURN x"#, json!([1, 2])),
        (r#"FOR x IN LENGTH([1, 2]) RETURN x"#, json!([2])),
    ];
    for (q, want) in cases {
        let parsed = parse(q).unwrap_or_else(|e| panic!("{q}: {e}"));
        let rows = QueryExecutor::new(&engine)
            .execute(&parsed)
            .unwrap_or_else(|e| panic!("{q}: {e}"));
        assert_eq!(serde_json::Value::Array(rows), want, "{q}");
    }
}
