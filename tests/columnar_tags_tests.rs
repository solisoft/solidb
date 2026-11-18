//! Tags of a columnar collection (card #981): a tag reads the rows as they
//! were when it was taken, whatever is written since, and a restore makes
//! them the collection's rows again.
//!
//! What a tag is held to: the answers of every query read at the tag are
//! those the same query gave when the tag was taken, after each kind of
//! write that retires a block (a delete in a block, compaction, a
//! retention, a schema change, a truncate), and after other tags are
//! dropped.

use serde_json::{json, Value};
use solidb::sdbql::{parse, QueryExecutor};
use solidb::sync::columnar;
use solidb::{ColumnDef, ColumnType, ColumnarCollection, CompressionType, StorageEngine};
use std::collections::HashMap;
use tempfile::TempDir;

fn column(name: &str, data_type: ColumnType) -> ColumnDef {
    ColumnDef {
        name: name.to_string(),
        data_type,
        nullable: true,
        indexed: false,
        index_type: None,
    }
}

fn rows(from: i64, to: i64) -> Vec<Value> {
    (from..to)
        .map(|i| json!({"host": format!("h{}", i % 7), "v": i, "ms": i as f64 / 8.0}))
        .collect()
}

/// `m`: two sealed blocks and a tail.
fn fixture() -> (StorageEngine, ColumnarCollection, TempDir) {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(dir.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    let columns = vec![
        column("host", ColumnType::String),
        column("v", ColumnType::Int64),
        column("ms", ColumnType::Float64),
    ];
    let col = columnar::create(&db, None, "m", columns, CompressionType::Lz4).unwrap();
    col.insert_rows(rows(0, 8_000)).unwrap();
    col.insert_rows(rows(8_000, 16_000)).unwrap();
    col.insert_rows(rows(16_000, 16_010)).unwrap();
    (engine, col, dir)
}

fn run(engine: &StorageEngine, query: &str) -> Result<Vec<Value>, String> {
    QueryExecutor::with_database_and_bind_vars(engine, "app".to_string(), HashMap::new())
        .execute(&parse(query).expect("query parses"))
        .map_err(|e| e.to_string())
}

/// What the tests compare, each with `$o` for the FOR's options.
const QUERIES: &[&str] = &[
    "FOR r IN m $o COLLECT h = r.host AGGREGATE n = COUNT(1), s = SUM(r.v), x = MAX(r.ms) RETURN {h, n, s, x}",
    "FOR r IN m $o FILTER r.v >= 7990 AND r.v < 8010 SORT r.v RETURN r",
    "FOR r IN m $o FILTER r.host == 'h3' COLLECT WITH COUNT INTO n RETURN n",
    "FOR r IN m $o FILTER r.v >= 16000 SORT r.v RETURN r.v",
    "RETURN LENGTH(FOR r IN m $o RETURN 1)",
];

fn answers(engine: &StorageEngine, options: &str) -> Vec<Vec<Value>> {
    QUERIES
        .iter()
        .map(|q| {
            let q = q.replace("$o", options);
            run(engine, &q).unwrap_or_else(|e| panic!("{q}: {e}"))
        })
        .collect()
}

const AT_T1: &str = "OPTIONS {tag: 't1'}";

#[test]
fn a_tag_reads_what_was_there_after_every_kind_of_write() {
    let (engine, col, _dir) = fixture();
    let then = answers(&engine, "");
    col.create_tag("t1").unwrap();
    assert_eq!(answers(&engine, AT_T1), then, "right after the tag");

    let steps: Vec<(&str, Box<dyn Fn(&ColumnarCollection)>)> = vec![
        (
            "rows inserted",
            Box::new(|c| {
                c.insert_rows(rows(20_000, 20_100)).unwrap();
            }),
        ),
        (
            "one row deleted in a block",
            Box::new(|c| {
                let ids = c.list_row_uuids().unwrap();
                c.delete_row(&ids[100]).unwrap();
            }),
        ),
        (
            "rows deleted across both blocks",
            Box::new(|c| {
                let ids = c.list_row_uuids().unwrap();
                let some: Vec<String> = ids.iter().step_by(97).cloned().collect();
                c.delete_rows(&some).unwrap();
            }),
        ),
        (
            "compacted",
            Box::new(|c| {
                c.compact().unwrap();
            }),
        ),
        (
            "a column renamed",
            Box::new(|c| c.rename_column("ms", "millis").unwrap()),
        ),
        (
            "a column dropped",
            Box::new(|c| c.drop_column("millis").unwrap()),
        ),
        (
            "the oldest rows pruned",
            Box::new(|c| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;
                c.prune_before(now + 1).unwrap();
            }),
        ),
        (
            "rows inserted after a prune",
            Box::new(|c| {
                let without_ms: Vec<Value> = (30_000..35_000)
                    .map(|i| json!({"host": format!("h{}", i % 7), "v": i}))
                    .collect();
                c.insert_rows(without_ms).unwrap();
            }),
        ),
        ("truncated", Box::new(|c| c.truncate().unwrap())),
    ];
    for (what, write) in steps {
        write(&col);
        assert_eq!(answers(&engine, AT_T1), then, "after: {what}");
    }

    // Restored, the collection answers as it did; the tag still reads.
    let restored = col.restore_tag("t1").unwrap();
    assert_eq!(restored, 16_010);
    assert_eq!(answers(&engine, ""), then, "restored");
    assert_eq!(answers(&engine, AT_T1), then, "the tag after a restore");
    // And takes writes again.
    col.insert_rows(rows(40_000, 40_005)).unwrap();
    assert_eq!(
        run(&engine, "RETURN LENGTH(FOR r IN m RETURN 1)").unwrap(),
        vec![json!(16_015)]
    );
}

