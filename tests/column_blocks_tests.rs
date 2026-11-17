//! Column blocks (`storage::blocks`): an aggregation over a time series
//! collection that keeps them is folded from the blocks' rows
//! (`execution::block_scan`). Each query is run both ways — with the fold,
//! and by the ordinary path — on the same data, and must give the same
//! answer: after sealing, after every kind of write that throws a block
//! away, after a restart, and over documents that do not fit columns.

use serde_json::{json, Value};
use solidb::storage::blocks::BlocksDef;

/// Enough points for several blocks to be sealed, and merged.
const BLOCK_ROWS: usize = 20_000;
use solidb::storage::StorageEngine;
use solidb::{parse, BindVars, QueryExecutor};
use tempfile::TempDir;

const DB: &str = "db";
const T0: i64 = 1_760_000_100_000;
const SERIES: i64 = 40;
const FOLDED: &str = "COLLECT folded from the column blocks";

fn point(i: i64) -> Value {
    let series = i % SERIES;
    json!({
        "series": format!("s{series:03}"),
        "ts": T0 + (i / SERIES) * 10_000,
        // Floats: whole numbers, fractions a sum holds exactly, and thirds.
        "v": ((i * 7919) % 1000) as f64 / if i % 3 == 0 { 4.0 } else { 3.0 },
        // An integer column: SUM must stay an integer.
        "n": (i * 31) % 17,
        "host": if series % 2 == 0 { "a" } else { "b" },
        // Integers and floats in one field, as JSON metrics often are.
        "m": if i % 2 == 0 { json!(i % 9) } else { json!((i % 9) as f64 + 0.5) },
    })
}

fn open(tmp: &TempDir) -> StorageEngine {
    StorageEngine::new(tmp.path().to_str().unwrap()).unwrap()
}

/// A time series collection of `points` points, its blocks declared.
fn engine_with(points: i64) -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = open(&tmp);
    engine.create_database(DB.to_string()).unwrap();
    let db = engine.get_database(DB).unwrap();
    db.create_collection("points".to_string(), Some("timeseries".to_string()))
        .unwrap();
    let coll = db.get_collection("points").unwrap();
    coll.set_column_blocks(BlocksDef {
        time: "ts".to_string(),
        fields: vec![
            "series".to_string(),
            "host".to_string(),
            "v".to_string(),
            "n".to_string(),
            "m".to_string(),
        ],
    })
    .unwrap();
    for chunk in (0..points).collect::<Vec<_>>().chunks(5_000) {
        coll.insert_batch(chunk.iter().map(|i| point(*i)).collect())
            .unwrap();
    }
    (engine, tmp)
}

fn run(engine: &StorageEngine, q: &str, fold: bool) -> Result<Vec<Value>, String> {
    let query = parse(q).unwrap_or_else(|e| panic!("parse {q}: {e}"));
    QueryExecutor::with_database_and_bind_vars(engine, DB.to_string(), BindVars::new())
        .with_streaming_collect(fold)
        .execute(&query)
        .map_err(|e| e.to_string())
}

fn folded(engine: &StorageEngine, q: &str) -> bool {
    QueryExecutor::with_database_and_bind_vars(engine, DB.to_string(), BindVars::new())
        .explain(&parse(q).unwrap())
        .unwrap()
        .warnings
        .iter()
        .any(|w| w.contains(FOLDED))
}

