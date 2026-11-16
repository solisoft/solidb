//! Counter fields (`storage::counters`): the database numbers a field on
//! insert, never twice the same number and with no gap, on every write
//! path.

use serde_json::{json, Value};
use solidb::error::DbError;
use solidb::storage::collection::counters::CounterSpec;
use solidb::storage::{Collection, StorageEngine};
use std::sync::Arc;
use tempfile::TempDir;

struct Shop {
    engine: StorageEngine,
    invoices: Collection,
    tmp: TempDir,
}

fn spec(body: Value) -> CounterSpec {
    serde_json::from_value(body).unwrap()
}

fn shop(counter: Value) -> Shop {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    db.create_collection("invoices".to_string(), None).unwrap();
    engine.initialize_transactions().unwrap();
    let invoices = db.get_collection("invoices").unwrap();
    invoices.add_counter(spec(counter)).unwrap();
    Shop {
        engine,
        invoices,
        tmp,
    }
}

/// The numbers the collection holds, in order.
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
fn inserts_are_numbered_from_one() {
    let s = shop(json!({"field": "number"}));
    let a = s.invoices.insert(json!({"total": 10})).unwrap();
    let b = s.invoices.insert(json!({"total": 20})).unwrap();
    assert_eq!(a.data["number"], json!(1));
    assert_eq!(b.data["number"], json!(2));
    let batch = s
        .invoices
        .insert_batch(vec![json!({"total": 1}), json!({"total": 2}), json!({})])
        .unwrap();
    let given: Vec<&Value> = batch.iter().map(|d| &d.data["number"]).collect();
    assert_eq!(given, [&json!(3), &json!(4), &json!(5)]);
    // What is stored is what was returned.
    assert_eq!(
        s.invoices.get(&batch[2].key).unwrap().data["number"],
        json!(5)
    );
}

#[test]
fn a_prefix_a_padding_and_a_start() {
    let s = shop(
        json!({"field": "number", "start": 41, "prefix": "INV-", "suffix": "/A", "padding": 4}),
    );
    let a = s.invoices.insert(json!({})).unwrap();
    assert_eq!(a.data["number"], json!("INV-0041/A"));
    let b = s.invoices.insert(json!({})).unwrap();
    assert_eq!(b.data["number"], json!("INV-0042/A"));
}

#[test]
fn a_refused_insert_takes_no_number() {
    let s = shop(json!({"field": "number"}));
    s.invoices.insert(json!({"_key": "a"})).unwrap();
    // The key is taken: nothing is written, nothing is consumed.
    assert!(s.invoices.insert(json!({"_key": "a"})).is_err());
    assert!(s
        .invoices
        .insert_batch(vec![json!({"_key": "b"}), json!({"_key": "a"})])
        .is_err());
    let next = s.invoices.insert(json!({"_key": "c"})).unwrap();
    assert_eq!(next.data["number"], json!(2));
    assert_eq!(numbers(&s.invoices), [1, 2]);
}

#[test]
fn the_client_does_not_choose_the_number() {
    let s = shop(json!({"field": "number"}));
    let refused = s.invoices.insert(json!({"number": 7}));
    assert!(
        matches!(refused, Err(DbError::BadRequest(_))),
        "{:?}",
        refused
    );
    // Null is the same as leaving it out.
    let a = s
        .invoices
        .insert(json!({"_key": "a", "number": null}))
        .unwrap();
    assert_eq!(a.data["number"], json!(1));

    // An update may not change it, and a merge leaves it alone.
    let changed = s.invoices.update("a", json!({"number": 9}));
    assert!(
        matches!(changed, Err(DbError::BadRequest(_))),
        "{:?}",
        changed
    );
    let merged = s.invoices.update("a", json!({"paid": true})).unwrap();
    assert_eq!(merged.data["number"], json!(1));
    // A replace that leaves the field out keeps the number.
    let replaced = s
        .invoices
        .replace_existing("a", None, json!({"total": 3}))
        .unwrap();
    assert_eq!(replaced.data["number"], json!(1));
    assert_eq!(replaced.data.get("paid"), None);
    let replaced = s
        .invoices
        .insert_or_replace(json!({"_key": "a", "total": 4}))
        .unwrap();
    assert_eq!(replaced.data["number"], json!(1));
    // And a replace of a key that is not there is a numbered insert.
    let new = s.invoices.insert_or_replace(json!({"_key": "z"})).unwrap();
    assert_eq!(new.data["number"], json!(2));
}

