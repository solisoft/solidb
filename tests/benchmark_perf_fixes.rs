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
    for (name, field) in [
        ("idx_status_dense", "status"),
        ("idx_owner_dense", "owner_id"),
    ] {
        rows.create_index_with(
            name.to_string(),
            vec![field.to_string()],
            IndexType::Persistent,
            false,
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
        // Non-sparse indexes, each beside the same condition written so the
        // planner cannot read it from an index (a scan, on this build).
        (
            "non-sparse: != alone",
            "FOR d IN rows FILTER d.status != \"Completed\" RETURN d._key",
        ),
        (
            "  same, scanned",
            "FOR d IN rows FILTER NOT (d.status == \"Completed\") RETURN d._key",
        ),
        // A sparse index on a field every row has: the ascending read must
        // first establish that no row is missing from it.
        ("sorted ASC, sparse, no nulls", "FOR d IN rows SORT d.a ASC LIMIT 10 RETURN d._key"),
        ("sorted DESC, sparse", "FOR d IN rows SORT d.a DESC LIMIT 10 RETURN d._key"),
        ("non-sparse: == null", "FOR d IN rows FILTER d.owner_id == null RETURN d._key"),
        (
            "  same, scanned",
            "FOR d IN rows FILTER NOT (d.owner_id != null) RETURN d._key",
        ),
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

/// The cost-based choice between candidate reads, on 200k documents: each
/// query as the planner now reads it, beside the read the fixed order took
/// before (forced with `indexHint`, which skips the counting), and two
/// shapes where both reads are cheap, to show what the counting costs.
#[test]
#[ignore]
fn benchmark_planner_cost_choice() {
    let (engine, _tmp) = create_test_db();
    let db = engine.get_database("bench_db").unwrap();
    db.create_collection("rows".to_string(), None).unwrap();
    let rows = db.get_collection("rows").unwrap();
    for start in (0..200_000usize).step_by(5000) {
        let batch: Vec<_> = (start..start + 5000)
            .map(|i| {
                json!({
                    "status": if i % 20 == 0 { "closed" } else { "open" },
                    "amount": i,
                    "owner_id": format!("u{}", i % 5),
                    "opportunity_id": format!("p{i}"),
                    "account_id": format!("a{}", i % 100_000),
                    "padding": "x".repeat(100),
                })
            })
            .collect();
        rows.insert_batch(batch).unwrap();
    }
    for (name, field, ty) in [
        ("idx_status", "status", IndexType::Hash),
        ("idx_amount", "amount", IndexType::Persistent),
        ("idx_owner", "owner_id", IndexType::Hash),
        ("idx_opp", "opportunity_id", IndexType::Hash),
        ("idx_acc", "account_id", IndexType::Hash),
    ] {
        rows.create_index(name.to_string(), vec![field.to_string()], ty, false)
            .unwrap();
    }
    let time = |q: &str| -> (usize, String, f64) {
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
            n = executor.execute(&ast).unwrap().len();
            best = best.min(start.elapsed());
        }
        (n, used, best.as_secs_f64() * 1e3)
    };
    for (label, filter, old) in [
        (
            "common == beside a selective range",
            "d.status == \"open\" AND d.amount >= 199990",
            "idx_status",
        ),
        (
            "team IN beside an OR of keys",
            "d.owner_id IN [\"u0\", \"u1\"] AND (d.opportunity_id == \"p5\" OR d.account_id == \"a7\")",
            "idx_owner",
        ),
        (
            "two-sided range, tight upper side",
            "d.amount >= 1000 AND d.amount < 1100",
            "idx_amount",
        ),
        (
            "cheap == then common ==",
            "d.opportunity_id == \"p5\" AND d.status == \"open\"",
            "idx_opp",
        ),
        (
            "common == then cheap ==",
            "d.status == \"open\" AND d.opportunity_id == \"p5\"",
            "idx_status",
        ),
        (
            "two cheap ==",
            "d.opportunity_id == \"p5\" AND d.account_id == \"a5\"",
            "idx_opp",
        ),
    ] {
        let (n, used, ms) = time(&format!("FOR d IN rows FILTER {filter} RETURN d._key"));
        let (old_n, old_used, old_ms) = time(&format!(
            "FOR d IN rows OPTIONS {{ indexHint: \"{old}\" }} FILTER {filter} RETURN d._key"
        ));
        assert_eq!(n, old_n, "{label}");
        assert_eq!(old_used, old, "{label}: the hint is what EXPLAIN names");
        println!("BENCH|{label}|{n}|{used}|{ms:.3}|{old_used}|{old_ms:.3}");
    }
}

/// SplitMix64, as `benchmarks/compare/src/data.rs` draws its orders.
fn mix(seed: u64, i: u64) -> u64 {
    let mut z = seed ^ i.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Order `i` of `benchmarks/compare` (`data.rs`): about 500 B of JSON, keyed
/// `o0000042`, whose customer (`c00042`, one of `customers`) is drawn at
/// random, so that a customer's orders lie in unrelated places of the
/// collection.
fn compare_order(i: u64, customers: u64) -> serde_json::Value {
    const STATUSES: [&str; 5] = ["delivered", "shipped", "pending", "cancelled", "returned"];
    const COUNTRIES: [&str; 20] = [
        "FR", "DE", "ES", "IT", "GB", "NL", "BE", "CH", "PT", "SE", "NO", "DK", "PL", "AT", "IE",
        "US", "CA", "BR", "JP", "AU",
    ];
    const CITIES: [&str; 8] = [
        "Lyon", "Paris", "Lille", "Nantes", "Bordeaux", "Berlin", "Madrid", "Milan",
    ];
    const STREETS: [&str; 4] = [
        "rue des Lilas",
        "avenue Jean Jaurès",
        "boulevard Voltaire",
        "place Bellecour",
    ];
    const TAGS: [&str; 12] = [
        "gift",
        "express",
        "promo",
        "first-order",
        "b2b",
        "fragile",
        "bulky",
        "subscription",
        "loyalty",
        "marketplace",
        "pickup",
        "eco",
    ];
    const NOTES: [&str; 4] = [
        "Leave the parcel with the concierge if nobody answers the door.",
        "Call before delivery; the intercom does not work.",
        "Deliver to the side entrance, next to the bakery.",
        "",
    ];
    let h = mix(4, i);
    let items: Vec<_> = (0..3u64)
        .map(|k| {
            let h = mix(10 + k, i);
            json!({
                "sku": format!("SKU-{:05}", h % 50_000),
                "qty": 1 + (h >> 20) % 4,
                "price": (199 + (h >> 32) % 9_800) as f64 / 100.0,
            })
        })
        .collect();
    json!({
        "_key": format!("o{i:07}"),
        "customer_id": format!("c{:05}", mix(1, i) % customers),
        "status": STATUSES[(mix(2, i) % 5) as usize],
        "amount": (mix(3, i) % 100_000) as f64 / 100.0,
        "currency": "EUR",
        "created_at": format!(
            "2025-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            1 + h % 12, 1 + (h >> 4) % 28, (h >> 9) % 24, (h >> 14) % 60, (h >> 20) % 60
        ),
        "country": COUNTRIES[(h % 20) as usize],
        "channel": (["web", "mobile", "store", "phone"][(h >> 8) as usize % 4]),
        "tags": [
            TAGS[(h >> 12) as usize % 12],
            TAGS[(h >> 16) as usize % 12],
            TAGS[(h >> 20) as usize % 12],
        ],
        "items": items,
        "address": {
            "street": format!("{} {}", 1 + (h >> 24) % 180, STREETS[(h >> 32) as usize % 4]),
            "city": CITIES[(h >> 36) as usize % 8],
            "zip": format!("{:05}", 1000 + (h >> 40) % 94_000),
        },
        "note": NOTES[(h >> 48) as usize % 4],
    })
}

/// This thread's CPU time in microseconds (Linux; wall-clock time from an
/// arbitrary origin elsewhere), so that a stage timed on a loaded host does
/// not count the time it waited for a core.
fn thread_cpu_us() -> f64 {
    #[cfg(target_os = "linux")]
    {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `ts` is a valid timespec for the call to fill.
        if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) } == 0 {
            return ts.tv_sec as f64 * 1e6 + ts.tv_nsec as f64 / 1e3;
        }
    }
    static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64() * 1e6
}

