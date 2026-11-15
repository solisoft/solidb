//! A columnar collection stores its rows in blocks and a tail
//! (`storage::columnar_blocks`). Whatever the path a row took — a large
//! insert sealed at once, small inserts sealed when enough waited, a
//! replicated row with an older id, a restart — reading gives back the
//! values that were inserted, in the order of the row ids.

use serde_json::{json, Value};
use solidb::storage::columnar::{
    AggregateOp, ColumnDef, ColumnType, ColumnarCollection, CompressionType,
};
use solidb::storage::StorageEngine;
use tempfile::TempDir;

const COLUMNS: [(&str, ColumnType); 6] = [
    ("id", ColumnType::Int64),
    ("name", ColumnType::String),
    ("score", ColumnType::Float64),
    ("active", ColumnType::Bool),
    ("extra", ColumnType::Json),
    ("mixed", ColumnType::Json),
];

fn names() -> Vec<&'static str> {
    COLUMNS.iter().map(|(name, _)| *name).collect()
}

fn open(tmp: &TempDir) -> StorageEngine {
    StorageEngine::new(tmp.path()).unwrap()
}

fn create(engine: &StorageEngine, compression: CompressionType) -> ColumnarCollection {
    engine.create_database("_system".to_string()).unwrap();
    let db = engine.get_database("_system").unwrap();
    db.create_collection("_columnar_events".to_string(), None)
        .unwrap();
    let columns = COLUMNS
        .iter()
        .map(|(name, data_type)| ColumnDef {
            name: name.to_string(),
            data_type: data_type.clone(),
            nullable: true,
            indexed: false,
            index_type: None,
        })
        .collect();
    ColumnarCollection::new(
        "events".to_string(),
        "_system",
        db.db_arc(),
        columns,
        compression,
    )
    .unwrap()
}

fn load(engine: &StorageEngine) -> ColumnarCollection {
    let db = engine.get_database("_system").unwrap();
    ColumnarCollection::load("events".to_string(), "_system", db.db_arc()).unwrap()
}

/// Row `i`: every kind of value, nulls, a missing field, a column of
/// several kinds.
fn row(i: i64) -> Value {
    let mut row = json!({
        "id": i,
        "name": format!("n{:03}", i % 37),
        "score": (i * 7 % 1000) as f64 / 3.0,
        "active": i % 3 == 0,
        "extra": {"tags": [i % 5, "x"], "deep": {"k": i}},
        "mixed": match i % 4 { 0 => json!(i), 1 => json!(i as f64 + 0.5), 2 => json!("s"), _ => json!(null) },
    });
    if i % 11 == 0 {
        row["score"] = Value::Null;
    }
    if i % 13 == 0 {
        row.as_object_mut().unwrap().remove("name");
    }
    row
}

/// What reading row `i` gives: a missing field is null.
fn read_back(i: i64) -> Value {
    let mut row = row(i);
    if i % 13 == 0 {
        row["name"] = Value::Null;
    }
    row
}

fn all(col: &ColumnarCollection) -> Vec<Value> {
    col.read_columns(&names(), None).unwrap()
}

/// `got` is `want`; on a difference, only the first row that differs is
/// printed.
fn same(got: Vec<Value>, want: &[Value]) {
    let first = got.iter().zip(want).position(|(g, w)| g != w);
    if let Some(at) = first {
        panic!("row {at}: {} instead of {}", got[at], want[at]);
    }
    assert_eq!(got.len(), want.len());
}

#[test]
fn a_large_insert_is_read_back_as_it_was_written() {
    for compression in [CompressionType::Lz4, CompressionType::None] {
        let tmp = TempDir::new().unwrap();
        let engine = open(&tmp);
        let col = create(&engine, compression);
        let ids = col.insert_rows((0..10_000).map(row).collect()).unwrap();
        assert_eq!(ids.len(), 10_000);
        let meta = col.metadata().unwrap();
        assert_eq!(
            (meta.row_count, meta.tail_rows),
            (10_000, 0),
            "sealed at once"
        );

        let expected: Vec<Value> = (0..10_000).map(read_back).collect();
        same(all(&col), &expected);
        assert_eq!(col.list_row_uuids().unwrap(), ids);
        assert_eq!(col.count(), 10_000);

        // By id, a single column, and the aggregates.
        let some = [
            ids[9_999].clone(),
            ids[0].clone(),
            "no-such-row".to_string(),
        ];
        assert_eq!(
            col.read_column("id", Some(&some)).unwrap(),
            vec![json!(9_999), json!(0), Value::Null]
        );
        assert_eq!(
            col.aggregate("id", AggregateOp::Sum).unwrap(),
            json!((0..10_000i64).sum::<i64>() as f64)
        );
        assert_eq!(
            col.aggregate("score", AggregateOp::Count).unwrap(),
            json!(10_000)
        );
        assert_eq!(
            col.aggregate("id", AggregateOp::Max).unwrap(),
            json!(9_999.0)
        );
        assert_eq!(
            col.aggregate("name", AggregateOp::CountDistinct).unwrap(),
            json!(38)
        );

        // Another handle, then another process, read the same.
        same(all(&load(&engine)), &expected);
        drop(col);
        drop(engine);
        let engine = open(&tmp);
        same(all(&load(&engine)), &expected);
        assert_eq!(load(&engine).list_row_uuids().unwrap(), ids);
    }
}