#[test]
fn a_merge_by_key_does_not_create_an_unnumbered_document() {
    let s = shop(json!({"field": "number"}));
    s.invoices.insert(json!({"_key": "a"})).unwrap();
    // Merging into a stored document is an update.
    s.invoices
        .upsert_batch(vec![("a".to_string(), json!({"paid": true}))])
        .unwrap();
    let refused = s
        .invoices
        .upsert_batch(vec![("b".to_string(), json!({"paid": true}))]);
    assert!(
        matches!(refused, Err(DbError::OperationNotSupported(_))),
        "{:?}",
        refused
    );
    assert_eq!(numbers(&s.invoices), [1]);
}

#[test]
fn concurrent_inserts_get_every_number_once() {
    let s = shop(json!({"field": "number", "durable": false}));
    let invoices = Arc::new(s.invoices.clone());
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let invoices = Arc::clone(&invoices);
            std::thread::spawn(move || {
                for i in 0..50 {
                    if i % 5 == 0 {
                        invoices
                            .insert_batch(vec![json!({"t": t}), json!({"t": t}), json!({"t": t})])
                            .unwrap();
                    } else {
                        invoices.insert(json!({"t": t})).unwrap();
                    }
                    // One in ten tries a key that is taken, and must not
                    // leave a hole.
                    if i % 10 == 0 {
                        let _ = invoices.insert(json!({"_key": "taken"}));
                    }
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let found = numbers(&s.invoices);
    let expected: Vec<u64> = (1..=found.len() as u64).collect();
    assert_eq!(found, expected);
    // 8 threads × (40 single + 10 batches of 3), plus the one "taken".
    assert_eq!(found.len(), 8 * (40 + 30) + 1);
}

#[test]
fn the_count_goes_on_after_a_restart_and_a_truncate() {
    let s = shop(json!({"field": "number"}));
    s.invoices.insert(json!({})).unwrap();
    s.invoices.insert(json!({})).unwrap();
    s.invoices.truncate().unwrap();
    assert_eq!(
        s.invoices.insert(json!({})).unwrap().data["number"],
        json!(3)
    );

    let Shop {
        engine,
        invoices,
        tmp,
    } = s;
    drop(invoices);
    drop(engine);
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    let invoices = engine
        .get_database("app")
        .unwrap()
        .get_collection("invoices")
        .unwrap();
    assert_eq!(invoices.insert(json!({})).unwrap().data["number"], json!(4));
}

#[test]
fn a_counter_on_documents_that_are_already_numbered() {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    db.create_collection("invoices".to_string(), None).unwrap();
    let invoices = db.get_collection("invoices").unwrap();
    for n in [1, 2, 3] {
        invoices.insert(json!({"number": n})).unwrap();
    }
    invoices.insert(json!({"draft": true})).unwrap();

    // The first number is taken: every insert would be refused.
    let refused = invoices.add_counter(spec(json!({"field": "number", "start": 3})));
    assert!(
        matches!(refused, Err(DbError::ConflictError(_))),
        "{:?}",
        refused
    );
    assert!(invoices.counter_defs().is_empty());
    assert!(
        invoices.get_all_indexes().is_empty(),
        "the index made for it is gone"
    );

    invoices
        .add_counter(spec(json!({"field": "number", "start": 4})))
        .unwrap();
    assert_eq!(invoices.insert(json!({})).unwrap().data["number"], json!(4));

    // Set the next number: refused on one that is held, taken otherwise.
    assert!(invoices.set_counter_next("number", 2).is_err());
    invoices.set_counter_next("number", 100).unwrap();
    assert_eq!(
        invoices.insert(json!({})).unwrap().data["number"],
        json!(100)
    );
    assert_eq!(invoices.counter_record("number").unwrap().state.next, 101);

    // Two documents with the same value cannot become a counter.
    db.create_collection("dup".to_string(), None).unwrap();
    let dup = db.get_collection("dup").unwrap();
    dup.insert(json!({"number": 1})).unwrap();
    dup.insert(json!({"number": 1})).unwrap();
    let refused = dup.add_counter(spec(json!({"field": "number"})));
    assert!(
        matches!(refused, Err(DbError::ConflictError(_))),
        "{:?}",
        refused
    );
}

#[test]
fn what_cannot_be_a_counter() {
    let s = shop(json!({"field": "number"}));
    let again = s.invoices.add_counter(spec(json!({"field": "number"})));
    assert!(
        matches!(again, Err(DbError::ConflictError(_))),
        "{:?}",
        again
    );
    let unnamed = s.invoices.add_counter(spec(
        json!({"field": "ref", "reset": "month", "prefix": "R-"}),
    ));
    assert!(
        matches!(unnamed, Err(DbError::BadRequest(_))),
        "{:?}",
        unnamed
    );
    // Its index stays as long as the counter does.
    let index = s.invoices.counter_defs()[0].index.clone();
    assert!(matches!(
        s.invoices.drop_index(&index),
        Err(DbError::ConflictError(_))
    ));
    s.invoices.drop_counter("number").unwrap();
    s.invoices.drop_index(&index).unwrap();
    // Without the counter the field is the client's again.
    let free = s.invoices.insert(json!({"number": 12})).unwrap();
    assert_eq!(free.data["number"], json!(12));
}

// ------------------------------------------------------------ transactions

type Stage<'a> = &'a mut solidb::transaction::Transaction;
type Wal = Arc<solidb::transaction::wal::WalWriter>;
type Locks = Arc<solidb::transaction::lock_manager::LockManager>;

/// Begin a transaction, stage what `stage` does, and return its id.
fn staged(s: &Shop, stage: impl FnOnce(Stage, &Wal, &Locks)) -> solidb::transaction::TransactionId {
    let manager = s.engine.transaction_manager().unwrap();
    let tx_id = manager
        .begin(solidb::transaction::IsolationLevel::ReadCommitted)
        .unwrap();
    let tx_arc = manager.get(tx_id).unwrap();
    let mut tx = tx_arc.write().unwrap();
    stage(
        &mut tx,
        &manager.wal().clone(),
        &manager.lock_manager().clone(),
    );
    tx_id
}

#[test]
fn a_transaction_is_numbered_when_it_commits() {
    let s = shop(json!({"field": "number"}));
    let tx = staged(&s, |tx, wal, locks| {
        let a = s
            .invoices
            .insert_tx(tx, wal, locks, json!({"_key": "a"}))
            .unwrap();
        // Not yet: the number is the commit's.
        assert_eq!(a.data.get("number"), None);
        s.invoices
            .insert_tx(tx, wal, locks, json!({"_key": "b"}))
            .unwrap();
    });
    // An insert that lands before the commit is numbered before it.
    assert_eq!(
        s.invoices.insert(json!({"_key": "first"})).unwrap().data["number"],
        json!(1)
    );
    s.engine.commit_transaction(tx).unwrap();
    assert_eq!(s.invoices.get("a").unwrap().data["number"], json!(2));
    assert_eq!(s.invoices.get("b").unwrap().data["number"], json!(3));
    assert_eq!(numbers(&s.invoices), [1, 2, 3]);
}

#[test]
fn a_rollback_and_a_refused_commit_take_no_number() {
    let s = shop(json!({"field": "number"}));
    let rolled = staged(&s, |tx, wal, locks| {
        s.invoices
            .insert_tx(tx, wal, locks, json!({"_key": "a"}))
            .unwrap();
    });
    s.engine.rollback_transaction(rolled).unwrap();

    // A commit that is refused as a whole: its second insert's key is taken
    // by the time it commits.
    let refused = staged(&s, |tx, wal, locks| {
        s.invoices
            .insert_tx(tx, wal, locks, json!({"_key": "b"}))
            .unwrap();
        s.invoices
            .insert_tx(tx, wal, locks, json!({"_key": "c", "ref": "x"}))
            .unwrap();
    });
    s.invoices
        .create_index(
            "by_ref".to_string(),
            vec!["ref".to_string()],
            solidb::storage::IndexType::Persistent,
            true,
        )
        .unwrap();
    s.invoices
        .insert(json!({"_key": "other", "ref": "x"}))
        .unwrap();
    assert!(s.engine.commit_transaction(refused).is_err());

    assert_eq!(
        s.invoices.insert(json!({"_key": "d"})).unwrap().data["number"],
        json!(2)
    );
    assert_eq!(numbers(&s.invoices), [1, 2]);
}

#[test]
fn what_a_transaction_does_to_a_document_it_inserted() {
    let s = shop(json!({"field": "number"}));
    let tx = staged(&s, |tx, wal, locks| {
        s.invoices
            .insert_tx(tx, wal, locks, json!({"_key": "kept"}))
            .unwrap();
        s.invoices
            .update_tx(tx, wal, locks, "kept", json!({"paid": true}))
            .unwrap();
        // Inserted and deleted: no number is spent on it.
        s.invoices
            .insert_tx(tx, wal, locks, json!({"_key": "gone"}))
            .unwrap();
        s.invoices.delete_tx(tx, wal, locks, "gone").unwrap();
        s.invoices
            .insert_tx(tx, wal, locks, json!({"_key": "last"}))
            .unwrap();
        // The number is not the client's in a transaction either.
        let given = s
            .invoices
            .insert_tx(tx, wal, locks, json!({"_key": "x", "number": 5}));
        assert!(matches!(given, Err(DbError::BadRequest(_))), "{:?}", given);
    });
    s.engine.commit_transaction(tx).unwrap();
    let kept = s.invoices.get("kept").unwrap();
    assert_eq!(kept.data["number"], json!(1));
    assert_eq!(kept.data["paid"], json!(true));
    assert!(s.invoices.get("gone").is_err());
    assert_eq!(s.invoices.get("last").unwrap().data["number"], json!(2));
    // The unique index holds what was stored.
    assert_eq!(
        s.invoices
            .index_lookup_eq("number", &json!(1))
            .unwrap()
            .len(),
        1
    );

    // An update in a transaction may not change a number.
    let tx = staged(&s, |tx, wal, locks| {
        let changed = s
            .invoices
            .update_tx(tx, wal, locks, "kept", json!({"number": 9}));
        assert!(
            matches!(changed, Err(DbError::BadRequest(_))),
            "{:?}",
            changed
        );
    });
    s.engine.rollback_transaction(tx).unwrap();
}

#[test]
fn transactions_and_plain_inserts_share_the_count() {
    let s = Arc::new(shop(json!({"field": "number", "durable": false})));
    let threads: Vec<_> = (0..6)
        .map(|t| {
            let s = Arc::clone(&s);
            std::thread::spawn(move || {
                for i in 0..40 {
                    if (t + i) % 2 == 0 {
                        s.invoices.insert(json!({"t": t})).unwrap();
                        continue;
                    }
                    let tx = staged(&s, |tx, wal, locks| {
                        s.invoices
                            .insert_tx(tx, wal, locks, json!({"t": t}))
                            .unwrap();
                        s.invoices
                            .insert_tx(tx, wal, locks, json!({"t": t}))
                            .unwrap();
                    });
                    if i % 7 == 0 {
                        s.engine.rollback_transaction(tx).unwrap();
                    } else {
                        s.engine.commit_transaction(tx).unwrap();
                    }
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let found = numbers(&s.invoices);
    let expected: Vec<u64> = (1..=found.len() as u64).collect();
    assert_eq!(found, expected);
    assert_eq!(found.len(), s.invoices.count());
}