/// CPU time, in nanoseconds, of this process's threads whose name starts
/// with `prefix` (Linux; 0 elsewhere). Read from `schedstat`, so a loaded
/// host's waiting does not count, as it does in wall-clock throughput.
fn threads_cpu_ns(prefix: &str) -> u64 {
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return 0;
    };
    tasks
        .filter_map(|t| t.ok())
        .filter(|t| {
            std::fs::read_to_string(t.path().join("comm")).is_ok_and(|c| c.starts_with(prefix))
        })
        .filter_map(|t| std::fs::read_to_string(t.path().join("schedstat")).ok())
        .filter_map(|s| s.split_whitespace().next()?.parse::<u64>().ok())
        .sum()
}

/// The server's work on one request of each shape, split by phase and
/// timed in process on one thread (thread CPU time, so a loaded host's
/// waiting does not count): what each step the driver takes costs, by
/// calling it directly as the handler calls it. Steps nest — the handler
/// includes the prepared-statement lookup, the executor, the storage reads —
/// so the differences are the phases:
///
/// * frame decode: the command's MessagePack into a `Command`;
/// * parse: the SDBQL text, uncached (what the prepared-statement cache
///   saves), and the cache lookup that replaces it;
/// * handler: `handle_query` whole (auth check, principal, lookup, the
///   inline decision, executor setup, planning, reads, rows);
/// * executor: the same query on a fresh executor, as the handler runs it
///   inline (`execute_raw_msgpack_small`, then `execute_small_sorted`);
/// * storage: the index read and document fetches alone, nothing decoded;
/// * response encode: the frame written back.
///
/// What end to end adds beyond these (socket reads and writes, the runtime's
/// wakeups) is the server CPU of the TCP series less the handler and the
/// two codecs.
#[allow(clippy::type_complexity)]
fn phase_split(
    engine: &StorageEngine,
    shapes: &[(
        &str,
        &str,
        &dyn Fn(&str, u64) -> std::collections::HashMap<String, serde_json::Value>,
    )],
    docs: u64,
) {
    use solidb::driver::protocol::{decode_command, encode_command, encode_response, Command};
    use solidb::sdbql::executor::SmallRaw;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let storage = std::sync::Arc::new(engine.clone());
    let mut handler = solidb::driver::DriverHandler::new(storage, None);
    let auth = rt.block_on(solidb::driver::handlers::auth::handle_auth(
        &mut handler,
        "bench_db".to_string(),
        "admin".to_string(),
        "bench-password".to_string(),
        None,
    ));
    assert!(
        matches!(auth, solidb::driver::protocol::Response::Ok { .. }),
        "{auth:?}"
    );
    // Every step reads the same 256 random keys, read once before the first
    // step: all of them find their blocks cached, so each step costs what it
    // costs on a warm cache, whichever ran before it. (Fresh keys per step,
    // or a pool too large for the block cache, left some steps paying for
    // block-cache misses — a decompression each — that others did not.)
    const POOL: u64 = 256;
    let n = 5_000u64;
    let counter = InstructionCounters::this_thread();
    // Each step's thousands of user-space instructions per request, in the
    // order measured: steady under load, where the times are not.
    let kinst = std::cell::RefCell::new(Vec::<f64>::new());
    let time = |f: &mut dyn FnMut(u64)| -> f64 {
        for r in 0..n / 5 {
            f(mix(1, r % POOL));
        }
        let start = thread_cpu_us();
        let counted = counter.read();
        for r in 0..n {
            f(mix(1, r % POOL));
        }
        kinst
            .borrow_mut()
            .push((counter.read() - counted) as f64 / 1e3 / n as f64);
        (thread_cpu_us() - start) / n as f64
    };
    let db = engine.get_database("bench_db").unwrap();
    let orders = db.get_collection("orders").unwrap();
    let customers = db.get_collection("customers").unwrap();
    let created_index = orders
        .get_all_indexes()
        .into_iter()
        .find(|i| i.fields == ["created_at"])
        .unwrap();
    for (shape, q, binds_of) in shapes {
        let warm = solidb::sdbql::parse(q).unwrap();
        for r in 0..POOL {
            QueryExecutor::with_database_and_bind_vars(
                engine,
                "bench_db".to_string(),
                binds_of(shape, mix(1, r)),
            )
            .execute(&warm)
            .unwrap();
        }
    }
    for r in 0..POOL {
        orders
            .get_value(&format!("o{:07}", mix(1, r) % docs))
            .unwrap();
    }
    for (shape, q, binds_of) in shapes {
        let frame = |r: u64| {
            let cmd = Command::Query {
                database: "bench_db".to_string(),
                sdbql: q.to_string(),
                bind_vars: Some(binds_of(shape, r)),
                cache: false,
            };
            encode_command(&cmd).unwrap()[4..].to_vec()
        };
        let frames: Vec<Vec<u8>> = (0..64).map(frame).collect();
        let decode = time(&mut |r| {
            std::hint::black_box(decode_command(&frames[r as usize % 64]).unwrap());
        });
        let parse = time(&mut |_| {
            std::hint::black_box(solidb::sdbql::parse(q).unwrap());
        });
        let lookup = time(&mut |_| {
            std::hint::black_box(
                solidb::sdbql::get_prepared_statement_cache()
                    .parse_if_needed(q)
                    .unwrap(),
            );
        });
        let mut encoded = 0usize;
        let mut encode = 0.0;
        let mut encode_inst = 0u64;
        let handle = time(&mut |r| {
            let resp = rt.block_on(solidb::driver::handlers::query::handle_query(
                &handler,
                "bench_db".to_string(),
                q.to_string(),
                Some(binds_of(shape, r)),
                false,
            ));
            let (start, counted) = (thread_cpu_us(), counter.read());
            encoded += encode_response(&resp).unwrap().len();
            encode += thread_cpu_us() - start;
            encode_inst += counter.read() - counted;
        });
        // `time` ran the handler `n + n / 5` times, all of them encoded.
        let encode = encode / (n + n / 5) as f64;
        let encode_kinst = encode_inst as f64 / 1e3 / (n + n / 5) as f64;
        let handle = handle - encode;
        let ast = solidb::sdbql::parse(q).unwrap();
        let execute = time(&mut |r| {
            let executor = QueryExecutor::with_database_and_bind_vars(
                engine,
                "bench_db".to_string(),
                binds_of(shape, r),
            )
            .with_timeout(std::time::Duration::from_secs(30));
            match executor.execute_raw_msgpack_small(&ast, 64).unwrap() {
                SmallRaw::Rows(rows) => {
                    std::hint::black_box(rows);
                }
                _ => {
                    std::hint::black_box(executor.execute_small_sorted(&ast, 64).unwrap().unwrap());
                }
            }
        });
        let storage = time(&mut |r| {
            let binds = binds_of(shape, r);
            if *shape == "by_customer" {
                // Only `_key` copied: the index read and the fetches.
                let shape = solidb::storage::raw_rows::RawShape::Single("_key".to_string());
                let read =
                    orders.index_raw_eq("customer_id", &binds["c"], 0, 64, &shape, |_| Ok(()));
                std::hint::black_box(read.unwrap());
                return;
            }
            // The 20 newest of the window, their bytes handed back untouched.
            let mut keys = Vec::new();
            let walked = orders.index_sorted_range_by(
                &created_index,
                &[],
                Some((&binds["lo"], true)),
                Some((&binds["hi"], false)),
                false,
                20,
                256,
                &mut |bytes| {
                    if *shape == "join" {
                        let doc = solidb::storage::serializer::deserialize_doc_projected(
                            bytes,
                            &["customer_id".to_string()],
                        )
                        .ok()?;
                        keys.push(doc["customer_id"].as_str()?.to_string());
                    }
                    Some(Some(bytes.len()))
                },
            );
            std::hint::black_box(walked.unwrap());
            for key in keys {
                std::hint::black_box(customers.get_value(&key).unwrap());
            }
        });
        println!(
            "BENCH|phases|{shape}|frame decode {decode:.2} us|parse {parse:.2} us (cached lookup {lookup:.2})|handler {handle:.2} us|executor {execute:.2} us|storage {storage:.2} us|response encode {encode:.2} us|{:.0} B/response",
            encoded as f64 / (n + n / 5) as f64
        );
        let k = kinst.borrow();
        if let [decode, parse, lookup, handle, execute, storage] = k[k.len() - 6..] {
            println!(
                "BENCH|phases kinst|{shape}|frame decode {decode:.1}|parse {parse:.1} (cached lookup {lookup:.1})|handler {:.1}|executor {execute:.1}|storage {storage:.1}|response encode {encode_kinst:.1}",
                handle - encode_kinst
            );
        }
    }
    let get = time(&mut |r| {
        let key = format!("o{:07}", r % docs);
        std::hint::black_box(solidb::driver::handlers::document::handle_get(
            &handler,
            "bench_db".to_string(),
            "orders".to_string(),
            key,
        ));
    });
    let point = time(&mut |r| {
        std::hint::black_box(orders.get_value(&format!("o{:07}", r % docs)).unwrap());
    });
    println!("BENCH|phases|get|handler {get:.2} us|storage (decoded) {point:.2} us");
    let k = kinst.borrow();
    if let [get, point] = k[k.len() - 2..] {
        println!("BENCH|phases kinst|get|handler {get:.1}|storage (decoded) {point:.1}");
    }
}

