//! A request may fail; it may not take the server with it.
//!
//! These pin the bounds that stop one query from exhausting memory, and the
//! panics that used to be reachable from a query string alone.

mod common;
use common::{create_test_engine, execute_query, execute_query_expect_err, execute_single};
use serde_json::json;
use solidb::{parse, QueryExecutor};

#[test]
fn slice_past_the_end_is_empty_not_a_panic() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(
        execute_single(&engine, "RETURN SLICE([1,2,3], 10)"),
        json!([])
    );
    assert_eq!(
        execute_single(&engine, "RETURN SLICE([1,2,3], 10, 0)"),
        json!([])
    );
    assert_eq!(
        execute_single(
            &engine,
            "RETURN SLICE([1], 9223372036854775807, 9223372036854775807)"
        ),
        json!([])
    );
    assert_eq!(
        execute_single(&engine, "RETURN SLICE([1,2,3], 1, 5)"),
        json!([2, 3])
    );
}

#[test]
fn out_of_range_float_literal_is_a_parse_error() {
    let q = format!("RETURN 1{}.0", "0".repeat(400));
    let err = parse(&q).unwrap_err();
    assert!(err.to_string().contains("out of range"), "{err}");
}

#[test]
fn range_over_the_full_i64_span_is_rejected() {
    let (engine, _tmp) = create_test_engine();
    let err = execute_query_expect_err(
        &engine,
        "RETURN RANGE(-9223372036854775807, 9223372036854775807)",
    );
    assert!(err.contains("max"), "{err}");
    assert_eq!(
        execute_single(&engine, "RETURN RANGE(1, 3)"),
        json!([1, 2, 3])
    );
    assert_eq!(
        execute_single(&engine, "RETURN RANGE(3, 1, -1)"),
        json!([3, 2, 1])
    );
}

#[test]
fn pad_rejects_before_allocating() {
    let (engine, _tmp) = create_test_engine();
    let started = std::time::Instant::now();
    let err = execute_query_expect_err(&engine, r#"RETURN PAD_LEFT("x", 4000000000)"#);
    assert!(err.contains("1 MiB"), "{err}");
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

fn seeded(n: usize) -> (solidb::storage::StorageEngine, tempfile::TempDir) {
    let (engine, tmp) = create_test_engine();
    engine.create_collection("big".to_string(), None).unwrap();
    let big = engine.get_collection("big").unwrap();
    for i in 0..n {
        big.insert(json!({"_key": i.to_string(), "k": i % 3, "v": i}))
            .unwrap();
    }
    (engine, tmp)
}

fn run_capped(
    engine: &solidb::storage::StorageEngine,
    cap: usize,
    q: &str,
) -> Result<Vec<serde_json::Value>, String> {
    QueryExecutor::new(engine)
        .with_max_intermediate_rows(cap)
        .execute(&parse(q).unwrap())
        .map_err(|e| e.to_string())
}

#[test]
fn direct_scan_fast_path_honours_the_ceiling() {
    let (engine, _tmp) = seeded(20);
    let err = run_capped(&engine, 5, "FOR d IN big RETURN d").unwrap_err();
    assert!(err.contains("intermediate row limit (6 > 5)"), "{err}");
    assert_eq!(
        run_capped(&engine, 5, "FOR d IN big LIMIT 3 RETURN d")
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        run_capped(&engine, 20, "FOR d IN big RETURN d")
            .unwrap()
            .len(),
        20
    );
}

#[test]
fn sorted_index_fast_path_honours_the_ceiling() {
    let (engine, _tmp) = seeded(20);
    let big = engine.get_collection("big").unwrap();
    big.create_index(
        "v_idx".to_string(),
        vec!["v".to_string()],
        solidb::storage::IndexType::Persistent,
        false,
    )
    .unwrap();
    let err = run_capped(&engine, 5, "FOR d IN big SORT d.v RETURN d.v").unwrap_err();
    assert!(err.contains("intermediate row limit"), "{err}");
    let ok = run_capped(&engine, 5, "FOR d IN big SORT d.v DESC LIMIT 2 RETURN d.v").unwrap();
    assert_eq!(ok, vec![json!(19), json!(18)]);
}

#[test]
fn set_operations_accumulate_against_the_ceiling() {
    let (engine, _tmp) = create_test_engine();
    let q =
        "FOR i IN 1..4 RETURN i UNION ALL FOR i IN 1..4 RETURN i UNION ALL FOR i IN 1..4 RETURN i";
    let err = run_capped(&engine, 10, q).unwrap_err();
    assert!(err.contains("intermediate row limit"), "{err}");
    assert_eq!(run_capped(&engine, 12, q).unwrap().len(), 12);
}

#[test]
fn streaming_bulk_insert_with_return_is_bounded() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("bulk".to_string(), None).unwrap();
    let err = run_capped(
        &engine,
        6000,
        "FOR i IN 1..10000 INSERT {v: i} INTO bulk RETURN i",
    )
    .unwrap_err();
    assert!(err.contains("intermediate row limit"), "{err}");
    // Without a RETURN nothing accumulates, so the insert itself is fine.
    run_capped(&engine, 6000, "FOR i IN 1..10000 INSERT {v: i} INTO bulk").unwrap();
    assert_eq!(engine.get_collection("bulk").unwrap().count(), 10000);
}

#[test]
fn join_right_side_is_bounded_by_the_ceiling() {
    let (engine, _tmp) = seeded(20);
    engine.create_collection("small".to_string(), None).unwrap();
    let small = engine.get_collection("small").unwrap();
    small.insert(json!({"_key": "a", "k": 1})).unwrap();
    let q = "FOR s IN small JOIN big ON big.k == s.k RETURN LENGTH(big)";
    let err = run_capped(&engine, 5, q).unwrap_err();
    assert!(err.contains("intermediate row limit"), "{err}");
    assert_eq!(run_capped(&engine, 50, q).unwrap(), vec![json!(7)]);
}

#[test]
fn k_paths_with_an_absurd_max_does_not_overflow_the_stack() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("cities".to_string(), None)
        .unwrap();
    engine
        .create_collection("roads".to_string(), Some("edge".to_string()))
        .unwrap();
    let cities = engine.get_collection("cities").unwrap();
    let roads = engine.get_collection("roads").unwrap();
    let names: Vec<String> = (0..80).map(|i| format!("c{i}")).collect();
    for n in &names {
        cities.insert(json!({"_key": n})).unwrap();
    }
    for w in names.windows(2) {
        roads
            .insert(json!({"_from": format!("cities/{}", w[0]), "_to": format!("cities/{}", w[1])}))
            .unwrap();
    }
    let q = r#"FOR v, e, p IN K_PATHS "cities/c0" TO "cities/c79" OUTBOUND roads OPTIONS { max: 5000000 } RETURN p.weight"#;
    // 79 hops is past the depth clamp, so no path is found; the point is
    // that the query returns at all.
    let results = execute_query(&engine, q);
    assert!(results.is_empty());
    let q = r#"FOR v, e, p IN K_PATHS "cities/c0" TO "cities/c10" OUTBOUND roads OPTIONS { max: 5000000 } RETURN p.weight"#;
    assert_eq!(execute_query(&engine, q), vec![json!(10.0)]);
}