#[test]
fn small_inserts_wait_in_the_tail_then_are_sealed() {
    let tmp = TempDir::new().unwrap();
    let engine = open(&tmp);
    let col = create(&engine, CompressionType::Lz4);
    let mut ids = Vec::new();
    for batch in 0..45 {
        ids.extend(
            col.insert_rows((batch * 100..batch * 100 + 100).map(row).collect())
                .unwrap(),
        );
        let meta = col.metadata().unwrap();
        assert_eq!(meta.row_count, (batch as u64 + 1) * 100);
        // The tail is sealed when it reaches 4,096 rows.
        assert_eq!(meta.tail_rows, (batch as u64 + 1) * 100 % 4_100);
    }
    let expected: Vec<Value> = (0..4_500).map(read_back).collect();
    same(all(&col), &expected);
    assert_eq!(col.list_row_uuids().unwrap(), ids);
    drop(col);
    drop(engine);
    let engine = open(&tmp);
    same(all(&load(&engine)), &expected);
}

#[test]
fn rows_with_older_ids_and_deleted_rows() {
    let tmp = TempDir::new().unwrap();
    let engine = open(&tmp);
    let col = create(&engine, CompressionType::Lz4);
    let ids = col.insert_rows((0..5_000).map(row).collect()).unwrap();
    col.insert_rows((5_000..5_010).map(row).collect()).unwrap();

    // A replicated row whose id sorts before every other, and one that
    // sorts inside the block: both are read where their ids put them.
    let first = "00000000-0000-7000-8000-000000000000";
    assert!(col.insert_row_with_id(first, row(-1)).unwrap());
    assert!(
        !col.insert_row_with_id(first, row(-2)).unwrap(),
        "already there"
    );
    assert!(
        !col.insert_row_with_id(&ids[17], row(-3)).unwrap(),
        "already there"
    );
    let between = format!("{}x", ids[2_000]);
    assert!(col.insert_row_with_id(&between, row(-4)).unwrap());

    let mut expected: Vec<Value> = vec![read_back(-1)];
    expected.extend((0..=2_000).map(read_back));
    expected.push(read_back(-4));
    expected.extend((2_001..5_010).map(read_back));
    same(all(&col), &expected);
    assert_eq!(col.count(), 5_012);

    // Deleted from the block, from the tail, and one that is not there.
    assert!(col.delete_row(&ids[3]).unwrap());
    assert!(col.delete_row(first).unwrap());
    assert!(!col.delete_row(first).unwrap());
    expected.remove(4);
    expected.remove(0);
    same(all(&col), &expected);
    assert_eq!(col.count(), 5_010);
    same(all(&load(&engine)), &expected);

    drop(col);
    drop(engine);
    let engine = open(&tmp);
    let col = load(&engine);
    same(all(&col), &expected);

    col.truncate().unwrap();
    assert!(all(&col).is_empty());
    assert_eq!(col.metadata().unwrap().tail_rows, 0);
    col.insert_rows((0..3).map(row).collect()).unwrap();
    assert_eq!(all(&col), (0..3).map(read_back).collect::<Vec<_>>());
}

