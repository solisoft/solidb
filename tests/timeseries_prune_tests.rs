//! Pruning a time series collection (`Collection::prune_older_than`). A
//! collection that holds nothing but its documents is pruned as one range
//! delete; one with an index, or with a key the client chose among the old
//! ones, is pruned in batches. Both must leave the same documents, the
//! same count and the same answers, on either storage engine.
//!
//! One test function: it sets `SOLIDB_ENGINE`, which is the process's.

use serde_json::{json, Value};
use solidb::storage::blocks::BlocksDef;
use solidb::storage::{Collection, IndexType, StorageEngine};
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn points(from: i64, to: i64) -> Vec<Value> {
    (from..to)
        .map(|i| json!({"series": format!("s{:02}", i % 20), "ts": 1_760_000_000_000i64 + i * 1000, "v": (i % 97) as f64 / 4.0}))
        .collect()
}

fn run(engine: &StorageEngine, q: &str) -> Vec<Value> {
    QueryExecutor::new(engine)
        .execute(&parse(q).unwrap())
        .unwrap_or_else(|e| panic!("{q}: {e}"))
}

/// What the collection answers, by documents and by blocks.
fn answers(engine: &StorageEngine, name: &str) -> Vec<Vec<Value>> {
    [
        format!("FOR p IN {name} SORT p.ts, p.series RETURN [p.series, p.ts, p.v]"),
        format!("FOR p IN {name} COLLECT s = p.series AGGREGATE n = COUNT(), t = SUM(p.v) SORT s RETURN [s, n, t]"),
        format!("RETURN COLLECTION_COUNT('{name}')"),
        format!("FOR p IN {name} FILTER p.ts < 1760000003000 RETURN p.ts"),
    ]
    .iter()
    .map(|q| run(engine, q))
    .collect()
}

fn scenario(engine_name: &str) {
    std::env::set_var("SOLIDB_ENGINE", engine_name);
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    let make = |name: &str| -> Collection {
        engine
            .create_collection(name.to_string(), Some("timeseries".to_string()))
            .unwrap();
        engine.get_collection(name).unwrap()
    };
    // `plain` holds only documents; `indexed` an index beside them; `keyed`
    // a key the client chose among the old ones; `blocks` column blocks.
    let plain = make("plain");
    let indexed = make("indexed");
    indexed
        .create_index(
            "by_series".to_string(),
            vec!["series".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();
    let keyed = make("keyed");
    let blocks = make("blocks");
    blocks
        .set_column_blocks(BlocksDef {
            time: "ts".to_string(),
            fields: vec!["series".to_string(), "v".to_string()],
        })
        .unwrap();
    let all = [&plain, &indexed, &keyed, &blocks];

    for c in all {
        c.insert_batch(points(0, 6_000)).unwrap();
    }
    keyed
        .insert(json!({"_key": "0-chosen", "series": "s00", "ts": 1, "v": 0.5}))
        .unwrap();
    blocks.seal_column_blocks().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let cut = now_ms();
    std::thread::sleep(std::time::Duration::from_millis(20));
    for c in all {
        c.insert_batch(points(6_000, 9_000)).unwrap();
    }
    blocks.seal_column_blocks().unwrap();
    // Read before the prune, so that a result kept from it would show.
    for c in ["plain", "indexed", "blocks"] {
        assert_eq!(
            run(&engine, &format!("RETURN COLLECTION_COUNT('{c}')")),
            vec![json!(9_000)]
        );
        assert_eq!(answers(&engine, c)[0].len(), 9_000, "{engine_name}: {c}");
    }

    assert_eq!(plain.prune_older_than(cut).unwrap(), 6_000, "{engine_name}");
    assert_eq!(
        indexed.prune_older_than(cut).unwrap(),
        6_000,
        "{engine_name}"
    );
    assert_eq!(keyed.prune_older_than(cut).unwrap(), 6_000, "{engine_name}");
    assert_eq!(
        blocks.prune_older_than(cut).unwrap(),
        6_000,
        "{engine_name}"
    );

    let expected = answers(&engine, "indexed");
    assert_eq!(expected[0].len(), 3_000);
    assert_eq!(answers(&engine, "plain"), expected, "{engine_name}: plain");
    assert_eq!(
        answers(&engine, "blocks"),
        expected,
        "{engine_name}: blocks"
    );
    // The key the client chose is still there, and nothing else is.
    assert_eq!(keyed.count(), 3_001, "{engine_name}");
    assert!(keyed.get("0-chosen").is_ok());
    for c in [&plain, &indexed, &blocks] {
        assert_eq!(c.count(), 3_000, "{engine_name}");
        assert_eq!(c.scan(None).len(), 3_000, "{engine_name}");
    }
    // The index holds the documents that are left, no other.
    assert_eq!(
        run(
            &engine,
            "RETURN LENGTH(FOR p IN indexed FILTER p.series == 's07' RETURN 1)"
        ),
        run(
            &engine,
            "RETURN LENGTH(FOR p IN plain FILTER p.series == 's07' RETURN 1)"
        ),
    );

    // Again: nothing left to prune; and a cut-off before everything.
    for c in all {
        assert_eq!(c.prune_older_than(cut).unwrap(), 0, "{engine_name}");
        assert_eq!(c.prune_older_than(1).unwrap(), 0, "{engine_name}");
    }
    // New points go on being stored and counted, then everything goes.
    for c in all {
        c.insert_batch(points(9_000, 9_500)).unwrap();
    }
    assert_eq!(
        answers(&engine, "plain"),
        answers(&engine, "indexed"),
        "{engine_name}"
    );
    assert_eq!(
        answers(&engine, "blocks"),
        answers(&engine, "indexed"),
        "{engine_name}"
    );
    let later = now_ms() + 60_000;
    assert_eq!(
        plain.prune_older_than(later).unwrap(),
        3_500,
        "{engine_name}"
    );
    assert_eq!(
        indexed.prune_older_than(later).unwrap(),
        3_500,
        "{engine_name}"
    );
    assert_eq!(
        blocks.prune_older_than(later).unwrap(),
        3_500,
        "{engine_name}"
    );
    for c in ["plain", "indexed", "blocks"] {
        assert_eq!(
            run(&engine, &format!("RETURN COLLECTION_COUNT('{c}')")),
            vec![json!(0)]
        );
        assert!(answers(&engine, c)[0].is_empty(), "{engine_name}: {c}");
    }

    // After a restart the counts are what was left.
    drop((plain, indexed, keyed, blocks));
    drop(engine);
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    assert_eq!(
        engine.get_collection("plain").unwrap().count(),
        0,
        "{engine_name}"
    );
    assert_eq!(
        engine.get_collection("keyed").unwrap().count(),
        3_501,
        "{engine_name}"
    );
}

#[test]
fn a_prune_by_range_leaves_what_a_prune_in_batches_leaves() {
    // Each engine is a Cargo feature; RocksDB is off by default from 3.0.
    if cfg!(feature = "engine-rocksdb") {
        scenario("rocksdb");
    }
    if cfg!(feature = "engine-solistore") {
        scenario("solistore");
    }
}
