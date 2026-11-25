//! EXPLAIN, QUALIFY, COLLECT ROLLUP / CUBE, and FOR SAMPLE.

mod common;

use std::collections::HashSet;

use common::{create_test_engine, execute_query, execute_with_binds};
use serde_json::{json, Value};
use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};

fn try_query(engine: &StorageEngine, query_str: &str) -> Result<Vec<Value>, String> {
    let query = parse(query_str).map_err(|e| e.to_string())?;
    QueryExecutor::new(engine)
        .execute(&query)
        .map_err(|e| e.to_string())
}

fn insert_orders(engine: &StorageEngine) {
    engine
        .create_collection("orders".to_string(), None)
        .unwrap();
    let orders = engine.get_collection("orders").unwrap();
    for (key, country, status, amount) in [
        ("us-open", "US", "open", 10),
        ("us-paid", "US", "paid", 5),
        ("fr-open", "FR", "open", 7),
    ] {
        orders
            .insert(json!({
                "_key": key,
                "country": country,
                "status": status,
                "amount": amount
            }))
            .unwrap();
    }
}

fn insert_employees(engine: &StorageEngine) {
    engine.create_collection("emp".to_string(), None).unwrap();
    let emp = engine.get_collection("emp").unwrap();
    for (key, dept, salary) in [
        ("e1", "eng", 100),
        ("e2", "eng", 90),
        ("e3", "eng", 80),
        ("e4", "eng", 70),
        ("e5", "sales", 60),
        ("e6", "sales", 50),
    ] {
        emp.insert(json!({"_key": key, "dept": dept, "salary": salary}))
            .unwrap();
    }
}

fn num(value: &Value) -> i64 {
    value.as_i64().unwrap_or_else(|| {
        value
            .as_f64()
            .map(|n| n as i64)
            .unwrap_or_else(|| panic!("not a number: {value}"))
    })
}

#[test]
fn explain_plan_does_not_run_or_write() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("t".to_string(), None).unwrap();

    let plan = execute_query(&engine, "EXPLAIN RETURN 1");
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0]["plan_only"], json!(true));
    assert!(plan[0]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap_or("").contains("plan only")));
    assert!(plan[0].get("results").is_none());

    let wrote = execute_query(&engine, r#"EXPLAIN INSERT {_key: "a", n: 1} INTO t"#);
    assert_eq!(wrote[0]["plan_only"], json!(true));
    assert!(execute_query(&engine, "FOR d IN t RETURN d").is_empty());
}

#[test]
fn explain_analyze_of_a_read_returns_the_plan() {
    let (engine, _tmp) = create_test_engine();
    let plan = execute_query(&engine, "EXPLAIN ANALYZE RETURN 1");
    assert_eq!(plan.len(), 1);
    assert_ne!(plan[0]["plan_only"], json!(true));
    assert!(plan[0].get("timing").is_some());
    assert!(plan[0].get("collections").is_some());
    assert_eq!(plan[0]["documents_returned"], json!(1));
}

#[test]
fn explain_analyze_of_a_write_runs_it_and_returns_the_plan() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("t".to_string(), None).unwrap();
    let plan = execute_query(
        &engine,
        r#"EXPLAIN ANALYZE INSERT {_key: "b", n: 1} INTO t"#,
    );
    assert_eq!(plan.len(), 1);
    assert!(plan[0]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap_or("").contains("writing query")));
    let keys = execute_query(&engine, "FOR d IN t RETURN d._key");
    assert_eq!(keys, vec![json!("b")]);
}

#[test]
fn explain_mentions_qualify_sample_and_rollup() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("t".to_string(), None).unwrap();
    let plan = execute_query(
        &engine,
        r#"EXPLAIN
           FOR d IN t
           SAMPLE 1
           COLLECT k = d.n ROLLUP AGGREGATE s = SUM(d.n)
           QUALIFY s > 0
           RETURN s"#,
    );
    let warnings = plan[0]["warnings"].to_string();
    assert!(warnings.contains("QUALIFY"));
    assert!(warnings.contains("SAMPLE"));
    assert!(warnings.contains("ROLLUP"));
}

