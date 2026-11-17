//! Counter fields on an instance that fsyncs every write
//! (`SOLIDB_DURABILITY=full`, solistore). The variable is the process's,
//! which is why this file holds one test.
//!
//! Such an instance used to fsync a numbered insert under the counter's
//! lock, whatever the counter's `durable`: one fsync per insert, shared
//! with nobody (card #943). The insert is now written unsynced under the
//! lock and waits, the lock released, for a sync it shares.

use serde_json::{json, Value};
use solidb::storage::collection::counters::CounterSpec;
use solidb::storage::{Collection, StorageEngine};
use tempfile::TempDir;

fn numbers(c: &Collection) -> Vec<u64> {
    let mut found: Vec<u64> = c
        .scan(None)
        .iter()
        .filter_map(|d| d.data.get("number").and_then(Value::as_u64))
        .collect();
    found.sort_unstable();
    found
}

#[test]
fn numbered_inserts_are_gapless_when_every_write_is_fsynced() {
    std::env::set_var("SOLIDB_DURABILITY", "full");
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();
    const WRITERS: u64 = 8;
    const EACH: u64 = 25;
    {
        let engine = StorageEngine::new(&path).unwrap();
        engine.initialize().unwrap();
        engine.create_database("app".to_string()).unwrap();
        let db = engine.get_database("app").unwrap();
        for (name, durable) in [("fast", false), ("durable", true)] {
            db.create_collection(name.to_string(), None).unwrap();
            let c = db.get_collection(name).unwrap();
            let spec: CounterSpec =
                serde_json::from_value(json!({"field": "number", "durable": durable})).unwrap();
            c.add_counter(spec).unwrap();
            std::thread::scope(|scope| {
                for w in 0..WRITERS {
                    let c = &c;
                    scope.spawn(move || {
                        for i in 0..EACH {
                            // Single inserts and batches, and one refused
                            // insert in four (a key already stored): it
                            // takes no number.
                            if i % 4 == 3 {
                                c.insert_batch(vec![json!({"w": w}), json!({"w": w})])
                                    .unwrap();
                                assert!(c
                                    .insert(json!({"_key": format!("k{w}-0"), "w": w}))
                                    .is_err());
                            } else {
                                c.insert(json!({"_key": format!("k{w}-{i}"), "w": w}))
                                    .unwrap();
                            }
                        }
                    });
                }
            });
            let per_writer = (0..EACH)
                .map(|i| if i % 4 == 3 { 2 } else { 1 })
                .sum::<u64>();
            let expected: Vec<u64> = (1..=WRITERS * per_writer).collect();
            assert_eq!(numbers(&c), expected, "{name}");
        }
    }
    // What was acknowledged is there after the instance is closed and
    // opened again, and the numbering goes on where it was.
    let engine = StorageEngine::new(&path).unwrap();
    engine.initialize().unwrap();
    let db = engine.get_database("app").unwrap();
    for name in ["fast", "durable"] {
        let c = db.get_collection(name).unwrap();
        let before = numbers(&c);
        let next = c.insert(json!({})).unwrap();
        assert_eq!(
            next.data["number"],
            json!(before.len() as u64 + 1),
            "{name}"
        );
    }
}
