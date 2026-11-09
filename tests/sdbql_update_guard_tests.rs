//! `UPDATE d WITH {n: d.n + 1}` from concurrent statements loses nothing
//! (card #796).
//!
//! The `WITH` value is computed from the document the `FOR` read; a write
//! that landed before the `UPDATE` wrote used to be overwritten by it. The
//! write is now guarded by the revision the query read and, when the
//! document moved on, the row is evaluated again on the current version
//! (its FILTERs and LETs included), or the statement fails with a conflict
//! when its pipeline cannot be re-run for one row.

use serde_json::{json, Value};
use solidb::storage::StorageEngine;
use solidb::{parse, DbError, QueryExecutor};
use std::collections::{HashMap, HashSet};
use tempfile::TempDir;

fn engine() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    (engine, tmp)
}

fn run(engine: &StorageEngine, q: &str, binds: Value) -> Result<Vec<Value>, DbError> {
    let query = parse(q).unwrap_or_else(|e| panic!("parse {q}: {e}"));
    let binds: HashMap<String, Value> = serde_json::from_value(binds).unwrap();
    QueryExecutor::with_bind_vars(engine, binds).execute(&query)
}

const THREADS: usize = 8;

#[test]
fn concurrent_increments_all_land() {
    let (engine, _tmp) = engine();
    engine.create_collection("c".into(), None).unwrap();
    let c = engine.get_collection("c").unwrap();
    c.insert(json!({"_key": "ctr", "n": 0, "vals": []}))
        .unwrap();

    const EACH: usize = 150;
    std::thread::scope(|s| {
        for t in 0..THREADS {
            let engine = &engine;
            s.spawn(move || {
                for i in 0..EACH {
                    run(
                        engine,
                        "FOR d IN c FILTER d._key == 'ctr' \
                         UPDATE d WITH {n: d.n + 1, vals: PUSH(d.vals, @v)} IN c",
                        json!({"v": t * EACH + i}),
                    )
                    .unwrap_or_else(|e| panic!("increment failed: {e}"));
                }
            });
        }
    });

    let doc = c.get("ctr").unwrap().to_value();
    assert_eq!(doc["n"], json!(THREADS * EACH), "every increment landed");
    let vals: HashSet<u64> = doc["vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert_eq!(vals.len(), THREADS * EACH, "every append landed, once");
}

/// Two workers racing for the same pending job: the one that finds it taken
/// re-checks the FILTER on the current document and leaves it alone.
#[test]
fn a_job_is_claimed_once() {
    let (engine, _tmp) = engine();
    engine.create_collection("jobs".into(), None).unwrap();
    let jobs = engine.get_collection("jobs").unwrap();
    const JOBS: usize = 40;
    for j in 0..JOBS {
        jobs.insert(json!({"_key": format!("j{j}"), "status": "pending"}))
            .unwrap();
    }

    let claims: Vec<(usize, Vec<Value>)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..THREADS)
            .map(|w| {
                let engine = &engine;
                s.spawn(move || {
                    let mut mine = Vec::new();
                    for j in 0..JOBS {
                        let got = run(
                            engine,
                            "FOR d IN jobs FILTER d._key == @k FILTER d.status == 'pending' \
                             UPDATE d WITH {status: 'taken', by: @w} IN jobs RETURN NEW._key",
                            json!({"k": format!("j{j}"), "w": w}),
                        )
                        .unwrap();
                        mine.extend(got);
                    }
                    (w, mine)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let mut claimed = HashMap::new();
    for (w, keys) in &claims {
        for k in keys {
            assert!(claimed.insert(k.clone(), *w).is_none(), "{k} claimed twice");
        }
    }
    assert_eq!(claimed.len(), JOBS, "every job claimed");
    for j in 0..JOBS {
        let doc = jobs.get(&format!("j{j}")).unwrap().to_value();
        assert_eq!(doc["status"], "taken");
        assert_eq!(
            doc["by"],
            json!(claimed[&json!(format!("j{j}"))]),
            "the claim that returned it wrote it"
        );
    }
}

/// More than 100 rows take the batch path; a row whose document moved on
/// is left out of the batch and evaluated again on its own.
#[test]
fn batched_updates_lose_nothing() {
    let (engine, _tmp) = engine();
    engine.create_collection("c".into(), None).unwrap();
    let c = engine.get_collection("c").unwrap();
    const DOCS: usize = 150;
    for d in 0..DOCS {
        c.insert(json!({"_key": format!("d{d}"), "n": 0})).unwrap();
    }
    const EACH: usize = 10;
    std::thread::scope(|s| {
        for _ in 0..4 {
            let engine = &engine;
            s.spawn(move || {
                for _ in 0..EACH {
                    run(
                        engine,
                        "FOR d IN c UPDATE d WITH {n: d.n + 1} IN c",
                        json!({}),
                    )
                    .unwrap();
                }
            });
        }
    });
    for d in 0..DOCS {
        assert_eq!(
            c.get(&format!("d{d}")).unwrap().to_value()["n"],
            json!(4 * EACH),
            "d{d}"
        );
    }
}

/// One statement that updates the same document from several rows: each
/// row read it before any of them wrote. They used to all write the same
/// `n + 1`.
#[test]
fn rows_of_one_statement_build_on_each_other() {
    let (engine, _tmp) = engine();
    engine.create_collection("c".into(), None).unwrap();
    let c = engine.get_collection("c").unwrap();
    c.insert(json!({"_key": "x", "n": 0})).unwrap();
    run(
        &engine,
        "FOR i IN 1..3 FOR d IN c FILTER d._key == 'x' UPDATE d WITH {n: d.n + 1} IN c",
        json!({}),
    )
    .unwrap();
    assert_eq!(c.get("x").unwrap().to_value()["n"], json!(3));
}

/// A pipeline that cannot be re-run for one row (here a FOR between the one
/// that read the document and the UPDATE) fails with a conflict when the
/// document moved on, rather than overwrite the newer version.
#[test]
fn a_pipeline_that_cannot_be_rerun_fails_instead_of_losing_the_write() {
    let (engine, _tmp) = engine();
    engine.create_collection("c".into(), None).unwrap();
    let c = engine.get_collection("c").unwrap();
    c.insert(json!({"_key": "x", "n": 0})).unwrap();
    // Two rows from the inner FOR read the same version; the second finds
    // it moved and cannot re-run the inner FOR for itself.
    let err = run(
        &engine,
        "FOR d IN c FILTER d._key == 'x' FOR i IN 1..2 UPDATE d WITH {n: d.n + 1} IN c",
        json!({}),
    )
    .unwrap_err();
    assert!(matches!(err, DbError::ConflictError(_)), "{err:?}");
    assert!(
        err.to_string().contains("changed while this query ran"),
        "{err}"
    );
    // The first row's write stands; nothing was overwritten with a stale value.
    assert_eq!(c.get("x").unwrap().to_value()["n"], json!(1));
}

/// `UPDATE "key" WITH …` names no document the query read: unguarded, as
/// before.
#[test]
fn an_update_by_literal_key_is_not_guarded() {
    let (engine, _tmp) = engine();
    engine.create_collection("c".into(), None).unwrap();
    let c = engine.get_collection("c").unwrap();
    c.insert(json!({"_key": "x", "n": 0})).unwrap();
    run(
        &engine,
        "FOR i IN 1..3 UPDATE 'x' WITH {n: i} IN c",
        json!({}),
    )
    .unwrap();
    assert_eq!(c.get("x").unwrap().to_value()["n"], json!(3));
}