#[test]
fn qualify_keeps_the_top_three_per_department() {
    let (engine, _tmp) = create_test_engine();
    insert_employees(&engine);
    let rows = execute_query(
        &engine,
        r#"
        FOR e IN emp
        QUALIFY ROW_NUMBER() OVER (PARTITION BY e.dept ORDER BY e.salary DESC) <= 3
        RETURN { key: e._key, dept: e.dept, salary: e.salary }
        "#,
    );
    let keys: HashSet<_> = rows
        .iter()
        .map(|r| r["key"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        keys,
        HashSet::from([
            "e1".to_string(),
            "e2".to_string(),
            "e3".to_string(),
            "e5".to_string(),
            "e6".to_string(),
        ])
    );
}

#[test]
fn qualify_window_numbers_are_from_before_the_filter() {
    let (engine, _tmp) = create_test_engine();
    insert_employees(&engine);
    let rows = execute_query(
        &engine,
        r#"
        FOR e IN emp
        QUALIFY ROW_NUMBER() OVER (ORDER BY e.salary DESC) > 1
        RETURN ROW_NUMBER() OVER (ORDER BY e.salary DESC)
        "#,
    );
    assert_eq!(rows.len(), 5);
    let numbers: Vec<i64> = rows.iter().map(num).collect();
    assert!(numbers.iter().all(|n| *n >= 2), "{numbers:?}");
    assert!(numbers.contains(&2));
    assert!(numbers.contains(&6));
}

#[test]
fn qualify_sorts_after_the_filter() {
    let (engine, _tmp) = create_test_engine();
    insert_employees(&engine);
    let rows = execute_query(
        &engine,
        r#"
        FOR e IN emp
        QUALIFY e.salary >= 80
        SORT e.salary
        RETURN e.salary
        "#,
    );
    assert_eq!(rows, vec![json!(80), json!(90), json!(100)]);
}

/// The scan decodes only the fields the query reads. QUALIFY's count: a
/// field that RETURN does not name arrived as null in the predicate, so
/// `QUALIFY e.salary >= 80 RETURN e._key` returned nothing.
#[test]
fn qualify_reads_fields_the_return_does_not() {
    let (engine, _tmp) = create_test_engine();
    insert_employees(&engine);

    let rows = execute_query(&engine, "FOR e IN emp QUALIFY e.salary >= 80 RETURN e._key");
    assert_eq!(rows, vec![json!("e1"), json!("e2"), json!("e3")]);

    // The window partitions and orders by two fields RETURN does not read.
    let rows = execute_query(
        &engine,
        r#"
        FOR e IN emp
        QUALIFY ROW_NUMBER() OVER (PARTITION BY e.dept ORDER BY e.salary ASC) <= 1
        SORT e._key
        RETURN e._key
        "#,
    );
    assert_eq!(rows, vec![json!("e4"), json!("e6")]);

    let rows = execute_query(
        &engine,
        "FOR e IN emp QUALIFY e.salary >= 60 RETURN DISTINCT e.dept",
    );
    let depts: HashSet<_> = rows.iter().map(|r| r.as_str().unwrap()).collect();
    assert_eq!(depts, HashSet::from(["eng", "sales"]));

    // A subquery has a projection of its own.
    let rows = execute_query(
        &engine,
        r#"
        FOR d IN ["eng", "sales"]
        LET top = (FOR e IN emp FILTER e.dept == d QUALIFY e.salary >= 90 SORT e._key RETURN e._key)
        RETURN top
        "#,
    );
    assert_eq!(rows, vec![json!(["e1", "e2"]), json!([])]);
}

/// The same for a traversal, whose vertices and edges are read with only
/// the fields the query names.
#[test]
fn qualify_reads_fields_of_a_traversal_the_return_does_not() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("people".to_string(), None)
        .unwrap();
    engine
        .create_collection("knows".to_string(), Some("edge".to_string()))
        .unwrap();
    let people = engine.get_collection("people").unwrap();
    for (key, age) in [("a", 10), ("b", 20), ("c", 30)] {
        people.insert(json!({"_key": key, "age": age})).unwrap();
    }
    let knows = engine.get_collection("knows").unwrap();
    for (to, since) in [("b", 2020), ("c", 2024)] {
        knows
            .insert(json!({
                "_from": "people/a",
                "_to": format!("people/{to}"),
                "since": since
            }))
            .unwrap();
    }

    let rows = execute_query(
        &engine,
        r#"FOR v IN 1..1 OUTBOUND "people/a" knows QUALIFY v.age >= 30 RETURN v._key"#,
    );
    assert_eq!(rows, vec![json!("c")]);

    let rows = execute_query(
        &engine,
        r#"FOR v, e IN 1..1 OUTBOUND "people/a" knows QUALIFY e.since < 2022 RETURN v._key"#,
    );
    assert_eq!(rows, vec![json!("b")]);
}