/// `FOR … FILTER … COLLECT … AGGREGATE` over a columnar collection is
/// folded from the arrays of its blocks (`execution::columnar_scan`): the
/// same answers as building every row and grouping the objects, over
/// blocks of plain columns, a block with nulls and mixed kinds, and a tail.
#[test]
fn a_collect_over_the_blocks_answers_what_the_rows_answer() {
    use solidb::{parse, BindVars, QueryExecutor};
    let tmp = TempDir::new().unwrap();
    let engine = open(&tmp);
    engine.create_database("_system".to_string()).unwrap();
    let db = engine.get_database("_system").unwrap();
    db.create_collection("_columnar_sales".to_string(), None)
        .unwrap();
    let columns = [
        ("ts", ColumnType::Int64),
        ("country", ColumnType::String),
        ("device", ColumnType::String),
        ("user", ColumnType::Int64),
        ("amount", ColumnType::Float64),
        ("bytes", ColumnType::Int64),
        ("paid", ColumnType::Bool),
    ]
    .iter()
    .map(|(name, data_type)| ColumnDef {
        name: name.to_string(),
        data_type: data_type.clone(),
        nullable: true,
        indexed: false,
        index_type: None,
    })
    .collect();
    let col = ColumnarCollection::new(
        "sales".to_string(),
        "_system",
        db.db_arc(),
        columns,
        CompressionType::Lz4,
    )
    .unwrap();
    const DEVICES: [&str; 3] = ["ios", "android", "web"];
    let sale = |i: i64| {
        json!({
            "ts": 1_000 + i,
            "country": format!("c{:02}", (i * 7) % 23),
            "device": DEVICES[(i % 3) as usize],
            "user": (i * 31) % 500,
            // Thirds: sums that round.
            "amount": ((i * 13) % 997) as f64 / 3.0,
            "bytes": (i * 17) % 10_000,
            "paid": i % 5 != 0,
        })
    };
    // Two blocks of plain columns.
    col.insert_rows((0..6_000).map(sale).collect()).unwrap();
    col.insert_rows((6_000..11_000).map(sale).collect())
        .unwrap();
    // A block with nulls, an integer among the floats, a missing field.
    col.insert_rows(
        (11_000..15_200)
            .map(|i| {
                let mut row = sale(i);
                match i % 9 {
                    0 => row["amount"] = Value::Null,
                    1 => row["amount"] = json!(i % 50),
                    2 => row["country"] = Value::Null,
                    3 => {
                        row.as_object_mut().unwrap().remove("bytes");
                    }
                    _ => {}
                }
                row
            })
            .collect(),
    )
    .unwrap();
    // And a tail.
    col.insert_rows((15_200..15_300).map(sale).collect())
        .unwrap();
    assert_eq!(col.metadata().unwrap().tail_rows, 100);

    let queries = [
        "FOR r IN sales FILTER r.country == 'c07' COLLECT AGGREGATE n = COUNT(), s = SUM(r.amount) \
         RETURN [n, s]",
        "FOR r IN sales COLLECT c = r.country AGGREGATE n = COUNT(), b = SUM(r.bytes), a = AVG(r.amount) \
         RETURN [c, n, b, a]",
        "FOR r IN sales FILTER r.ts >= 3000 AND r.ts < 12500 COLLECT d = r.device \
         AGGREGATE n = COUNT(), b = SUM(r.bytes), lo = MIN(r.amount), hi = MAX(r.amount) \
         RETURN [d, n, b, lo, hi]",
        "FOR r IN sales COLLECT u = r.user AGGREGATE s = SUM(r.amount) SORT s DESC, u LIMIT 10 \
         RETURN [u, s]",
        "FOR r IN sales FILTER r.paid == true AND r.bytes > 5000.5 COLLECT c = r.country, d = r.device \
         AGGREGATE n = COUNT(r.amount), m = MAX(r.bytes) RETURN [c, d, n, m]",
        "FOR r IN sales FILTER r.amount <= 100 AND r.device == 'web' COLLECT p = r.paid \
         AGGREGATE n = COUNT(), s = SUM(r.amount), c = COUNT(r.country) RETURN [p, n, s, c]",
        "FOR r IN sales FILTER r.country == 'nowhere' COLLECT AGGREGATE n = COUNT(), s = SUM(r.bytes) \
         RETURN [n, s]",
        "FOR r IN sales FILTER r.user == 42 COLLECT WITH COUNT INTO n RETURN n",
        "FOR r IN sales COLLECT a = r.amount AGGREGATE n = COUNT() SORT n DESC, a LIMIT 5 RETURN [a, n]",
        // Not the arrays' to answer: the ordinary path, same result.
        "FOR r IN sales FILTER r.country != 'c07' COLLECT d = r.device AGGREGATE n = COUNT() RETURN [d, n]",
        "FOR r IN sales COLLECT d = r.device AGGREGATE m = MEDIAN(r.bytes) RETURN [d, m]",
    ];
    for q in queries {
        let run = |fold: bool| {
            QueryExecutor::with_database_and_bind_vars(
                &engine,
                "_system".to_string(),
                BindVars::new(),
            )
            .with_streaming_collect(fold)
            .execute(&parse(q).unwrap())
            .map_err(|e| e.to_string())
        };
        let folded = run(true);
        assert!(folded.is_ok(), "{q}: {folded:?}");
        assert!(!folded.as_ref().unwrap().is_empty(), "{q}");
        assert_eq!(folded, run(false), "{q}");
        // And on several threads: the blocks shared out, or the keys.
        let threaded = QueryExecutor::with_database_and_bind_vars(
            &engine,
            "_system".to_string(),
            BindVars::new(),
        )
        .with_scan_threads(3, 1_000)
        .execute(&parse(q).unwrap())
        .map_err(|e| e.to_string());
        assert_eq!(threaded, folded, "three threads: {q}");
    }
}