fn queries() -> Vec<String> {
    let (lo, hi) = (T0 + 600_000, T0 + 4_200_000);
    vec![
        format!(
            "FOR p IN points FILTER p.series == 's007' AND p.ts >= {lo} AND p.ts < {hi} \
             COLLECT AGGREGATE c = COUNT(), t = SUM(p.v), n = SUM(p.n) RETURN [c, t, n]"
        ),
        format!(
            "FOR p IN points FILTER p.series == 's011' AND p.ts >= {lo} AND p.ts < {hi} \
             COLLECT b = TIME_BUCKET(p.ts, \"5m\") AGGREGATE a = AVG(p.v), m = MAX(p.n) \
             RETURN [b, a, m]"
        ),
        format!(
            "FOR p IN points FILTER p.ts >= {lo} AND p.ts < {} \
             COLLECT AGGREGATE c = COUNT(), t = SUM(p.v) RETURN [c, t]",
            lo + 600_000
        ),
        format!(
            "FOR p IN points FILTER p.ts > {lo} AND p.ts <= {hi} AND p.host == 'b' \
             COLLECT s = p.series AGGREGATE lo = MIN(p.v), hi = MAX(p.v), c = COUNT(p.n) \
             RETURN [s, lo, hi, c]"
        ),
        "FOR p IN points COLLECT h = p.host, b = TIME_BUCKET(p.ts, \"1h\") \
         AGGREGATE c = COUNT(), s = SUM(p.n), d = COUNT_DISTINCT(p.series) RETURN [h, b, c, s, d]"
            .to_string(),
        "FOR p IN points FILTER p.series == 'nobody' COLLECT AGGREGATE c = COUNT(), t = SUM(p.v) \
         RETURN [c, t]"
            .to_string(),
        format!(
            "FOR p IN points FILTER p.series == 's003' AND p.host == 'b' AND p.ts < {hi} \
             COLLECT WITH COUNT INTO c RETURN c"
        ),
        "FOR p IN points COLLECT AGGREGATE first = MIN(p.ts), last = MAX(p.ts), c = COUNT() \
         RETURN [first, last, c]"
            .to_string(),
        format!(
            "FOR p IN points FILTER p.ts >= {lo} AND p.ts < {hi} COLLECT AGGREGATE s = SUM(p.m), \
             a = AVG(p.m), lo = MIN(p.m), hi = MAX(p.m), c = COUNT(p.m) RETURN [s, a, lo, hi, c]"
        ),
        format!(
            "FOR p IN points FILTER p.series == 's004' AND p.ts >= {lo} \
             COLLECT b = TIME_BUCKET(p.ts, \"30m\") AGGREGATE s = SUM(p.m), lo = MIN(p.m), \
             hi = MAX(p.m) RETURN [b, s, lo, hi]"
        ),
        // Only the even points of a series: integers alone, an integer sum.
        "FOR p IN points FILTER p.series == 's002' COLLECT AGGREGATE s = SUM(p.m), hi = MAX(p.m) \
         RETURN [s, hi]"
            .to_string(),
    ]
}

/// Every query, both ways; with `expect_fold`, the fold is the path taken.
fn same(engine: &StorageEngine, expect_fold: bool, when: &str) {
    for q in queries() {
        let with = run(engine, &q, true);
        assert_eq!(with, run(engine, &q, false), "{when}: {q}");
        assert!(with.is_ok(), "{when}: {q}: {with:?}");
        assert_eq!(folded(engine, &q), expect_fold, "{when}: {q}");
    }
}