#[test]
fn sort_before_qualify_is_a_parse_error() {
    let err = parse("FOR e IN emp SORT e.salary QUALIFY e.salary > 0 RETURN e").unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("Unexpected") || text.contains("QUALIFY"),
        "{text}"
    );
}

#[test]
fn qualify_evaluation_error_fails_the_query() {
    let (engine, _tmp) = create_test_engine();
    insert_employees(&engine);
    let err = try_query(
        &engine,
        "FOR e IN emp QUALIFY NO_SUCH_FUNCTION(e.salary) RETURN e",
    )
    .unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn rollup_prefixes_and_grouping_bits() {
    let (engine, _tmp) = create_test_engine();
    insert_orders(&engine);
    let rows = execute_query(
        &engine,
        r#"
        FOR o IN orders
        COLLECT country = o.country, status = o.status ROLLUP
        AGGREGATE total = SUM(o.amount)
        RETURN {
            country,
            status,
            total,
            g_country: GROUPING(country),
            g_status: GROUPING(status)
        }
        "#,
    );
    let got: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r["country"].as_str().map(str::to_string),
                r["status"].as_str().map(str::to_string),
                num(&r["total"]),
                num(&r["g_country"]),
                num(&r["g_status"]),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (Some("FR".into()), Some("open".into()), 7, 0, 0),
            (Some("US".into()), Some("open".into()), 10, 0, 0),
            (Some("US".into()), Some("paid".into()), 5, 0, 0),
            (Some("FR".into()), None, 7, 0, 1),
            (Some("US".into()), None, 15, 0, 1),
            (None, None, 22, 1, 1),
        ]
    );
}

#[test]
fn cube_adds_every_subset() {
    let (engine, _tmp) = create_test_engine();
    insert_orders(&engine);
    let rows = execute_query(
        &engine,
        r#"
        FOR o IN orders
        COLLECT country = o.country, status = o.status CUBE
        AGGREGATE total = SUM(o.amount)
        RETURN {
            country,
            status,
            total,
            g_country: GROUPING(country),
            g_status: GROUPING(status)
        }
        "#,
    );
    let got: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r["country"].as_str().map(str::to_string),
                r["status"].as_str().map(str::to_string),
                num(&r["total"]),
                num(&r["g_country"]),
                num(&r["g_status"]),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (Some("FR".into()), Some("open".into()), 7, 0, 0),
            (Some("US".into()), Some("open".into()), 10, 0, 0),
            (Some("US".into()), Some("paid".into()), 5, 0, 0),
            (None, Some("open".into()), 17, 1, 0),
            (None, Some("paid".into()), 5, 1, 0),
            (Some("FR".into()), None, 7, 0, 1),
            (Some("US".into()), None, 15, 0, 1),
            (None, None, 22, 1, 1),
        ]
    );
}

#[test]
fn grouping_is_zero_on_an_ordinary_collect_and_rejected_outside_one() {
    let (engine, _tmp) = create_test_engine();
    insert_orders(&engine);
    let rows = execute_query(
        &engine,
        r#"
        FOR o IN orders
        COLLECT country = o.country AGGREGATE total = SUM(o.amount)
        RETURN GROUPING(country)
        "#,
    );
    assert!(rows.iter().all(|r| num(r) == 0));
    let err = try_query(&engine, "RETURN GROUPING(country)").unwrap_err();
    assert!(err.contains("COLLECT"), "{err}");
}

#[test]
fn rollup_of_no_rows_has_no_grand_total() {
    let (engine, _tmp) = create_test_engine();
    insert_orders(&engine);
    let rows = execute_query(
        &engine,
        r#"
        FOR o IN orders
        FILTER o.country == "ZZ"
        COLLECT country = o.country, status = o.status ROLLUP
        AGGREGATE total = SUM(o.amount)
        RETURN total
        "#,
    );
    assert!(rows.is_empty());
}