/// User and system CPU time, in nanoseconds, of this process's threads
/// whose name starts with `prefix` (Linux; zeros elsewhere), from each
/// thread's `stat` (clock ticks, so only good over a second or more): what
/// the server spends in its own code and what in the kernel's socket calls.
fn threads_cpu_split_ns(prefix: &str) -> (u64, u64) {
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return (0, 0);
    };
    #[cfg(target_os = "linux")]
    // SAFETY: sysconf only reads a configuration value.
    let tick_ns = 1_000_000_000 / unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
    #[cfg(not(target_os = "linux"))]
    let tick_ns = 10_000_000u64;
    tasks
        .filter_map(|t| t.ok())
        .filter(|t| {
            std::fs::read_to_string(t.path().join("comm")).is_ok_and(|c| c.starts_with(prefix))
        })
        .filter_map(|t| std::fs::read_to_string(t.path().join("stat")).ok())
        .filter_map(|s| {
            // Fields after the parenthesised name: state is the 3rd field,
            // utime the 14th and stime the 15th.
            let rest = &s[s.rfind(')')? + 2..];
            let mut f = rest.split_whitespace().skip(11);
            Some((
                f.next()?.parse::<u64>().ok()?,
                f.next()?.parse::<u64>().ok()?,
            ))
        })
        .fold((0, 0), |(u, k), (du, dk)| {
            (u + du * tick_ns, k + dk * tick_ns)
        })
}

/// User-space instruction counters on this process's threads whose name
/// starts with a prefix (Linux, `perf_event_open`; empty elsewhere or when
/// the kernel refuses): what a series costs in work, unmoved by the load of
/// a shared host, where CPU time is not.
struct InstructionCounters(Vec<i32>);