/// Wait for the sealing thread to have sealed all but the newest points
/// (it leaves up to 1,536 of them), and give the state of the blocks.
fn sealed_behind(coll: &solidb::storage::Collection) -> solidb::storage::blocks::BlocksStatus {
    let began = std::time::Instant::now();
    loop {
        let status = coll.column_blocks_status().unwrap();
        if status.sealed_documents + 1_536 > status.documents as u64 {
            return status;
        }
        assert!(
            began.elapsed() < std::time::Duration::from_secs(20),
            "not sealed: {status:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn points_of(engine: &StorageEngine) -> solidb::storage::Collection {
    engine
        .get_database(DB)
        .unwrap()
        .get_collection("points")
        .unwrap()
}

#[test]
fn blocks_answer_what_the_documents_answer() {
    let points = 3 * BLOCK_ROWS as i64 + 5_000;
    let (engine, _tmp) = engine_with(points);
    let coll = points_of(&engine);
    // The writes had the sealing thread seal behind them, all but the
    // newest points; the rest on demand.
    let before = sealed_behind(&coll);
    assert!(before.blocks >= 1, "{before:?}");
    same(&engine, true, "sealed by the writes");

    let sealed = coll.seal_column_blocks().unwrap();
    let status = coll.column_blocks_status().unwrap();
    assert!(sealed >= 1, "{status:?}");
    assert_eq!(status.sealed_documents, points as u64, "{status:?}");
    assert_eq!(status.opaque_blocks, 0, "{status:?}");
    same(&engine, true, "all sealed");
}

/// More documents than two blocks hold, with no block yet: one call seals
/// them all, several blocks at a time on as many threads, and the blocks
/// follow one another.
#[test]
fn a_seal_builds_several_blocks_at_once() {
    let points = 150_000;
    let (engine, _tmp) = engine_with(points);
    let coll = points_of(&engine);
    // What the sealing thread did behind the writes (it too takes several
    // blocks at once when it is that far behind) answers already.
    sealed_behind(&coll);
    same(&engine, true, "sealed behind the writes");

    // Again from nothing, by the call.
    let def = coll.column_blocks_definition().unwrap();
    coll.drop_column_blocks().unwrap();
    coll.set_column_blocks(def).unwrap();
    let sealed = coll.seal_column_blocks().unwrap();
    let status = coll.column_blocks_status().unwrap();
    assert!(sealed >= 3, "{sealed} blocks sealed, {status:?}");
    assert_eq!(status.sealed_documents, points as u64, "{status:?}");
    assert_eq!(status.opaque_blocks, 0, "{status:?}");
    same(&engine, true, "sealed by one call");
}

/// `FILTER p.series == x SORT p.ts DESC LIMIT 1`, as the driver runs it:
/// answered from the blocks when they hold every document and the answer
/// is one row beyond doubt, and always what the index answers.
#[test]
fn the_newest_point_of_a_series_comes_from_the_blocks() {
    use solidb::storage::blocks::NEWEST_FROM_BLOCKS;
    use std::sync::atomic::Ordering;
    let tmp = TempDir::new().unwrap();
    let engine = open(&tmp);
    engine.create_database(DB.to_string()).unwrap();
    let db = engine.get_database(DB).unwrap();
    db.create_collection("last".to_string(), Some("timeseries".to_string()))
        .unwrap();
    let coll = db.get_collection("last").unwrap();
    coll.create_index(
        "by_series_ts".to_string(),
        vec!["series".to_string(), "ts".to_string()],
        solidb::storage::IndexType::Persistent,
        false,
    )
    .unwrap();
    coll.set_column_blocks(BlocksDef {
        time: "ts".to_string(),
        fields: vec!["series".to_string(), "v".to_string(), "m".to_string()],
    })
    .unwrap();
    let row = |i: i64| {
        let mut p = point(i);
        p.as_object_mut().unwrap().remove("host");
        p
    };
    for chunk in (0..30_000i64).collect::<Vec<_>>().chunks(5_000) {
        coll.insert_batch(chunk.iter().map(|i| row(*i)).collect())
            .unwrap();
    }
    // Two points of s007 at its newest instant: which comes first is the
    // keys' business.
    let newest = T0 + (29_999 / SERIES) * 10_000;
    coll.insert(json!({"series": "s007", "ts": newest, "v": -1.0, "m": 1}))
        .unwrap();
    coll.seal_column_blocks().unwrap();

    let both = |q: &str| -> (Vec<Value>, bool) {
        let query = parse(q).unwrap();
        let executor =
            || QueryExecutor::with_database_and_bind_vars(&engine, DB.to_string(), BindVars::new());
        let before = NEWEST_FROM_BLOCKS.load(Ordering::Relaxed);
        let small = executor().execute_small_sorted(&query, 64).unwrap();
        let from_blocks = NEWEST_FROM_BLOCKS.load(Ordering::Relaxed) > before;
        let ordinary = executor().execute(&query).unwrap();
        if let Some(small) = &small {
            assert_eq!(small, &ordinary, "{q}");
        }
        (ordinary, from_blocks)
    };
    let last = |series: &str, ret: &str| {
        format!("FOR p IN last FILTER p.series == '{series}' SORT p.ts DESC LIMIT 1 RETURN {ret}")
    };

    for series in ["s000", "s013", "s039"] {
        let (rows, from_blocks) = both(&last(series, "[p.ts, p.v]"));
        assert_eq!(rows.len(), 1, "{series}");
        assert!(from_blocks, "{series}");
        let (_, from_blocks) = both(&last(series, "{at: p.ts, m: p.m, s: p.series}"));
        assert!(from_blocks, "{series}");
    }
    // A series that is not there: no row, known from the blocks.
    let (rows, from_blocks) = both(&last("nope", "p.ts"));
    assert!(rows.is_empty() && from_blocks);
    // Left to the index: two rows at the newest instant, a field the
    // blocks do not hold, a FILTER the newest row does not pass, the
    // oldest row, two rows.
    for q in [
        last("s007", "[p.ts, p.v]"),
        last("s003", "[p.ts, p.n]"),
        "FOR p IN last FILTER p.series == 's003' AND p.v < -5 SORT p.ts DESC LIMIT 1 RETURN p.ts"
            .to_string(),
        "FOR p IN last FILTER p.series == 's003' SORT p.ts LIMIT 1 RETURN p.ts".to_string(),
        "FOR p IN last FILTER p.series == 's003' SORT p.ts DESC LIMIT 2 RETURN p.ts".to_string(),
    ] {
        let (_, from_blocks) = both(&q);
        assert!(!from_blocks, "{q}");
    }

    // A point written since: the index answers until it is sealed, and
    // the answer is the new point both times.
    let later = newest + 60_000;
    coll.insert(json!({"series": "s013", "ts": later, "v": 2.5, "m": 2.5}))
        .unwrap();
    let (rows, from_blocks) = both(&last("s013", "[p.ts, p.v]"));
    assert_eq!(rows, vec![json!([later, 2.5])]);
    assert!(!from_blocks);
    coll.seal_column_blocks().unwrap();
    let (rows, from_blocks) = both(&last("s013", "[p.ts, p.v, p.m]"));
    assert_eq!(rows, vec![json!([later, 2.5, 2.5])]);
    assert!(from_blocks);
}

#[test]
fn a_write_throws_away_the_block_it_falls_in() {
    let points = 3 * BLOCK_ROWS as i64;
    let (engine, _tmp) = engine_with(points);
    let coll = points_of(&engine);
    coll.seal_column_blocks().unwrap();
    let all = coll.column_blocks_status().unwrap();

    // New points, behind the last block: nothing is thrown away.
    coll.insert_batch((points..points + 500).map(point).collect())
        .unwrap();
    assert_eq!(coll.column_blocks_status().unwrap().blocks, all.blocks);
    same(&engine, true, "after appends");

    // A point whose key sorts before every other: the first block goes.
    let mut late = point(7);
    late["_key"] = json!("00000000-0000-7000-8000-000000000000");
    coll.insert(late).unwrap();
    // The write that threw it away had its range sealed again.
    sealed_behind(&coll);
    same(&engine, true, "after a late point");

    // Sealed again, the late point is in a block.
    coll.seal_column_blocks().unwrap();
    assert_eq!(
        coll.column_blocks_status().unwrap().sealed_documents,
        coll.count() as u64
    );
    same(&engine, true, "sealed again");

    // A delete, then a prune of everything.
    // The block the deleted point was in goes; until its range is sealed
    // again the documents answer, by whichever path.
    coll.delete("00000000-0000-7000-8000-000000000000").unwrap();
    for q in queries() {
        assert_eq!(
            run(&engine, &q, true),
            run(&engine, &q, false),
            "after a delete: {q}"
        );
    }
    coll.seal_column_blocks().unwrap();
    same(&engine, true, "sealed after the delete");
    let deleted = coll.prune_older_than(u64::MAX >> 16).unwrap();
    assert_eq!(deleted, points as usize + 500);
    assert_eq!(coll.column_blocks_status().unwrap().sealed_documents, 0);
    same(&engine, false, "after a prune");
}

#[test]
fn blocks_survive_a_restart_and_are_checked_again() {
    let points = 2 * BLOCK_ROWS as i64 + 100;
    let (engine, tmp) = engine_with(points);
    points_of(&engine).seal_column_blocks().unwrap();
    let before: Vec<_> = queries().iter().map(|q| run(&engine, q, true)).collect();
    drop(engine);

    let engine = open(&tmp);
    let status = points_of(&engine).column_blocks_status().unwrap();
    assert_eq!(status.sealed_documents, points as u64, "{status:?}");
    same(&engine, true, "after a restart");
    let after: Vec<_> = queries().iter().map(|q| run(&engine, q, true)).collect();
    assert_eq!(before, after);
}

#[test]
fn documents_that_do_not_fit_columns_are_read() {
    let points = 2 * BLOCK_ROWS as i64;
    let (engine, _tmp) = engine_with(points);
    let coll = points_of(&engine);
    // A point with no value, a time that is a string, an integer where the
    // others hold floats.
    coll.insert(json!({"series": "s001", "ts": T0 + 50_000, "host": "b", "n": 3}))
        .unwrap();
    coll.insert(
        json!({"series": "s001", "ts": "2025-10-09T08:00:00Z", "v": 1.5, "n": 1, "host": "b"}),
    )
    .unwrap();
    coll.insert(json!({"series": "s007", "ts": T0 + 700_000, "v": 12, "n": 2, "host": "b"}))
        .unwrap();
    coll.seal_column_blocks().unwrap();
    let status = coll.column_blocks_status().unwrap();
    // In one block or two, as the seals behind the writes fell.
    assert!(status.opaque_blocks >= 1, "{status:?}");
    assert!(status.sealed_documents < coll.count() as u64, "{status:?}");
    same(&engine, true, "with an opaque block");
}

#[test]
fn only_a_time_series_collection_takes_blocks() {
    let (engine, _tmp) = engine_with(10);
    let db = engine.get_database(DB).unwrap();
    db.create_collection("plain".to_string(), None).unwrap();
    let def = BlocksDef {
        time: "ts".to_string(),
        fields: vec!["v".to_string()],
    };
    assert!(db
        .get_collection("plain")
        .unwrap()
        .set_column_blocks(def.clone())
        .is_err());
    let coll = points_of(&engine);
    for bad in [
        BlocksDef {
            time: "ts".to_string(),
            fields: vec!["ts".to_string()],
        },
        BlocksDef {
            time: "a.b".to_string(),
            fields: vec![],
        },
        BlocksDef {
            time: "ts".to_string(),
            fields: vec!["v".to_string(), "v".to_string()],
        },
        BlocksDef {
            time: "_key".to_string(),
            fields: vec![],
        },
    ] {
        assert!(coll.set_column_blocks(bad.clone()).is_err(), "{bad:?}");
    }
    // Too little is sealed for the fold to be worth it: the ordinary path.
    same(&engine, false, "ten points");
    coll.drop_column_blocks().unwrap();
    assert!(coll.column_blocks_status().is_none());
    same(&engine, false, "no blocks");
}

/// Keys the caller chose are not ordered by time: the blocks still cover
/// them, and a write among them still throws its block away.
#[test]
fn keys_that_are_not_generated_are_sealed_too() {
    let (engine, _tmp) = engine_with(0);
    let coll = points_of(&engine);
    let keyed = |i: i64| {
        let mut doc = point(i);
        doc["_key"] = json!(format!("p{:06}", i * 2));
        doc
    };
    for chunk in (0..9_000).collect::<Vec<i64>>().chunks(3_000) {
        coll.insert_batch(chunk.iter().map(|i| keyed(*i)).collect())
            .unwrap();
    }
    coll.seal_column_blocks().unwrap();
    let status = coll.column_blocks_status().unwrap();
    assert_eq!(status.sealed_documents, 9_000, "{status:?}");
    same(&engine, true, "chosen keys");

    // Generated keys after them, and one more chosen key between two.
    coll.insert_batch((9_000..12_000).map(point).collect())
        .unwrap();
    let mut between = point(3);
    between["_key"] = json!("p000101");
    coll.insert(between).unwrap();
    for q in queries() {
        assert_eq!(
            run(&engine, &q, true),
            run(&engine, &q, false),
            "mixed keys: {q}"
        );
    }
    coll.seal_column_blocks().unwrap();
    assert_eq!(
        coll.column_blocks_status().unwrap().sealed_documents,
        coll.count() as u64
    );
    same(&engine, true, "mixed keys, sealed");
}

/// Points arriving from several threads while blocks are sealed and merged
/// behind them: nothing is lost and nothing is counted twice.
#[test]
fn concurrent_writes_and_seals_agree_with_the_documents() {
    let (engine, _tmp) = engine_with(0);
    let coll = points_of(&engine);
    const WRITERS: i64 = 4;
    const BATCHES: i64 = 60;
    std::thread::scope(|scope| {
        for writer in 0..WRITERS {
            let coll = coll.clone();
            scope.spawn(move || {
                for batch in 0..BATCHES {
                    let from = (writer * BATCHES + batch) * 250;
                    coll.insert_batch((from..from + 250).map(point).collect())
                        .unwrap();
                    // Single points too: they take the other write path.
                    coll.insert(point(1_000_000 + from)).unwrap();
                }
            });
        }
        // Queries and whole seals meanwhile.
        let engine = &engine;
        let coll = coll.clone();
        scope.spawn(move || {
            for _ in 0..20 {
                coll.seal_column_blocks().unwrap();
                for q in queries().iter().take(3) {
                    run(engine, q, true).unwrap();
                }
            }
        });
    });
    let total = (WRITERS * BATCHES * 251) as u64;
    assert_eq!(coll.count() as u64, total);
    for q in queries() {
        assert_eq!(
            run(&engine, &q, true),
            run(&engine, &q, false),
            "after the writers: {q}"
        );
    }
    coll.seal_column_blocks().unwrap();
    let status = coll.column_blocks_status().unwrap();
    assert_eq!(status.sealed_documents, total, "{status:?}");
    same(&engine, true, "after the writers, sealed");
}

/// After a restart with no explicit seal, the blocks the sealing thread
/// wrote are read back and checked, and the answers are the documents'.
#[test]
fn a_restart_without_an_explicit_seal() {
    let points = 30_000;
    let (engine, tmp) = engine_with(points);
    sealed_behind(&points_of(&engine));
    let before: Vec<_> = queries().iter().map(|q| run(&engine, q, false)).collect();
    drop(engine);

    let engine = open(&tmp);
    let coll = points_of(&engine);
    for (q, want) in queries().iter().zip(&before) {
        assert_eq!(&run(&engine, q, true), want, "after the restart: {q}");
    }
    sealed_behind(&coll);
    same(&engine, true, "after the restart");
}

// ---- Card #934: a row query reads only the key ranges whose block may
// ---- hold a row its FILTERs keep.

fn row_queries() -> Vec<String> {
    let (lo, hi) = (T0 + 600_000, T0 + 660_000);
    vec![
        // An instant, a span, a span of one series.
        format!("FOR p IN points FILTER p.ts == {lo} RETURN p"),
        format!("FOR p IN points FILTER p.ts == {lo} RETURN [p.series, p.v]"),
        format!("FOR p IN points FILTER p.ts >= {lo} AND p.ts < {hi} RETURN p.n"),
        format!("FOR p IN points FILTER p.ts > {lo} FILTER p.ts <= {hi} RETURN p._key"),
        format!(
            "FOR p IN points FILTER p.series == 's007' AND p.ts >= {lo} AND p.ts < {hi} RETURN p"
        ),
        // A series alone: every block holds it, and nothing is passed over.
        "FOR p IN points FILTER p.series == 's007' RETURN p.ts".to_string(),
        // A series no block holds, with and without a time.
        "FOR p IN points FILTER p.series == 'nope' RETURN p".to_string(),
        format!("FOR p IN points FILTER p.series == 'nope' AND p.ts >= {lo} RETURN p"),
        // Conjuncts the blocks do not read, beside ones they do.
        format!("FOR p IN points FILTER p.ts >= {lo} AND p.ts < {hi} AND p.n > 8 RETURN p.v"),
        format!("FOR p IN points FILTER p.ts < {hi} AND p.v > 300 AND p.host == 'b' RETURN p.m"),
        format!("FOR p IN points FILTER p.ts == {lo} OR p.n == 3 RETURN p.ts"),
        // Bounds that are not whole numbers, that hold nothing, a string.
        format!("FOR p IN points FILTER p.ts >= {lo}.5 AND p.ts < {hi}.5 RETURN p.ts"),
        format!("FOR p IN points FILTER p.ts == {lo}.5 RETURN p"),
        format!("FOR p IN points FILTER p.ts > {hi} AND p.ts < {lo} RETURN p"),
        "FOR p IN points FILTER p.ts == '2025-10-09T08:00:00Z' RETURN p.v".to_string(),
        "FOR p IN points FILTER p.ts < 0 RETURN p".to_string(),
        // Two strings for one field, a number for a string field.
        "FOR p IN points FILTER p.series == 's007' AND p.series == 's008' RETURN p".to_string(),
        "FOR p IN points FILTER p.series == 7 RETURN p".to_string(),
        // Clauses after the FILTER.
        format!("FOR p IN points FILTER p.ts >= {lo} AND p.ts < {hi} SORT p.v DESC LIMIT 5 RETURN p.v"),
        format!("FOR p IN points FILTER p.ts >= {lo} AND p.ts < {hi} LIMIT 7 RETURN p._key"),
        format!(
            "FOR p IN points FILTER p.ts >= {lo} AND p.ts < {hi} LET d = p.v * 2 FILTER d > 100 RETURN d"
        ),
        // The collection inside another loop, its bound from the outer row.
        format!(
            "FOR t IN [{lo}, {hi}] FOR p IN points FILTER p.ts == t AND p.series == 's003' RETURN [t, p.v]"
        ),
    ]
}

/// Each row query both ways: the same rows, in the same order.
fn same_rows(engine: &StorageEngine, when: &str) {
    for q in row_queries() {
        let with = run(engine, &q, true);
        assert!(with.is_ok(), "{when}: {q}: {with:?}");
        assert_eq!(with, run(engine, &q, false), "{when}: {q}");
    }
}

fn passed_over() -> u64 {
    solidb::storage::blocks::BLOCKS_PASSED_OVER.load(std::sync::atomic::Ordering::Relaxed)
}

#[test]
fn a_row_query_reads_only_the_blocks_that_may_hold_its_rows() {
    let points = 6 * BLOCK_ROWS as i64;
    let (engine, _tmp) = engine_with(points);
    let coll = points_of(&engine);
    same_rows(&engine, "before any block");
    coll.seal_column_blocks().unwrap();
    let blocks = coll.column_blocks_status().unwrap().blocks as u64;
    assert!(blocks >= 3, "{blocks} blocks");
    same_rows(&engine, "sealed");

    // An instant lies in one block: the others are passed over, and the
    // rows are the ones the whole collection gives.
    let instant = format!(
        "FOR p IN points FILTER p.ts == {} RETURN p.series",
        T0 + 600_000
    );
    let before = passed_over();
    let rows = run(&engine, &instant, true).unwrap();
    assert_eq!(rows.len(), SERIES as usize);
    assert!(
        passed_over() - before >= blocks - 2,
        "{} of {blocks}",
        passed_over() - before
    );
    // A series no block holds: nothing is read at all.
    let before = passed_over();
    assert!(run(
        &engine,
        "FOR p IN points FILTER p.series == 'nope' RETURN p",
        true
    )
    .unwrap()
    .is_empty());
    assert_eq!(passed_over() - before, blocks);
    // Without the switch, the ordinary scan: no block is consulted.
    let before = passed_over();
    run(&engine, &instant, false).unwrap();
    assert_eq!(passed_over(), before);

    // Writes: a point in the middle of a sealed range, points past the
    // last block, a point that fits no column.
    coll.insert(
        json!({"series": "s007", "ts": T0 + 600_000, "v": 1.25, "n": 2, "host": "b", "m": 1}),
    )
    .unwrap();
    coll.insert(json!({"_key": "0", "series": "s003", "ts": T0 + 660_000, "v": 2.5, "n": 1, "host": "b", "m": 2}))
        .unwrap();
    coll.insert(
        json!({"series": "s001", "ts": "2025-10-09T08:00:00Z", "v": 1.5, "n": 1, "host": "b"}),
    )
    .unwrap();
    same_rows(&engine, "after writes");
    coll.seal_column_blocks().unwrap();
    same_rows(&engine, "sealed again");

    // After a restart the blocks are read from disk and checked again.
    drop(coll);
    drop(engine);
    let engine = open(&_tmp);
    same_rows(&engine, "after a restart");
}