#[test]
fn tags_taken_at_different_times_each_read_their_own() {
    let (engine, col, _dir) = fixture();
    let first = answers(&engine, "");
    col.create_tag("t1").unwrap();
    let ids = col.list_row_uuids().unwrap();
    col.delete_row(&ids[10]).unwrap();
    let second = answers(&engine, "");
    assert_ne!(first, second);
    col.create_tag("t2").unwrap();
    // The same block, rewritten again: t1 and t2 each keep their version.
    col.delete_row(&ids[11]).unwrap();
    col.delete_row(&ids[12]).unwrap();
    col.compact().unwrap();
    assert_eq!(answers(&engine, AT_T1), first);
    assert_eq!(answers(&engine, "OPTIONS {tag: 't2'}"), second);

    // Dropping t1 leaves t2 whole.
    col.drop_tag("t1").unwrap();
    assert_eq!(answers(&engine, "OPTIONS {tag: 't2'}"), second);
    assert!(run(&engine, "FOR r IN m OPTIONS {tag: 't1'} RETURN r").is_err());
    let listed: Vec<String> = col
        .list_tags()
        .unwrap()
        .into_iter()
        .map(|t| t.tag)
        .collect();
    assert_eq!(listed, vec!["t2".to_string()]);
}

#[test]
fn a_query_reads_a_collection_at_one_tag() {
    let (engine, col, _dir) = fixture();
    col.create_tag("t1").unwrap();
    col.create_tag("t2").unwrap();
    for q in [
        "FOR a IN m OPTIONS {tag: 't1'} FOR b IN m FILTER a.v == b.v LIMIT 1 RETURN 1",
        "FOR a IN m OPTIONS {tag: 't1'} LET n = (FOR b IN m OPTIONS {tag: 't2'} RETURN 1) LIMIT 1 RETURN n",
    ] {
        let refused = run(&engine, q);
        assert!(refused.as_ref().is_err_and(|e| e.contains("one tag")), "{q}: {refused:?}");
    }
    // A tag of a document collection, or a tag that does not exist.
    let db = engine.get_database("app").unwrap();
    db.create_collection("d".to_string(), None).unwrap();
    assert!(run(&engine, "FOR r IN d OPTIONS {tag: 't1'} RETURN r").is_err());
    assert!(run(&engine, "FOR r IN m OPTIONS {tag: 'nope'} RETURN r").is_err());
    // Names that are not names.
    assert!(col.create_tag("").is_err());
    assert!(col.create_tag("a/b").is_err());
    assert!(col.create_tag("t1").is_err(), "taken twice");
}