impl InstructionCounters {
    fn open(prefix: &str) -> Self {
        let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
            return Self(Vec::new());
        };
        let tids: Vec<i32> = tasks
            .filter_map(|t| t.ok())
            .filter(|t| {
                std::fs::read_to_string(t.path().join("comm")).is_ok_and(|c| c.starts_with(prefix))
            })
            .filter_map(|t| t.file_name().to_str()?.parse().ok())
            .collect();
        Self::of(tids)
    }

    /// A counter on the calling thread alone.
    fn this_thread() -> Self {
        #[cfg(target_os = "linux")]
        // SAFETY: gettid has no preconditions.
        let tid = unsafe { libc::syscall(libc::SYS_gettid) } as i32;
        #[cfg(not(target_os = "linux"))]
        let tid = 0;
        Self::of(vec![tid])
    }

    fn of(tids: Vec<i32>) -> Self {
        #[cfg(target_os = "linux")]
        {
            // `struct perf_event_attr`, PERF_ATTR_SIZE_VER5 (112 bytes):
            // type PERF_TYPE_HARDWARE (0), config PERF_COUNT_HW_INSTRUCTIONS
            // (1), flags exclude_kernel (bit 5) and exclude_hv (bit 6).
            let mut attr = [0u64; 14];
            attr[0] = 112 << 32;
            attr[1] = 1;
            attr[5] = (1 << 5) | (1 << 6);
            let fds = tids
                .into_iter()
                .filter_map(|tid| {
                    // SAFETY: `attr` is a valid, fully initialised
                    // perf_event_attr of the size it declares.
                    let fd = unsafe {
                        libc::syscall(
                            libc::SYS_perf_event_open,
                            attr.as_ptr(),
                            tid,
                            -1i32,
                            -1i32,
                            0u64,
                        )
                    };
                    (fd >= 0).then_some(fd as i32)
                })
                .collect();
            Self(fds)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = tids;
            Self(Vec::new())
        }
    }

    /// Instructions counted so far, all threads together (0 when none).
    fn read(&self) -> u64 {
        #[cfg(target_os = "linux")]
        {
            self.0
                .iter()
                .map(|fd| {
                    let mut count = 0u64;
                    // SAFETY: reading 8 bytes into a u64 from a counter fd.
                    let n = unsafe { libc::read(*fd, (&mut count as *mut u64).cast(), 8) };
                    if n == 8 {
                        count
                    } else {
                        0
                    }
                })
                .sum()
        }
        #[cfg(not(target_os = "linux"))]
        {
            0
        }
    }
}

impl Drop for InstructionCounters {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        for fd in &self.0 {
            // SAFETY: closing a descriptor this value owns.
            unsafe { libc::close(*fd) };
        }
    }
}

/// What [`driver_series`] measured.
struct Series {
    /// Operations per second, all connections together.
    ops: f64,
    /// Latency percentiles, microseconds.
    p50: f64,
    p99: f64,
    /// The server's CPU per operation, microseconds (`schedstat`), and its
    /// user / kernel split (`stat`, in ticks: the two need not add up to it
    /// exactly).
    server_cpu: f64,
    server_user: f64,
    server_sys: f64,
    /// The client's CPU per operation, microseconds: encoding the command,
    /// decoding the response into `serde_json::Value`s, its socket calls.
    client_cpu: f64,
    /// User-space instructions per operation, thousands, server and client
    /// ([`InstructionCounters`]; 0 where they cannot be read).
    server_kinst: f64,
    client_kinst: f64,
}

impl std::fmt::Display for Series {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:.0}/s|p50 {:.1} us|p99 {:.1} us|server cpu {:.1} us/op (user {:.1}, sys {:.1})|client cpu {:.1} us/op|kinst/op server {:.1} client {:.1}",
            self.ops,
            self.p50,
            self.p99,
            self.server_cpu,
            self.server_user,
            self.server_sys,
            self.client_cpu,
            self.server_kinst,
            self.client_kinst
        )
    }
}

/// One measured series over the driver: `conc` connections, each sending
/// `op` back to back for `secs` after `warm`: throughput, latency and the
/// CPU each side spends per operation.
fn driver_series<F, Fut>(
    rt: &tokio::runtime::Runtime,
    addr: &str,
    conc: usize,
    warm: std::time::Duration,
    secs: std::time::Duration,
    op: F,
) -> Series
where
    F: Fn(solidb_client::SoliDBClient, u64) -> Fut + Copy + Send + 'static,
    Fut: std::future::Future<Output = solidb_client::SoliDBClient> + Send + 'static,
{
    rt.block_on(async move {
        let mut clients = Vec::with_capacity(conc);
        for _ in 0..conc {
            clients.push(
                solidb_client::SoliDBClientBuilder::new(addr)
                    .auth("bench_db", "admin", "bench-password")
                    .pool_size(1)
                    .build()
                    .await
                    .unwrap(),
            );
        }
        let start = Instant::now() + std::time::Duration::from_millis(50);
        let (measure_from, until) = (start + warm, start + warm + secs);
        let cpu = tokio::spawn(async move {
            tokio::time::sleep_until(measure_from.into()).await;
            let counters = (
                InstructionCounters::open("bench-server"),
                InstructionCounters::open("bench-client"),
            );
            let before = (
                threads_cpu_ns("bench-server"),
                threads_cpu_split_ns("bench-server"),
                threads_cpu_ns("bench-client"),
                counters.0.read(),
                counters.1.read(),
            );
            tokio::time::sleep_until(until.into()).await;
            let (user, sys) = threads_cpu_split_ns("bench-server");
            (
                threads_cpu_ns("bench-server") - before.0,
                (user - before.1 .0, sys - before.1 .1),
                threads_cpu_ns("bench-client") - before.2,
                (counters.0.read() - before.3, counters.1.read() - before.4),
            )
        });
        let tasks: Vec<_> = clients
            .into_iter()
            .enumerate()
            .map(|(n, mut client)| {
                tokio::spawn(async move {
                    tokio::time::sleep_until(start.into()).await;
                    let mut rng = mix(99, n as u64) | 1;
                    let mut lat: Vec<u32> = Vec::with_capacity(1 << 16);
                    loop {
                        rng ^= rng << 13;
                        rng ^= rng >> 7;
                        rng ^= rng << 17;
                        let t = Instant::now();
                        if t >= until {
                            break;
                        }
                        client = op(client, rng).await;
                        if t >= measure_from {
                            lat.push(t.elapsed().as_nanos() as u32);
                        }
                    }
                    lat
                })
            })
            .collect();
        let mut all = Vec::new();
        for t in tasks {
            all.extend(t.await.unwrap());
        }
        all.sort_unstable();
        let pct = |p: f64| all[((all.len() - 1) as f64 * p) as usize] as f64 / 1e3;
        let (server, (user, sys), client, (server_inst, client_inst)) = cpu.await.unwrap();
        let per_op = |ns: u64| ns as f64 / 1e3 / all.len().max(1) as f64;
        Series {
            ops: all.len() as f64 / secs.as_secs_f64(),
            p50: pct(0.5),
            p99: pct(0.99),
            server_cpu: per_op(server),
            server_user: per_op(user),
            server_sys: per_op(sys),
            client_cpu: per_op(client),
            server_kinst: per_op(server_inst),
            client_kinst: per_op(client_inst),
        }
    })
}

