//! Performance benchmarks for the query-engine bottleneck fixes:
//! index-metadata caching, clone-free field access, decorated sort,
//! and LIMIT pushdown into index lookups.
//!
//! Run with:
//!   cargo test --profile ci --features bench-tests \
//!     --test benchmark_perf_fixes -- --ignored --nocapture

use serde_json::json;
use solidb::sdbql::QueryExecutor;
use solidb::storage::{IndexType, StorageEngine};
use std::time::Instant;
use tempfile::TempDir;

fn create_test_db() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine.create_database("bench_db".to_string()).unwrap();
    (engine, tmp_dir)
}

fn seed_items(engine: &StorageEngine, count: usize) {
    let db = engine.get_database("bench_db").unwrap();
    db.create_collection("items".to_string(), None).unwrap();
    let items = db.get_collection("items").unwrap();

    for chunk_start in (0..count).step_by(5000) {
        let chunk_end = (chunk_start + 5000).min(count);
        let batch: Vec<_> = (chunk_start..chunk_end)
            .map(|i| {
                json!({
                    "val": (i * 17) % 1000,
                    "name": format!("item-{}", i),
                    "padding": "x".repeat(100),
                })
            })
            .collect();
        items.insert_batch(batch).unwrap();
    }
}

fn run_query(executor: &QueryExecutor, query: &str, label: &str) -> usize {
    // Warm-up run to populate block cache so we measure CPU, not cold I/O.
    let ast = solidb::sdbql::parse(query).unwrap();
    executor.execute(&ast).unwrap();

    let iterations = 5;
    let start = Instant::now();
    let mut len = 0;
    for _ in 0..iterations {
        let result = executor.execute(&ast).unwrap();
        len = result.len();
    }
    let avg = start.elapsed() / iterations;
    println!("{label}: {len} rows, avg {avg:?} over {iterations} runs");
    len
}

#[test]
#[ignore]
fn benchmark_full_scan_filter() {
    let (engine, _tmp) = create_test_db();
    seed_items(&engine, 100_000);
    let executor = QueryExecutor::with_database(&engine, "bench_db".to_string());

    let len = run_query(
        &executor,
        "FOR d IN items FILTER d.val == 42 RETURN d.name",
        "full-scan FILTER",
    );
    assert_eq!(len, 100);
}

#[test]
#[ignore]
fn benchmark_sort_limit_unindexed() {
    let (engine, _tmp) = create_test_db();
    seed_items(&engine, 100_000);
    let executor = QueryExecutor::with_database(&engine, "bench_db".to_string());

    let len = run_query(
        &executor,
        "FOR d IN items SORT d.val ASC, d.name DESC LIMIT 10 RETURN d.name",
        "SORT+LIMIT (no index)",
    );
    assert_eq!(len, 10);
}