/// Timings of the fold alone, without a server: `cargo test … -- --ignored
/// --nocapture timings_of_the_fold`.
#[test]
#[ignore]
fn timings_of_the_fold() {
    use solidb::{parse, BindVars, QueryExecutor};
    let tmp = TempDir::new().unwrap();
    let engine = open(&tmp);
    engine.create_database("_system".to_string()).unwrap();
    let db = engine.get_database("_system").unwrap();
    db.create_collection("_columnar_events".to_string(), None)
        .unwrap();
    let columns = [
        ("ts", ColumnType::Int64),
        ("user", ColumnType::Int64),
        ("country", ColumnType::String),
        ("device", ColumnType::String),
        ("amount", ColumnType::Float64),
        ("bytes", ColumnType::Int64),
    ]
    .iter()
    .map(|(name, data_type)| ColumnDef {
        name: name.to_string(),
        data_type: data_type.clone(),
        nullable: true,
        indexed: false,
        index_type: None,
    })
    .collect();
    let col = ColumnarCollection::new(
        "events".to_string(),
        "_system",
        db.db_arc(),
        columns,
        CompressionType::Lz4,
    )
    .unwrap();
    let mix = |mut z: u64| {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    };
    const ROWS: u64 = 4_000_000;
    const LETTERS: [&str; 5] = ["a", "b", "c", "d", "e"];
    let began = std::time::Instant::now();
    for from in (0..ROWS).step_by(20_000) {
        let rows = (from..from + 20_000)
            .map(|i| {
                let h = mix(i);
                json!({"ts": i as i64 * 100, "user": mix(h ^ 1) % 100_000,
                       "country": format!("c{:02}", mix(h ^ 2) % 50),
                       "device": LETTERS[(mix(h ^ 3) % 5) as usize],
                       "amount": (mix(h ^ 4) % 40_000) as f64 / 4.0,
                       "bytes": (mix(h ^ 5) % 100_000) as i64})
            })
            .collect();
        col.insert_rows(rows).unwrap();
    }
    println!("loaded {ROWS} rows in {:?}", began.elapsed());
    let queries = [
        ("filter_agg", "FOR r IN events FILTER r.country == 'c07' COLLECT AGGREGATE n = COUNT(), s = SUM(r.amount) RETURN [n, s]"),
        ("group_all", "FOR r IN events COLLECT c = r.country AGGREGATE n = COUNT(), b = SUM(r.bytes) RETURN [c, n, b]"),
        ("window_group", "FOR r IN events FILTER r.ts >= 100000000 AND r.ts < 120000000 COLLECT d = r.device AGGREGATE n = COUNT(), b = SUM(r.bytes) RETURN [d, n, b]"),
        ("top_users", "FOR r IN events COLLECT u = r.user AGGREGATE s = SUM(r.amount) SORT s DESC LIMIT 10 RETURN [u, s]"),
    ];
    for threads in [1usize, 8] {
        for (name, q) in queries {
            let query = parse(q).unwrap();
            let run = || {
                QueryExecutor::with_database_and_bind_vars(
                    &engine,
                    "_system".to_string(),
                    BindVars::new(),
                )
                .with_scan_threads(threads, 32_768)
                .execute(&query)
                .unwrap()
            };
            run();
            let began = std::time::Instant::now();
            let times = 10;
            for _ in 0..times {
                run();
            }
            println!(
                "{name} on {threads} thread(s): {:?} each",
                began.elapsed() / times
            );
        }
    }
    // Four clients at once, each query free to take the threads it finds.
    for (name, q) in queries {
        let query = parse(q).unwrap();
        let began = std::time::Instant::now();
        let times = 20;
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..times {
                        QueryExecutor::with_database_and_bind_vars(
                            &engine,
                            "_system".to_string(),
                            BindVars::new(),
                        )
                        .execute(&query)
                        .unwrap();
                    }
                });
            }
        });
        let each = began.elapsed() / times;
        println!(
            "{name}, 4 clients: {each:?} each, {:.0}/s in all",
            4.0 / each.as_secs_f64()
        );
    }
}