/// Bonfire card #775: `FOR o IN orders FILTER o.customer_id == @c RETURN o`
/// sent over the native driver with the query cache off, as
/// `benchmarks/compare` sends it — about ten orders of about 500 B per
/// customer, behind the sparse single-field persistent index the driver's
/// CreateIndex makes — beside a one-key `Get` over the same driver, and the
/// harness's `recent` and `join` (the newest 20 orders of a day through the
/// index on `created_at`, alone and with each order's customer).
///
/// First, on one thread and in process, what each stage of the raw-rows
/// path costs: the index seek alone, then the documents fetched, then copied
/// whole, then the executor around them, then the ordinary (decoding) path
/// for comparison. Then, for the four shapes, the server's work on one
/// request split by phase ([`phase_split`]: frame decode, parse and the
/// parse-cache lookup, the handler, the executor, the storage reads, the
/// response encode), in microseconds and in thousands of instructions. Then
/// the driver end to end on loopback TCP, at concurrency 1 and 32, the
/// server on its own runtime: throughput, p50 / p99, and per operation the
/// server's CPU (user / kernel), the client's, and both sides' user-space
/// instructions ([`InstructionCounters`], steady on a loaded host).
///
/// `SOLIDB_BENCH_ORDERS` (200,000), `SOLIDB_BENCH_BLOCK_CACHE_MB` (the
/// engine's default), `SOLIDB_BENCH_SECS` (5),
/// `SOLIDB_BENCH_SERVER_THREADS` (8), `SOLIDB_BENCH_CONC` (`1,32`) and
/// `SOLIDB_BENCH_SERIES` (`inproc,phases,by_customer,get,recent,join`)
/// override the defaults;
/// `SOLIDB_ENGINE=solistore` runs it on solistore (built with
/// `engine-solistore`).
#[test]
#[ignore]
fn benchmark_driver_by_customer() {
    use solidb::storage::raw_rows::RawShape;
    let docs: u64 = env_or("SOLIDB_BENCH_ORDERS", 200_000);
    let secs = std::time::Duration::from_secs(env_or("SOLIDB_BENCH_SECS", 5));
    let server_threads: usize = env_or("SOLIDB_BENCH_SERVER_THREADS", 8);
    if let Some(mb) = std::env::var("SOLIDB_BENCH_BLOCK_CACHE_MB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        let mut profile = solidb::storage::engine::EngineProfile::prod();
        profile.block_cache_bytes = mb << 20;
        solidb::storage::engine::set_engine_profile(profile);
    }
    std::env::set_var("SOLIDB_ADMIN_PASSWORD", "bench-password");

    let (engine, tmp) = create_test_db();
    let db = engine.get_database("bench_db").unwrap();
    db.create_collection("orders".to_string(), None).unwrap();
    let orders = db.get_collection("orders").unwrap();
    // `create_index` makes them sparse, as the driver's CreateIndex does.
    for field in ["customer_id", "created_at"] {
        orders
            .create_index(
                format!("idx_{field}"),
                vec![field.to_string()],
                IndexType::Persistent,
                false,
            )
            .unwrap();
    }
    let customers = (docs / 10).max(1);
    db.create_collection("customers".to_string(), None).unwrap();
    let customer_docs = db.get_collection("customers").unwrap();
    for start in (0..customers).step_by(10_000) {
        let batch: Vec<_> = (start..(start + 10_000).min(customers))
            .map(|c| {
                json!({
                    "_key": format!("c{c:05}"),
                    "name": format!("Customer {c}"),
                    "email": format!("customer{c}@example.com"),
                    "tier": (["bronze", "silver", "gold", "platinum"][(mix(5, c) % 4) as usize]),
                })
            })
            .collect();
        customer_docs.insert_batch(batch).unwrap();
    }
    let mut per_customer = vec![0u32; customers as usize];
    let seeded = Instant::now();
    for start in (0..docs).step_by(10_000) {
        let batch: Vec<_> = (start..(start + 10_000).min(docs))
            .map(|i| {
                per_customer[(mix(1, i) % customers) as usize] += 1;
                compare_order(i, customers)
            })
            .collect();
        orders.insert_batch(batch).unwrap();
    }
    engine.flush().unwrap();
    let sample = orders.get_value("o0000000").unwrap();
    let mut data = sample.as_object().unwrap().clone();
    data.retain(|k, _| !k.starts_with('_'));
    println!(
        "{}: seeded {docs} orders, {customers} customers in {:?}; one is {} B of JSON",
        engine.engine_kind(),
        seeded.elapsed(),
        serde_json::to_string(&data).unwrap().len(),
    );

    // In process, one thread.
    let q = "FOR o IN orders FILTER o.customer_id == @c RETURN o";
    let ast = solidb::sdbql::parse(q).unwrap();
    let customer_of = |r: u64| format!("c{:05}", mix(7, r) % customers);
    let stage = |label: &str, f: &mut dyn FnMut(&str) -> usize| {
        let n = 20_000u64;
        for r in 0..2_000 {
            f(&customer_of(r + 1_000_000));
        }
        let start = thread_cpu_us();
        let mut rows = 0;
        for r in 0..n {
            rows += f(&customer_of(r));
        }
        let us = thread_cpu_us() - start;
        println!(
            "BENCH|in process|{label}|{:.2} us/query|{:.2} us/row|{:.1} rows/query",
            us / n as f64,
            us / rows.max(1) as f64,
            rows as f64 / n as f64
        );
    };
    let series = std::env::var("SOLIDB_BENCH_SERIES")
        .unwrap_or_else(|_| "inproc,phases,by_customer,get,recent,join".to_string());
    let runs = |name: &str| series.split(',').any(|s| s == name);
    let field = "customer_id";
    let binds = |c: &str| -> std::collections::HashMap<String, serde_json::Value> {
        [("c".to_string(), json!(c))].into_iter().collect()
    };
    if runs("inproc") {
        stage("index seek alone (count)", &mut |c| {
            orders
                .index_count_eq(field, &json!(c), || Ok(()))
                .unwrap()
                .unwrap() as usize
        });
        stage("+ multi_get, _key copied", &mut |c| {
            let shape = RawShape::Single("_key".to_string());
            let r = orders.index_raw_eq(field, &json!(c), 0, usize::MAX, &shape, |_| Ok(()));
            r.unwrap().unwrap().0
        });
        stage("+ whole documents copied", &mut |c| {
            let r =
                orders.index_raw_eq(
                    field,
                    &json!(c),
                    0,
                    usize::MAX,
                    &RawShape::Whole,
                    |_| Ok(()),
                );
            r.unwrap().unwrap().0
        });
        stage("executor, raw rows", &mut |c| {
            let executor = QueryExecutor::with_database_and_bind_vars(
                &engine,
                "bench_db".to_string(),
                binds(c),
            );
            let raw = executor
                .execute_raw_msgpack(&ast)
                .unwrap()
                .expect("raw path");
            rmp::decode::read_array_len(&mut raw.as_slice()).unwrap() as usize
        });
        stage("executor, ordinary path", &mut |c| {
            let executor = QueryExecutor::with_database_and_bind_vars(
                &engine,
                "bench_db".to_string(),
                binds(c),
            );
            executor.execute(&ast).unwrap().len()
        });
    }

    // Over the driver, on loopback TCP.
    engine.create_database("_system".to_string()).unwrap();
    solidb::server::auth::AuthService::init(&engine, None, tmp.path().to_str().unwrap()).unwrap();
    let server = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(server_threads)
        .thread_name("bench-server")
        // Blocking-pool threads must outlive a series for their CPU to count.
        .thread_keep_alive(std::time::Duration::from_secs(3600))
        .enable_all()
        .build()
        .unwrap();
    let storage = std::sync::Arc::new(engine.clone());
    let listener = server
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    server.spawn(async move {
        use tokio::io::AsyncReadExt;
        loop {
            let (mut stream, peer) = listener.accept().await.unwrap();
            stream.set_nodelay(true).unwrap();
            let storage = storage.clone();
            tokio::spawn(async move {
                let mut magic = [0u8; 14];
                stream.read_exact(&mut magic).await.unwrap();
                let mut handler = solidb::driver::DriverHandler::new(storage, None);
                handler.handle_connection(stream, peer.to_string()).await;
            });
        }
    });
    let client = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .thread_name("bench-client")
        .enable_all()
        .build()
        .unwrap();

    // Every answer checked before anything is timed.
    client.block_on(async {
        let mut c = solidb_client::SoliDBClientBuilder::new(&addr)
            .auth("bench_db", "admin", "bench-password")
            .pool_size(1)
            .build()
            .await
            .unwrap();
        for r in 0..200 {
            let id = mix(7, r) % customers;
            let vars = binds(&format!("c{id:05}"));
            let rows = c
                .query_with_cache("bench_db", q, Some(vars), false)
                .await
                .unwrap();
            assert_eq!(rows.len(), per_customer[id as usize] as usize, "c{id:05}");
        }
    });

    // `recent` and `join` of `benchmarks/compare`: the 20 most recent orders
    // of a one-day window (about `docs / 336` orders), alone and with their
    // customers.
    const Q_RECENT: &str = "FOR o IN orders FILTER o.created_at >= @lo AND o.created_at < @hi \
         SORT o.created_at DESC LIMIT 20 RETURN o";
    const Q_JOIN: &str = "FOR o IN orders FILTER o.created_at >= @lo AND o.created_at < @hi \
         SORT o.created_at DESC LIMIT 20 \
         LET c = DOCUMENT(CONCAT(\"customers/\", o.customer_id)) \
         RETURN {order: o._key, amount: o.amount, name: c.name, tier: c.tier}";
    let window = |r: u64| -> std::collections::HashMap<String, serde_json::Value> {
        let day = format!("2025-{:02}-{:02}", 1 + r % 12, 1 + (r / 12) % 28);
        [
            ("lo".to_string(), json!(format!("{day}T00:00:00Z"))),
            ("hi".to_string(), json!(format!("{day}U"))),
        ]
        .into_iter()
        .collect()
    };
    if runs("inproc") {
        for (label, q) in [("recent", Q_RECENT), ("join", Q_JOIN)] {
            let ast = solidb::sdbql::parse(q).unwrap();
            let n = 2_000u64;
            let start = thread_cpu_us();
            let mut rows = 0;
            for r in 0..n {
                let executor = QueryExecutor::with_database_and_bind_vars(
                    &engine,
                    "bench_db".to_string(),
                    window(r),
                );
                rows += match executor.execute_raw_msgpack(&ast).unwrap() {
                    Some(raw) => rmp::decode::read_array_len(&mut raw.as_slice()).unwrap() as usize,
                    None => executor.execute(&ast).unwrap().len(),
                };
            }
            let us = thread_cpu_us() - start;
            println!(
                "BENCH|in process|{label}, as the driver runs it|{:.2} us/query|{:.2} us/row",
                us / n as f64,
                us / rows.max(1) as f64
            );
        }
    }
    client.block_on(async {
        let mut c = solidb_client::SoliDBClientBuilder::new(&addr)
            .auth("bench_db", "admin", "bench-password")
            .pool_size(1)
            .build()
            .await
            .unwrap();
        for r in 0..50 {
            let rows = c
                .query_with_cache("bench_db", Q_RECENT, Some(window(r)), false)
                .await
                .unwrap();
            assert_eq!(rows.len(), 20, "recent");
            let created: Vec<&str> = rows
                .iter()
                .map(|o| o["created_at"].as_str().unwrap())
                .collect();
            assert!(created.windows(2).all(|w| w[0] >= w[1]), "newest first");
            let rows = c
                .query_with_cache("bench_db", Q_JOIN, Some(window(r)), false)
                .await
                .unwrap();
            assert_eq!(
                rows.iter().filter(|r| r["name"].is_string()).count(),
                20,
                "join"
            );
        }
    });

    if runs("phases") {
        let binds_of =
            move |shape: &str, r: u64| -> std::collections::HashMap<String, serde_json::Value> {
                if shape == "by_customer" {
                    return [("c".to_string(), json!(format!("c{:05}", r % customers)))]
                        .into_iter()
                        .collect();
                }
                window(r)
            };
        phase_split(
            &engine,
            &[
                ("by_customer", q, &binds_of),
                ("recent", Q_RECENT, &binds_of),
                ("join", Q_JOIN, &binds_of),
            ],
            docs,
        );
    }

    let warm = std::time::Duration::from_secs(1);
    let concs: Vec<usize> = std::env::var("SOLIDB_BENCH_CONC")
        .unwrap_or_else(|_| "1,32".to_string())
        .split(',')
        .filter_map(|c| c.parse().ok())
        .collect();
    for conc in concs {
        if runs("by_customer") {
            let series = driver_series(&client, &addr, conc, warm, secs, move |mut c, r| {
                let id = format!("c{:05}", r % customers);
                async move {
                    let vars = [("c".to_string(), json!(id))].into_iter().collect();
                    c.query_with_cache("bench_db", q, Some(vars), false)
                        .await
                        .unwrap();
                    c
                }
            });
            println!("BENCH|driver|by_customer|c={conc}|{series}");
        }
        if runs("get") {
            let series = driver_series(&client, &addr, conc, warm, secs, move |mut c, r| {
                let key = format!("o{:07}", r % docs);
                async move {
                    c.get("bench_db", "orders", &key).await.unwrap();
                    c
                }
            });
            println!("BENCH|driver|get|c={conc}|{series}");
        }
        for (label, q) in [("recent", Q_RECENT), ("join", Q_JOIN)] {
            if !runs(label) {
                continue;
            }
            let series = driver_series(
                &client,
                &addr,
                conc,
                warm,
                secs,
                move |mut c, r| async move {
                    let day = format!("2025-{:02}-{:02}", 1 + r % 12, 1 + (r / 12) % 28);
                    let vars = [
                        ("lo".to_string(), json!(format!("{day}T00:00:00Z"))),
                        ("hi".to_string(), json!(format!("{day}U"))),
                    ]
                    .into_iter()
                    .collect();
                    c.query_with_cache("bench_db", q, Some(vars), false)
                        .await
                        .unwrap();
                    c
                },
            );
            println!("BENCH|driver|{label}|c={conc}|{series}");
        }
    }
    println!(
        "block cache: {} MiB used",
        engine.memory_breakdown().block_cache_bytes >> 20
    );
    client.shutdown_background();
    server.shutdown_background();
}