#[test]
fn ungrouped_rollup_matches_an_ordinary_collect() {
    let (engine, _tmp) = create_test_engine();
    insert_orders(&engine);
    let plain = execute_query(
        &engine,
        r#"
        FOR o IN orders
        FILTER o.country == "ZZ"
        COLLECT AGGREGATE total = SUM(o.amount)
        RETURN total
        "#,
    );
    let rolled = execute_query(
        &engine,
        r#"
        FOR o IN orders
        FILTER o.country == "ZZ"
        COLLECT ROLLUP AGGREGATE total = SUM(o.amount)
        RETURN total
        "#,
    );
    assert_eq!(plain, rolled);
    assert_eq!(plain.len(), 1);
}

#[test]
fn rollup_merges_collect_list() {
    let (engine, _tmp) = create_test_engine();
    insert_orders(&engine);
    let rows = execute_query(
        &engine,
        r#"
        FOR o IN orders
        FILTER o.country == "US"
        COLLECT country = o.country, status = o.status ROLLUP
        AGGREGATE amounts = COLLECT_LIST(o.amount)
        RETURN { country, status, amounts, g_status: GROUPING(status) }
        "#,
    );
    let subtotal = rows
        .iter()
        .find(|r| r["country"] == "US" && r["status"].is_null() && num(&r["g_status"]) == 1)
        .unwrap();
    let mut amounts: Vec<i64> = subtotal["amounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(num)
        .collect();
    amounts.sort();
    assert_eq!(amounts, vec![5, 10]);
}

#[test]
fn rollup_rejects_more_than_31_group_variables() {
    let (engine, _tmp) = create_test_engine();
    let vars = (0..32)
        .map(|i| format!("a{i} = {i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!("COLLECT {vars} ROLLUP AGGREGATE n = SUM(1) RETURN n");
    let err = try_query(&engine, &query).unwrap_err();
    assert!(err.contains("31"), "{err}");
}

#[test]
fn sample_count_is_a_subset_and_percent_keeps_order() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("events".to_string(), None)
        .unwrap();
    let events = engine.get_collection("events").unwrap();
    for i in 0..20 {
        events
            .insert(json!({"_key": format!("k{i:02}"), "n": i}))
            .unwrap();
    }

    let sample = execute_query(&engine, "FOR d IN events SAMPLE 5 RETURN d._key");
    assert_eq!(sample.len(), 5);
    let all = execute_query(&engine, "FOR d IN events RETURN d._key");
    assert!(sample.iter().all(|k| all.contains(k)));

    let everything = execute_query(&engine, "FOR d IN events SAMPLE 100 RETURN d._key");
    assert_eq!(everything, all);

    let percent = execute_query(&engine, "FOR d IN events SAMPLE 100 PERCENT RETURN d._key");
    assert_eq!(percent, all);

    assert!(execute_query(&engine, "FOR d IN events SAMPLE 0 RETURN d").is_empty());
    assert!(execute_query(&engine, "FOR d IN events SAMPLE 0 PERCENT RETURN d").is_empty());
    assert!(execute_query(&engine, "FOR d IN events SAMPLE 0 FILTER true RETURN d").is_empty());
}

#[test]
fn sample_one_eventually_returns_each_of_two_rows() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("pair".to_string(), None).unwrap();
    let pair = engine.get_collection("pair").unwrap();
    pair.insert(json!({"_key": "a"})).unwrap();
    pair.insert(json!({"_key": "b"})).unwrap();
    let mut seen = HashSet::new();
    for _ in 0..40 {
        let rows = execute_query(&engine, "FOR d IN pair SAMPLE 1 RETURN d._key");
        assert_eq!(rows.len(), 1);
        seen.insert(rows[0].as_str().unwrap().to_string());
    }
    assert_eq!(seen, HashSet::from(["a".to_string(), "b".to_string()]));
}

#[test]
fn sample_rejects_a_bad_count_or_percent() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("events".to_string(), None)
        .unwrap();
    for query in [
        "FOR d IN events SAMPLE -1 RETURN d",
        "FOR d IN events SAMPLE 1.5 RETURN d",
        "FOR d IN events SAMPLE 101 PERCENT RETURN d",
        "FOR d IN events SAMPLE -0.1 PERCENT RETURN d",
    ] {
        let err = try_query(&engine, query).unwrap_err();
        assert!(err.contains("SAMPLE"), "{query}: {err}");
    }
}