#[test]
#[ignore]
fn benchmark_indexed_filter_limit() {
    let (engine, _tmp) = create_test_db();
    seed_items(&engine, 100_000);
    let db = engine.get_database("bench_db").unwrap();
    let items = db.get_collection("items").unwrap();
    items
        .create_index(
            "val_idx".to_string(),
            vec!["val".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();
    let executor = QueryExecutor::with_database(&engine, "bench_db".to_string());

    let len = run_query(
        &executor,
        "FOR d IN items FILTER d.val == 42 LIMIT 10 RETURN d.name",
        "indexed FILTER+LIMIT",
    );
    assert_eq!(len, 10);
}

#[test]
#[ignore]
fn benchmark_single_inserts_with_indexes() {
    let (engine, _tmp) = create_test_db();
    let db = engine.get_database("bench_db").unwrap();
    db.create_collection("items".to_string(), None).unwrap();
    let items = db.get_collection("items").unwrap();
    for (name, field) in [("idx_a", "a"), ("idx_b", "b"), ("idx_c", "c")] {
        items
            .create_index(
                name.to_string(),
                vec![field.to_string()],
                IndexType::Persistent,
                false,
            )
            .unwrap();
    }

    let count = 10_000;
    let start = Instant::now();
    for i in 0..count {
        items
            .insert(json!({ "a": i, "b": i % 100, "c": format!("c-{}", i) }))
            .unwrap();
    }
    let elapsed = start.elapsed();
    println!(
        "single inserts with 3 indexes: {count} docs in {elapsed:?} ({:.0} docs/s)",
        count as f64 / elapsed.as_secs_f64()
    );
}

#[test]
#[ignore]
fn benchmark_join_hash_vs_nested() {
    let (engine, _tmp) = create_test_db();
    let db = engine.get_database("bench_db").unwrap();
    db.create_collection("orders".to_string(), None).unwrap();
    db.create_collection("customers".to_string(), None).unwrap();

    let customers = db.get_collection("customers").unwrap();
    customers
        .insert_batch(
            (0..5_000)
                .map(|i| json!({"cid": i, "name": format!("cust-{}", i)}))
                .collect(),
        )
        .unwrap();
    let orders = db.get_collection("orders").unwrap();
    orders
        .insert_batch(
            (0..1_000)
                .map(|i| json!({"customer_id": i % 5_000, "total": i}))
                .collect(),
        )
        .unwrap();

    let executor = QueryExecutor::with_database(&engine, "bench_db".to_string());

    // Equi-join: hash path, O(L+R)
    let len = run_query(
        &executor,
        "FOR o IN orders JOIN customers ON o.customer_id == customers.cid RETURN LENGTH(customers)",
        "equi JOIN (hash path, 1k x 5k)",
    );
    assert_eq!(len, 1_000);

    // Non-equi join: nested-loop fallback, O(L*R) — the old cost of ALL joins
    let len = run_query(
        &executor,
        "FOR o IN orders FILTER o.total < 100 JOIN customers ON customers.cid < o.total RETURN LENGTH(customers)",
        "non-equi JOIN (nested loop, 100 x 5k)",
    );
    // 99: the total==0 order matches no customer (no cid < 0) and INNER drops it
    assert_eq!(len, 99);
}

/// A scan after `LET`s that bound sizeable arrays: every row of the `FOR`
/// clones the context, and the clone used to deep-copy those arrays. The
/// shape is bonfire's activity feed — id lists and key/parent pairs, then a
/// scan of comments filtered against them.
#[test]
#[ignore]
fn benchmark_scan_under_wide_lets() {
    let (engine, _tmp) = create_test_db();
    seed_items(&engine, 2_000);
    let executor = QueryExecutor::with_database(&engine, "bench_db".to_string());

    let lets = "LET ids = (FOR i IN 0..499 RETURN CONCAT(\"item-\", i)) \
                LET pairs = (FOR i IN 0..499 RETURN { k: CONCAT(\"item-\", i), p: i % 9 }) \
                LET more_pairs = (FOR i IN 0..499 RETURN { k: CONCAT(\"x-\", i), col: i }) ";
    let narrow = run_query(
        &executor,
        "LET ids = (FOR i IN 0..499 RETURN CONCAT(\"item-\", i)) \
         FOR doc IN items FILTER doc.name IN ids SORT doc.val DESC LIMIT 100 RETURN doc.name",
        "scan, one LET",
    );
    let wide = run_query(
        &executor,
        &format!(
            "{lets} FOR doc IN items FILTER doc.name IN ids SORT doc.val DESC LIMIT 100 RETURN doc.name"
        ),
        "scan, three LETs",
    );
    assert_eq!(narrow, wide);
}

/// The card board: 168 columns, 1,079 cards with a `column_id`, 2,022
/// comments with a `card_id`, in a database, read as an admin.
fn seed_board(engine: &StorageEngine) {
    let db = engine.get_database("bench_db").unwrap();
    for c in ["card_columns", "cards", "comments"] {
        db.create_collection(c.to_string(), None).unwrap();
    }
    let insert = |coll: &str, docs: Vec<serde_json::Value>| {
        db.get_collection(coll).unwrap().insert_batch(docs).unwrap();
    };
    insert(
        "card_columns",
        (0..168)
            .map(|i| json!({"_key": format!("col{i:03}"), "position": i}))
            .collect(),
    );
    insert(
        "cards",
        (0..1_079)
            .map(|i| {
                json!({
                    "_key": format!("card{i:04}"),
                    "column_id": format!("col{:03}", i % 168),
                    "title": format!("card {i}"),
                })
            })
            .collect(),
    );
    insert(
        "comments",
        (0..2_022)
            .map(|i| {
                let card = format!("card{:04}", (i * 7) % 1_079);
                json!({
                    "_key": format!("m{i:04}"),
                    "card_id": card,
                    "commentable_type": "card",
                    "commentable_id": card,
                    "body": format!("{i:04} {}", "lorem ipsum dolor sit amet ".repeat(3)),
                })
            })
            .collect(),
    );
}

/// One shape's time with reuse on and off, each on a fresh executor with a
/// 30 s budget: best of three when a run is quick, else the one run.
fn time_shape(engine: &StorageEngine, label: &str, query: &str) {
    let ast = solidb::sdbql::parse(query).unwrap();
    let admin = solidb::sdbql::QueryPrincipal::from_roles("admin", vec!["admin".to_string()]);
    for reuse in [true, false] {
        let mut best: Option<std::time::Duration> = None;
        let mut line = String::new();
        for _ in 0..3 {
            let executor = QueryExecutor::with_database(engine, "bench_db".to_string())
                .with_principal(admin.clone())
                .with_timeout(std::time::Duration::from_secs(30))
                .with_reuse_within_query(reuse);
            let start = Instant::now();
            let result = executor.execute(&ast);
            let elapsed = start.elapsed();
            match result {
                Ok(rows) => {
                    if best.is_none_or(|b| elapsed < b) {
                        best = Some(elapsed);
                        line = format!(
                            "{} rows, {} scans, {} subquery runs, {} probe candidates",
                            rows.len(),
                            executor.collection_scans(),
                            executor.subquery_runs(),
                            executor.probe_candidates()
                        );
                    }
                }
                Err(e) => {
                    println!(
                        "{label} [reuse {}]: TIMED OUT or failed after {elapsed:?}: {e}",
                        if reuse { "on" } else { "off" }
                    );
                    best = None;
                    break;
                }
            }
            if elapsed > std::time::Duration::from_secs(1) {
                break;
            }
        }
        if let Some(best) = best {
            println!(
                "{label} [reuse {}]: {best:?} ({line})",
                if reuse { "on" } else { "off" }
            );
        }
    }
}

/// The subquery shapes measured on the board (2.5.1 timings in the spec):
/// an inner subquery reading only an outer row, a LET shared by every row
/// inside and before a FOR, `IN (subquery)` over an index, and a correlated
/// FILTER with no index.
#[test]
#[ignore]
fn benchmark_subquery_shapes() {
    let (engine, _tmp) = create_test_db();
    seed_board(&engine);
    let shapes: &[(&str, &str, bool)] = &[
        (
            "#2 inner subquery reads the outer row",
            "FOR col IN card_columns LET ms = (FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == col._key RETURN c._key) RETURN m) RETURN ms",
            true,
        ),
        (
            "#3 shared LET inside the FOR",
            "FOR m IN comments LET cs = (FOR c IN cards RETURN c) RETURN 1",
            false,
        ),
        (
            "#4 shared LET before the FOR",
            "LET cs = (FOR c IN cards RETURN c) FOR m IN comments RETURN LENGTH(cs)",
            false,
        ),
        (
            "#5 IN (subquery)",
            r#"FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == "col001" RETURN c._key) RETURN m"#,
            true,
        ),
        (
            "#6 correlated FILTER",
            "FOR card IN cards LET ms = (FOR m IN comments FILTER m.card_id == card._key RETURN m) RETURN ms",
            true,
        ),
        (
            "#6b correlated FILTER, discriminator first",
            r#"FOR card IN cards LET ms = (FOR m IN comments FILTER m.commentable_type == "card" AND m.commentable_id == card._key RETURN m._key) RETURN ms"#,
            true,
        ),
    ];
    for (label, query, _) in shapes {
        time_shape(&engine, &format!("{label}, no index"), query);
    }
    let db = engine.get_database("bench_db").unwrap();
    for (coll, field) in [
        ("cards", "column_id"),
        ("comments", "card_id"),
        ("comments", "commentable_id"),
    ] {
        db.get_collection(coll)
            .unwrap()
            .create_index(
                format!("idx_{field}"),
                vec![field.to_string()],
                IndexType::Persistent,
                false,
            )
            .unwrap();
    }
    for (label, query, indexed) in shapes {
        if *indexed {
            time_shape(&engine, &format!("{label}, indexed"), query);
        }
    }
}

/// Soli's job claim on a queue that is mostly future work: 100k `scheduled`
/// and 20k `failed` jobs due later, 2k future `pending`, 50k `done`, and 50
/// jobs due now. Three setups: no index; one single-field index per field
/// (what 2.6.2 served the claim from — it read every row in the claimed
/// states); and the `[state, run_at, priority]` index Soli creates.
#[test]
#[ignore]
fn benchmark_job_claim() {
    let (engine, _tmp) = create_test_db();
    let db = engine.get_database("bench_db").unwrap();
    db.create_collection("jobs".to_string(), None).unwrap();
    let jobs = db.get_collection("jobs").unwrap();
    let groups: &[(&str, usize, bool)] = &[
        ("scheduled", 100_000, false),
        ("failed", 20_000, false),
        ("pending", 2_000, false),
        ("done", 50_000, true),
        ("pending", 30, true),
        ("failed", 10, true),
        ("scheduled", 10, true),
    ];
    let mut seq = 0usize;
    for &(state, count, past) in groups {
        for start in (0..count).step_by(5000) {
            let batch: Vec<_> = (start..(start + 5000).min(count))
                .map(|i| {
                    seq += 1;
                    let day = if past { 1 + i % 3 } else { 5 + i % 25 };
                    json!({
                        "state": state,
                        "run_at": format!("2026-11-{day:02}T{:02}:{:02}:00Z", i % 24, i % 60),
                        "priority": i % 3,
                        "queue": "default",
                        "class": "SendReminder",
                        "args": { "id": seq, "note": "x".repeat(120) },
                    })
                })
                .collect();
            jobs.insert_batch(batch).unwrap();
        }
    }

    const NOW: &str = "2026-11-04T12:00:00Z";
    let queries = [
        (
            "claim",
            format!(
                "FOR doc IN jobs FILTER doc.state IN [\"pending\", \"scheduled\", \"failed\"] \
                 AND doc.run_at <= \"{NOW}\" SORT doc.priority DESC, doc.run_at ASC \
                 LIMIT 200 RETURN doc"
            ),
        ),
        (
            "claim + indexHint",
            format!(
                "FOR doc IN jobs OPTIONS {{ indexHint: \"idx_jobs_claim\" }} \
                 FILTER doc.state IN [\"pending\", \"scheduled\", \"failed\"] \
                 AND doc.run_at <= \"{NOW}\" SORT doc.priority DESC, doc.run_at ASC \
                 LIMIT 200 RETURN doc"
            ),
        ),
        (
            "one state, ==",
            format!(
                "FOR doc IN jobs FILTER doc.state == \"scheduled\" AND doc.run_at <= \"{NOW}\" \
                 RETURN doc._key"
            ),
        ),
        (
            "one day's window",
            "FOR doc IN jobs FILTER doc.state == \"scheduled\" \
             AND doc.run_at >= \"2026-11-10\" AND doc.run_at < \"2026-11-11\" \
             COLLECT WITH COUNT INTO n RETURN n"
                .to_string(),
        ),
    ];

    let measure = |setup: &str| {
        for (label, q) in &queries {
            let ast = solidb::sdbql::parse(q).unwrap();
            let executor = QueryExecutor::with_database(&engine, "bench_db".to_string());
            let used = executor.explain(&ast).unwrap().collections[0]
                .index_used
                .clone()
                .unwrap_or_else(|| "full scan".into());
            executor.execute(&ast).unwrap();
            let mut best = std::time::Duration::MAX;
            let mut rows = 0;
            for _ in 0..5 {
                let start = Instant::now();
                rows = executor.execute(&ast).unwrap().len();
                best = best.min(start.elapsed());
            }
            println!(
                "BENCH|{setup}|{label}|{rows}|{used}|{:.3}",
                best.as_secs_f64() * 1e3
            );
        }
    };

    measure("no index");
    for f in ["state", "run_at", "priority"] {
        jobs.create_index(
            format!("idx_{f}"),
            vec![f.to_string()],
            IndexType::Hash,
            false,
        )
        .unwrap();
    }
    measure("single-field indexes (2.6.2 plan)");
    jobs.create_index(
        "idx_jobs_claim".to_string(),
        ["state", "run_at", "priority"].map(String::from).to_vec(),
        IndexType::Persistent,
        false,
    )
    .unwrap();
    measure("[state, run_at, priority]");
}

/// The planner shapes added after 2.6.3, on 200k documents: a composite index
/// read for its first field alone (rows and count), `!=` / `NOT IN` after a
/// fixed prefix, `IN` holding null, and an `OR` of two indexed fields.
#[test]
#[ignore]
fn benchmark_planner_shapes() {
    let (engine, _tmp) = create_test_db();
    let db = engine.get_database("bench_db").unwrap();
    db.create_collection("rows".to_string(), None).unwrap();
    let rows = db.get_collection("rows").unwrap();
    for start in (0..200_000usize).step_by(5000) {
        let batch: Vec<_> = (start..start + 5000)
            .map(|i| {
                let mut doc = json!({
                    "status": match (i / 50) % 10 { 0 => "Open", 1 => "Working", _ => "Completed" },
                    "object": format!("o{}", i % 40),
                    "sort_order": i % 20,
                    "account_id": format!("a{}", i % 2000),
                    "a": i % 100,
                    "b": i % 250,
                    "padding": "x".repeat(100),
                });
                if i % 97 != 0 {
                    doc["owner_id"] = json!(format!("u{}", i % 50));
                }
                doc["opportunity_id"] = match i % 5 {
                    0 => json!(null),
                    1 => json!(""),
                    _ => json!(format!("p{}", i % 1000)),
                };
                doc
            })
            .collect();
        rows.insert_batch(batch).unwrap();
    }
    for (name, fields, ty) in [
        (
            "idx_owner_status",
            vec!["owner_id", "status"],
            IndexType::Persistent,
        ),
        (
            "idx_object_sort",
            vec!["object", "sort_order"],
            IndexType::Hash,
        ),
        (
            "idx_opp_account",
            vec!["opportunity_id", "account_id"],
            IndexType::Persistent,
        ),
        ("idx_a", vec!["a"], IndexType::Hash),
        ("idx_b", vec!["b"], IndexType::Hash),
    ] {
        rows.create_index(
            name.to_string(),
            fields.into_iter().map(String::from).collect(),
            ty,
            false,
        )
        .unwrap();
    }
    let queries = [
        ("composite, first field only", "FOR d IN rows FILTER d.object == \"o7\" RETURN d._key"),
        (
            "composite, first field, count",
            "FOR d IN rows FILTER d.object == \"o7\" COLLECT WITH COUNT INTO n RETURN n",
        ),
        (
            "!= after the prefix",
            "FOR d IN rows FILTER d.owner_id == \"u7\" AND d.status != \"Completed\" RETURN d._key",
        ),
        (
            "NOT IN after the prefix",
            "FOR d IN rows FILTER d.owner_id == \"u7\" AND d.status NOT IN [\"Completed\", \"Working\"] RETURN d._key",
        ),
        (
            "IN holding null",
            "FOR d IN rows FILTER d.opportunity_id IN [null, \"\"] AND d.account_id IN [\"a1\", \"a2\", \"a3\"] RETURN d._key",
        ),
        ("OR of two indexes", "FOR d IN rows FILTER d.a == 3 OR d.b == 7 RETURN d._key"),
    ];
    for (label, q) in queries {
        let ast = solidb::sdbql::parse(q).unwrap();
        let executor = QueryExecutor::with_database(&engine, "bench_db".to_string());
        let used = executor.explain(&ast).unwrap().collections[0]
            .index_used
            .clone()
            .unwrap_or_else(|| "full scan".into());
        executor.execute(&ast).unwrap();
        let mut best = std::time::Duration::MAX;
        let mut n = 0;
        for _ in 0..5 {
            let start = Instant::now();
            let result = executor.execute(&ast).unwrap();
            best = best.min(start.elapsed());
            n = if result.len() == 1 {
                result[0].as_u64().unwrap_or(1) as usize
            } else {
                result.len()
            };
        }
        println!("BENCH|{label}|{n}|{used}|{:.3}", best.as_secs_f64() * 1e3);
    }
}