/// Bonfire card #723, on `ORDERS` orders whose `created_at` (ISO strings)
/// spreads over two years, behind the sparse persistent index the driver
/// creates: the one-day window read newest first (`SORT … DESC LIMIT 20`),
/// the same window returned whole, and the window joined to its customers.
/// `SOLIDB_BENCH_ORDERS` overrides the 1,000,000 orders.
///
/// On 2.7.2 the window was read from one bound — every order before
/// `@hi`, 498,630 of them — and then sorted: about 2.2 s for each shape.
/// The window is now read as one interval (1,370 entries, 5.3 ms whole),
/// and the SORT + LIMIT shapes walk it in index order and stop at the 20th
/// row (0.1 ms; 0.18 ms joined).
#[test]
#[ignore]
fn benchmark_range_window() {
    let orders_n: usize = std::env::var("SOLIDB_BENCH_ORDERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000);
    const CUSTOMERS: usize = 10_000;
    const TWO_YEARS_SECS: i64 = 730 * 86_400;

    let (engine, _tmp) = create_test_db();
    let db = engine.get_database("bench_db").unwrap();
    for c in ["orders", "customers"] {
        db.create_collection(c.to_string(), None).unwrap();
    }
    let customers = db.get_collection("customers").unwrap();
    customers
        .insert_batch(
            (0..CUSTOMERS)
                .map(|i| {
                    json!({
                        "_key": format!("c{i}"),
                        "name": format!("Customer {i}"),
                        "tier": (["gold", "silver", "bronze"][i % 3]),
                    })
                })
                .collect(),
        )
        .unwrap();
    let orders = db.get_collection("orders").unwrap();
    orders
        .create_index(
            "idx_created_at".to_string(),
            vec!["created_at".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();
    let base = chrono::DateTime::parse_from_rfc3339("2023-06-01T00:00:00Z")
        .unwrap()
        .timestamp();
    let seeded = Instant::now();
    for start in (0..orders_n).step_by(10_000) {
        let batch: Vec<_> = (start..(start + 10_000).min(orders_n))
            .map(|i| {
                // A permutation of the slots, so key order is not time order.
                let slot = (i as i64 * 7_919) % orders_n as i64;
                let t = base + slot * TWO_YEARS_SECS / orders_n as i64;
                json!({
                    "customer_id": format!("c{}", i % CUSTOMERS),
                    "status": (["open", "paid", "shipped", "cancelled"][i % 4]),
                    "amount": (i % 997) as f64 + 0.5,
                    "created_at": chrono::DateTime::from_timestamp(t, 0)
                        .unwrap()
                        .format("%Y-%m-%dT%H:%M:%SZ")
                        .to_string(),
                    "padding": "x".repeat(100),
                })
            })
            .collect();
        orders.insert_batch(batch).unwrap();
    }
    println!(
        "seeded {orders_n} orders and {CUSTOMERS} customers in {:?}",
        seeded.elapsed()
    );

    let window = "FILTER o.created_at >= @lo AND o.created_at < @hi";
    let queries = [
        (
            "window, SORT DESC LIMIT 20",
            format!("FOR o IN orders {window} SORT o.created_at DESC LIMIT 20 RETURN o"),
        ),
        (
            "window, SORT ASC LIMIT 20",
            format!("FOR o IN orders {window} SORT o.created_at ASC LIMIT 20 RETURN o"),
        ),
        (
            "window, every row",
            format!("FOR o IN orders {window} RETURN o"),
        ),
        (
            "window, joined",
            format!(
                "FOR o IN orders {window} SORT o.created_at DESC LIMIT 20 \
                 LET c = DOCUMENT(CONCAT(\"customers/\", o.customer_id)) \
                 RETURN {{order: o._key, amount: o.amount, name: c.name, tier: c.tier}}"
            ),
        ),
        (
            "since @lo, SORT DESC LIMIT 20",
            "FOR o IN orders FILTER o.created_at >= @lo SORT o.created_at DESC LIMIT 20 RETURN o"
                .to_string(),
        ),
    ];
    let binds: std::collections::HashMap<String, serde_json::Value> = [
        ("lo".to_string(), json!("2024-06-01T00:00:00Z")),
        ("hi".to_string(), json!("2024-06-02T00:00:00Z")),
    ]
    .into_iter()
    .collect();
    for (label, q) in &queries {
        let ast = solidb::sdbql::parse(q).unwrap();
        let executor = QueryExecutor::with_database_and_bind_vars(
            &engine,
            "bench_db".to_string(),
            binds.clone(),
        );
        let e = executor.explain(&ast).unwrap();
        let access = &e.collections[0];
        let plan = format!(
            "{} {} est={:?} scanned={} returned={}",
            access.access_type,
            access.index_used.as_deref().unwrap_or("-"),
            access.estimated_rows,
            e.documents_scanned,
            e.documents_returned,
        );
        executor.execute(&ast).unwrap();
        let mut times = Vec::new();
        let mut rows = 0;
        for _ in 0..7 {
            let start = Instant::now();
            rows = executor.execute(&ast).unwrap().len();
            times.push(start.elapsed());
        }
        times.sort();
        println!(
            "BENCH|{label}|{rows}|{plan}|median {:.3} ms",
            times[times.len() / 2].as_secs_f64() * 1e3
        );
    }
}

/// `orders` as in the full-scan aggregate report (Bonfire #725): about 500 B
/// of JSON each, with a status (5 values), a float amount, a customer id, a
/// date, three tags, three nested line items and an address object.
fn seed_orders(engine: &StorageEngine, count: usize) {
    let db = engine.get_database("bench_db").unwrap();
    db.create_collection("orders".to_string(), None).unwrap();
    let orders = db.get_collection("orders").unwrap();
    let statuses = ["pending", "paid", "shipped", "delivered", "cancelled"];
    let cities = ["Paris", "Lyon", "Marseille", "Toulouse", "Bordeaux"];
    for start in (0..count).step_by(10_000) {
        let batch: Vec<_> = (start..(start + 10_000).min(count))
            .map(|i| {
                let item = |n: usize, price: f64| {
                    json!({
                        "sku": format!("SKU-{:05}", (i * 31 + n) % 10_000),
                        "name": format!("Product {}", (i + n) % 997),
                        "qty": 1 + (i + n) % 4,
                        "price": price,
                    })
                };
                json!({
                    "status": statuses[i % 5],
                    "amount": ((i * 7919) % 100_000) as f64 / 100.0,
                    "customer_id": format!("cust-{:06}", i % 20_000),
                    "created_at": format!(
                        "2026-{:02}-{:02}T{:02}:{:02}:00Z",
                        1 + i % 12, 1 + i % 28, i % 24, i % 60
                    ),
                    "tags": ["priority", format!("region-{}", i % 7), format!("channel-{}", i % 3)],
                    "items": [item(0, 9.99), item(1, 24.5), item(2, 3.25)],
                    "address": {
                        "street": format!("{} rue de la République", i % 900),
                        "city": cities[i % 5],
                        "zip": format!("{:05}", 75_000 + i % 1000),
                        "country": "FR",
                    },
                })
            })
            .collect();
        orders.insert_batch(batch).unwrap();
    }
}

/// Peak resident set size since the last reset, in MiB (Linux only).
fn peak_rss_mib() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kib: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib / 1024.0)
}

/// Reset the peak RSS to the current RSS (Linux only), after handing freed
/// heap back to the system: glibc keeps it, and a query would otherwise
/// reuse what the previous one freed without raising the peak.
fn reset_peak_rss() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim only releases free heap pages; no pointer is involved.
    unsafe {
        libc::malloc_trim(0);
    }
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

/// The full-scan COLLECT shapes, every way they can run, on the engine
/// `SOLIDB_ENGINE` selects (RocksDB unless it says `solistore`):
///
/// - materialised (`with_streaming_collect(false)`, the path before 2.7.3);
/// - streamed, decoding the fields read (`with_typed_collect(false)`, the
///   2.7.3 path, now over borrowed storage bytes and the allocation-free fold);
/// - streamed with typed reads, on one thread;
/// - the same split across threads (`BENCH_THREADS`, default 4), at one
///   snapshot (both engines; solistore from 0.3, cut at its split points).
///
/// Median of three runs after one warm-up, and the peak RSS of a run of each;
/// beside the cost of the scan alone (stored bytes only, then whole documents
/// decoded, then only `status` and `amount`). All the ways must agree.
fn bench_full_scan_collect(docs: usize) {
    let (engine, _tmp) = create_test_db();
    let seed_start = Instant::now();
    seed_orders(&engine, docs);
    let orders = engine
        .get_database("bench_db")
        .unwrap()
        .get_collection("orders")
        .unwrap();
    let sample = orders.scan_values(Some(1)).remove(0);
    let mut data = sample.as_object().unwrap().clone();
    data.retain(|k, _| !k.starts_with('_'));
    let engine_name = std::env::var("SOLIDB_ENGINE").unwrap_or_else(|_| "rocksdb".to_string());
    let threads: usize = std::env::var("BENCH_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    println!(
        "seeded {docs} orders on {engine_name} in {:?}; one is {} B of JSON without system \
         fields; parallel scans {}",
        seed_start.elapsed(),
        serde_json::to_string(&data).unwrap().len(),
        if orders.supports_parallel_scan() {
            "available"
        } else {
            "unavailable (no snapshot)"
        }
    );

    let median = |f: &mut dyn FnMut()| -> f64 {
        f();
        let mut times: Vec<f64> = (0..3)
            .map(|_| {
                let start = Instant::now();
                f();
                start.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        times[1]
    };
    let fields = vec!["amount".to_string(), "status".to_string()];
    let bytes_only = median(&mut || {
        orders.for_each_doc_bytes(|_| Ok(true)).unwrap();
    });
    let whole = median(&mut || {
        orders.for_each_value(None, |_| Ok(())).unwrap();
    });
    let projected = median(&mut || {
        orders.for_each_value(Some(&fields), |_| Ok(())).unwrap();
    });
    let per_doc = |ms: f64| ms * 1e3 / docs as f64;
    for (label, ms) in [
        ("scan alone, stored bytes only", bytes_only),
        ("scan alone, whole documents decoded", whole),
        ("scan alone, status + amount decoded", projected),
    ] {
        println!(
            "BENCH|{engine_name}|{label}|{docs}|{ms:.1} ms|{:.3} us/doc",
            per_doc(ms)
        );
    }

    let modes: [(&str, bool, bool, usize); 4] = [
        ("materialised", false, false, 1),
        ("streamed, decoded", true, false, 1),
        ("typed, 1 thread", true, true, 1),
        ("typed, parallel", true, true, threads),
    ];
    for (label, q) in [
        (
            "GROUP BY status, COUNT + SUM",
            "FOR o IN orders COLLECT status = o.status \
             AGGREGATE n = COUNT(), total = SUM(o.amount) RETURN {status, n, total}",
        ),
        (
            "FILTER amount > 500, GROUP BY status",
            "FOR o IN orders FILTER o.amount > 500 COLLECT status = o.status \
             AGGREGATE n = COUNT(), total = SUM(o.amount) RETURN {status, n, total}",
        ),
        (
            "COLLECT WITH COUNT INTO",
            "FOR o IN orders COLLECT WITH COUNT INTO n RETURN n",
        ),
    ] {
        let ast = solidb::sdbql::parse(q).unwrap();
        let mut results = Vec::new();
        let mut line = format!("BENCH|{engine_name}|{label}|{docs}");
        for (mode, stream, typed, threads) in modes {
            let executor = QueryExecutor::with_database(&engine, "bench_db".to_string())
                .with_streaming_collect(stream)
                .with_typed_collect(typed)
                .with_scan_threads(threads, 16_384);
            reset_peak_rss();
            let before = peak_rss_mib();
            results.push(executor.execute(&ast).unwrap());
            let peak = match (before, peak_rss_mib()) {
                (Some(b), Some(p)) => format!("+{:.0} MiB peak", p - b),
                _ => "peak n/a".to_string(),
            };
            let ms = median(&mut || {
                executor.execute(&ast).unwrap();
            });
            // "…; 4 key ranges read in parallel)" when it was split.
            let shape = executor
                .explain(&ast)
                .unwrap()
                .warnings
                .iter()
                .find_map(|w| {
                    let (head, _) = w.split_once(" key ranges read in parallel")?;
                    head.rsplit(' ').next().map(|n| format!(", {n} ranges"))
                })
                .unwrap_or_default();
            line.push_str(&format!(
                "|{mode}: {ms:.1} ms ({:.3} us/doc, {peak}{shape})",
                per_doc(ms)
            ));
        }
        for r in &results[1..] {
            assert_eq!(&results[0], r, "{label}");
        }
        println!("{line}");
    }
}

#[test]
#[ignore]
fn benchmark_full_scan_collect() {
    bench_full_scan_collect(100_000);
}

#[test]
#[ignore]
fn benchmark_full_scan_collect_1m() {
    bench_full_scan_collect(1_000_000);
}