#[test]
fn sample_accepts_a_bind_variable_and_an_array() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("events".to_string(), None)
        .unwrap();
    let events = engine.get_collection("events").unwrap();
    for i in 0..6 {
        events
            .insert(json!({"_key": format!("k{i}"), "n": i}))
            .unwrap();
    }
    let mut binds = std::collections::HashMap::new();
    binds.insert("n".to_string(), json!(2));
    let rows = execute_with_binds(&engine, "FOR d IN events SAMPLE @n RETURN d._key", binds);
    assert_eq!(rows.len(), 2);

    let from_array = execute_query(&engine, "FOR x IN [1, 2, 3, 4, 5] SAMPLE 2 RETURN x");
    assert_eq!(from_array.len(), 2);
    assert!(from_array.iter().all(|v| (1..=5).contains(&num(v))));

    let all = execute_query(&engine, "FOR i IN 1..5 RETURN i");
    let sampled = execute_query(&engine, "FOR i IN 1..5 SAMPLE 100 PERCENT RETURN i");
    assert_eq!(sampled, all);
    assert!(execute_query(&engine, "FOR i IN 1..5 SAMPLE 0 RETURN i").is_empty());
}

#[test]
fn sample_does_not_take_the_raw_whole_collection_path() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("events".to_string(), None)
        .unwrap();
    let events = engine.get_collection("events").unwrap();
    for i in 0..12 {
        events
            .insert(json!({"_key": format!("k{i:02}"), "n": i}))
            .unwrap();
    }
    let rows = execute_query(&engine, "FOR d IN events SAMPLE 2 RETURN d");
    assert_eq!(rows.len(), 2);
}

// Review findings (2026-11-25): one test per finding.

#[test]
fn explain_of_a_stream_or_materialized_view_statement_is_refused() {
    // A plan-only EXPLAIN counts as a read, and these statements were run
    // before anything looked at it: a reader could create them.
    for query in [
        "EXPLAIN CREATE MATERIALIZED VIEW mv AS FOR d IN orders RETURN d",
        "EXPLAIN ANALYZE CREATE MATERIALIZED VIEW mv AS FOR d IN orders RETURN d",
        "EXPLAIN REFRESH MATERIALIZED VIEW mv",
        "EXPLAIN CREATE STREAM s AS FOR d IN orders RETURN d",
    ] {
        let err = parse(query).expect_err(query).to_string();
        assert!(err.contains("EXPLAIN applies to queries"), "{query}: {err}");
    }
}

#[test]
fn explain_never_takes_the_drivers_raw_reads() {
    let (engine, _tmp) = create_test_engine();
    insert_orders(&engine);
    let executor = QueryExecutor::new(&engine);
    for query in [
        "EXPLAIN FOR v IN orders RETURN v",
        "EXPLAIN FOR v IN orders RETURN v.country",
    ] {
        let parsed = parse(query).unwrap();
        assert!(
            executor.execute_raw_msgpack(&parsed).unwrap().is_none(),
            "{query} was answered with rows"
        );
        let plan = executor.execute(&parsed).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0]["plan_only"], json!(true), "{query}");
    }
}

#[test]
fn sample_and_qualify_hold_under_an_index_sort_with_a_limit() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("people".to_string(), None)
        .unwrap();
    let people = engine.get_collection("people").unwrap();
    for age in 0..40 {
        people
            .insert(json!({"_key": format!("p{age:02}"), "age": age}))
            .unwrap();
    }
    people
        .create_index(
            "age_idx".to_string(),
            vec!["age".to_string()],
            solidb::storage::IndexType::Persistent,
            false,
        )
        .unwrap();

    // QUALIFY with no window function is a filter: no one under 30.
    let ages = execute_query(
        &engine,
        "FOR d IN people QUALIFY d.age >= 30 SORT d.age LIMIT 3 RETURN d.age",
    );
    assert_eq!(ages, vec![json!(30), json!(31), json!(32)]);

    // Three of the five sampled, not the three youngest of all: over
    // twenty runs, some answer is not [0, 1, 2].
    let youngest = vec![json!(0), json!(1), json!(2)];
    let differs = (0..20).any(|_| {
        execute_query(
            &engine,
            "FOR d IN people SAMPLE 5 SORT d.age LIMIT 3 RETURN d.age",
        ) != youngest
    });
    assert!(differs, "SAMPLE was ignored by the index sort");
}

#[test]
fn sample_and_qualify_hold_in_a_subquery_over_an_array() {
    let (engine, _tmp) = create_test_engine();
    let rows = execute_query(
        &engine,
        "RETURN (FOR i IN [1, 2, 3, 4] QUALIFY i > 2 RETURN i)",
    );
    assert_eq!(rows, vec![json!([3, 4])]);

    let rows = execute_query(
        &engine,
        "FOR d IN [{items: [1, 2, 3, 4, 5, 6]}] RETURN (FOR i IN d.items SAMPLE 1 RETURN i)",
    );
    assert_eq!(rows[0].as_array().unwrap().len(), 1);
}

#[test]
fn a_window_call_is_numbered_again_in_a_subquery_and_after_a_filter() {
    let (engine, _tmp) = create_test_engine();
    // The subquery's rows inherit the outer binding of the same call: they
    // are still numbered on their own rows.
    let rows = execute_query(
        &engine,
        "FOR u IN [{items: [7, 8, 9]}] RETURN {n: ROW_NUMBER() OVER (), items: (FOR i IN u.items RETURN ROW_NUMBER() OVER ())}",
    );
    assert_eq!(rows, vec![json!({"n": 1, "items": [1, 2, 3]})]);

    // A LET after a FILTER numbers the rows that passed it.
    let rows = execute_query(
        &engine,
        "FOR x IN [{k: 1, ok: false}, {k: 2, ok: true}, {k: 3, ok: true}] \
         LET a = ROW_NUMBER() OVER (ORDER BY x.k) FILTER x.ok \
         LET b = ROW_NUMBER() OVER (ORDER BY x.k) RETURN [a, b]",
    );
    assert_eq!(rows, vec![json!([2, 1]), json!([3, 2])]);
}

#[test]
fn a_bare_into_after_a_collect_shows_no_internal_binding() {
    let (engine, _tmp) = create_test_engine();
    let rows = execute_query(
        &engine,
        "FOR x IN [{a: 1}, {a: 2}] COLLECT a = x.a \
         FOR y IN [10] COLLECT b = y COLLECT c = 1 INTO g RETURN g",
    );
    for member in rows[0].as_array().unwrap() {
        for name in member.as_object().unwrap().keys() {
            assert!(
                !name.starts_with('\u{0}'),
                "internal binding {name:?} in {member}"
            );
        }
    }
}

#[test]
fn a_subquery_that_samples_is_drawn_for_each_row() {
    let (engine, _tmp) = create_test_engine();
    // The subquery reads no outer binding: memoised, every row got the
    // same pick. Over 64 rows and 8 values, two picks differ.
    let rows = execute_query(
        &engine,
        "FOR u IN 1..64 LET pick = (FOR p IN 1..8 SAMPLE 1 RETURN p)[0] RETURN pick",
    );
    let picks: HashSet<String> = rows.iter().map(|v| v.to_string()).collect();
    assert!(picks.len() > 1, "every row got the same sample: {picks:?}");
}

#[test]
fn cube_copies_count_toward_the_row_ceiling() {
    let (engine, _tmp) = create_test_engine();
    // 64 rows, 6 group variables: CUBE copies every member 63 more times,
    // about 4,000 held values, which a ceiling of 1,000 refuses.
    // (Without INTO it holds one value per group, 729, under the ceiling.)
    let query = "FOR x IN 0..63 COLLECT a = x % 2, b = FLOOR(x / 2) % 2, c = FLOOR(x / 4) % 2, \
         d = FLOOR(x / 8) % 2, e = FLOOR(x / 16) % 2, f = FLOOR(x / 32) % 2 CUBE";
    let run = |q: String| {
        QueryExecutor::new(&engine)
            .with_max_intermediate_rows(1000)
            .execute(&parse(&q).unwrap())
            .map_err(|e| e.to_string())
    };
    assert_eq!(run(format!("{query} RETURN 1")).unwrap().len(), 729);
    let err = run(format!("{query} INTO g RETURN 1")).expect_err("CUBE INTO held past the ceiling");
    assert!(err.contains("SOLIDB_MAX_INTERMEDIATE_ROWS"), "{err}");
}

#[test]
fn sample_count_is_uniform() {
    let (engine, _tmp) = create_test_engine();
    // 400 draws of 3 from 20: each value is expected 60 times. Keeping the
    // wrong end of the shuffle returned only the first and last three.
    let mut seen = std::collections::HashMap::<i64, usize>::new();
    for _ in 0..400 {
        for v in execute_query(&engine, "FOR i IN 0..19 SAMPLE 3 RETURN i") {
            *seen.entry(num(&v)).or_default() += 1;
        }
    }
    for i in 0..20 {
        let n = seen.get(&i).copied().unwrap_or(0);
        assert!(n >= 25, "{i} drawn {n} times of an expected 60: {seen:?}");
    }
}

#[test]
fn sample_of_a_source_past_the_ceiling_is_refused() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("big".to_string(), None).unwrap();
    let big = engine.get_collection("big").unwrap();
    for i in 0..50 {
        big.insert(json!({"_key": format!("k{i:02}")})).unwrap();
    }
    // The scan stops one past the ceiling; a sample of that would be of
    // the first rows only.
    let err = QueryExecutor::new(&engine)
        .with_max_intermediate_rows(10)
        .execute(&parse("FOR d IN big SAMPLE 2 RETURN d._key").unwrap())
        .expect_err("sampled a cut scan");
    assert!(
        err.to_string().contains("SOLIDB_MAX_INTERMEDIATE_ROWS"),
        "{err}"
    );
}

// Second review (2026-11-25).

#[test]
fn explain_is_refused_anywhere_but_the_start_of_a_statement() {
    for query in [
        "(EXPLAIN FOR d IN c RETURN d) UNION (FOR d IN c RETURN d)",
        "FOR x IN [1] RETURN x UNION ALL EXPLAIN FOR y IN [2] RETURN y",
        "WITH t AS (EXPLAIN FOR i IN 1..3 RETURN i) FOR x IN t RETURN x",
    ] {
        let err = parse(query).expect_err(query).to_string();
        assert!(err.contains("start of a statement"), "{query}: {err}");
    }
    // A subquery does not start with EXPLAIN at all.
    parse("RETURN (EXPLAIN FOR i IN 1..3 RETURN i)").expect_err("EXPLAIN in a subquery");
    parse("EXPLAIN (FOR a IN one RETURN a) UNION (FOR b IN two RETURN b)").unwrap();
}

#[test]
fn explain_analyze_of_a_sampled_or_qualified_read_counts_its_rows() {
    let (engine, _tmp) = create_test_engine();
    insert_employees(&engine);
    let plan = execute_query(&engine, "EXPLAIN ANALYZE FOR e IN emp SAMPLE 2 RETURN e");
    assert_eq!(plan[0]["documents_returned"], json!(2));
    assert!(plan[0].get("plan_only").is_none(), "{}", plan[0]);
    let plan = execute_query(
        &engine,
        "EXPLAIN ANALYZE FOR e IN emp QUALIFY ROW_NUMBER() OVER (PARTITION BY e.dept ORDER BY e.salary DESC) <= 1 RETURN e",
    );
    assert_eq!(plan[0]["documents_returned"], json!(2));
}

#[test]
fn a_return_window_that_reads_a_late_let_is_numbered_after_sort() {
    let (engine, _tmp) = create_test_engine();
    let rows = |qualify: &str| {
        execute_query(
            &engine,
            &format!(
                "FOR e IN [{{k: 1, d: \"b\"}}, {{k: 2, d: \"a\"}}, {{k: 3, d: \"c\"}}] {qualify} \
                 SORT e.k LIMIT 10 LET r = RANK() OVER (ORDER BY e.d) \
                 RETURN [e.k, ROW_NUMBER() OVER (ORDER BY r)]"
            ),
        )
    };
    let without = rows("");
    assert_eq!(rows("QUALIFY e.k > 0"), without);
    assert_eq!(without, vec![json!([1, 2]), json!([2, 1]), json!([3, 3])]);
}

#[test]
fn grouping_misused_in_a_filter_fails_and_is_zero_past_31_variables() {
    let (engine, _tmp) = create_test_engine();
    insert_orders(&engine);
    let err = try_query(&engine, "FOR d IN orders FILTER GROUPING(d) == 0 RETURN d")
        .expect_err("GROUPING outside a COLLECT");
    assert!(err.contains("GROUPING"), "{err}");

    let vars: Vec<String> = (1..=32).map(|i| format!("v{i} = {i}")).collect();
    let rows = execute_query(
        &engine,
        &format!(
            "FOR x IN [1] COLLECT {} RETURN GROUPING(v32)",
            vars.join(", ")
        ),
    );
    assert_eq!(rows, vec![json!(0)]);
}
