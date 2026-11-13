//! Foreign keys (`storage::foreign_keys`): a child's reference is checked
//! when it is written, a parent's delete follows `on_delete`, on every
//! write path and inside a transaction's commit.

use serde_json::json;
use solidb::error::DbError;
use solidb::storage::collection::references::ForeignKeySpec;
use solidb::storage::foreign_keys::OnDelete;
use solidb::storage::{Collection, StorageEngine};
use solidb::transaction::IsolationLevel;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

struct Shop {
    engine: StorageEngine,
    customers: Collection,
    orders: Collection,
    _tmp: TempDir,
}

fn shop(on_delete: OnDelete) -> Shop {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    db.create_collection("customers".to_string(), None).unwrap();
    db.create_collection("orders".to_string(), None).unwrap();
    engine.initialize_transactions().unwrap();
    let customers = db.get_collection("customers").unwrap();
    let orders = db.get_collection("orders").unwrap();
    customers.insert(json!({"_key": "alice"})).unwrap();
    customers.insert(json!({"_key": "bob"})).unwrap();
    orders
        .add_foreign_key(ForeignKeySpec::new("customer", "customers", on_delete))
        .unwrap();
    Shop {
        engine,
        customers,
        orders,
        _tmp: tmp,
    }
}

fn is_violation<T: std::fmt::Debug>(result: &Result<T, DbError>) -> bool {
    matches!(result, Err(DbError::ConflictError(m)) if m.starts_with("Foreign key violation"))
}

#[test]
fn a_child_must_name_a_parent_that_exists() {
    let s = shop(OnDelete::Restrict);

    s.orders
        .insert(json!({"_key": "o1", "customer": "alice"}))
        .unwrap();
    // Null and absent refer to nothing.
    s.orders
        .insert(json!({"_key": "o2", "customer": null}))
        .unwrap();
    s.orders.insert(json!({"_key": "o3"})).unwrap();

    let missing = s.orders.insert(json!({"_key": "o4", "customer": "carol"}));
    assert!(is_violation(&missing), "{:?}", missing);
    assert!(s.orders.get("o4").is_err());

    let not_a_key = s.orders.insert(json!({"_key": "o5", "customer": 12}));
    assert!(is_violation(&not_a_key), "{:?}", not_a_key);
}

#[test]
fn every_write_path_checks_the_reference() {
    let s = shop(OnDelete::Restrict);
    s.orders
        .insert(json!({"_key": "o1", "customer": "alice"}))
        .unwrap();

    assert!(is_violation(
        &s.orders.update("o1", json!({"customer": "carol"}))
    ));
    assert!(is_violation(
        &s.orders.merge_update("o1", json!({"customer": "carol"}))
    ));
    assert!(is_violation(&s.orders.replace_existing(
        "o1",
        None,
        json!({"customer": "carol"})
    )));
    assert!(is_violation(
        &s.orders
            .insert_or_replace(json!({"_key": "o1", "customer": "carol"}))
    ));
    assert!(is_violation(&s.orders.insert_batch(vec![
        json!({"_key": "o2", "customer": "bob"}),
        json!({"_key": "o3", "customer": "carol"}),
    ])));
    assert!(is_violation(&s.orders.upsert_batch(vec![(
        "o1".to_string(),
        json!({"customer": "carol"})
    )])));
    assert!(is_violation(&s.orders.update_batch(&[(
        "o1".to_string(),
        json!({"customer": "carol"})
    )])));

    // Nothing of the refused writes is there.
    assert_eq!(s.orders.count(), 1);
    assert_eq!(s.orders.get("o1").unwrap().data["customer"], json!("alice"));

    // The same paths take a parent that exists.
    s.orders.update("o1", json!({"customer": "bob"})).unwrap();
    s.orders
        .insert_batch(vec![json!({"_key": "o2", "customer": "bob"})])
        .unwrap();
    s.orders
        .upsert_batch(vec![("o9".to_string(), json!({"customer": "alice"}))])
        .unwrap();
    assert_eq!(s.orders.count(), 3);
}

#[test]
fn restrict_refuses_the_delete_of_a_referenced_parent() {
    let s = shop(OnDelete::Restrict);
    s.orders
        .insert(json!({"_key": "o1", "customer": "alice"}))
        .unwrap();

    assert!(is_violation(&s.customers.delete("alice")));
    assert!(is_violation(
        &s.customers
            .delete_batch(vec!["bob".to_string(), "alice".to_string()])
    ));
    // The batch is one write: bob stays too.
    assert_eq!(s.customers.count(), 2);

    // Nobody refers to bob.
    s.customers.delete("bob").unwrap();
    assert!(matches!(
        s.customers.delete("bob"),
        Err(DbError::DocumentNotFound(_))
    ));

    s.orders.delete("o1").unwrap();
    s.customers.delete("alice").unwrap();
    assert_eq!(s.customers.count(), 0);
}

#[test]
fn cascade_deletes_the_children_with_the_parent() {
    let s = shop(OnDelete::Cascade);
    for (key, customer) in [("o1", "alice"), ("o2", "alice"), ("o3", "bob")] {
        s.orders
            .insert(json!({"_key": key, "customer": customer}))
            .unwrap();
    }
    let mut events = s.orders.change_sender.subscribe();

    s.customers.delete("alice").unwrap();

    assert!(s.customers.get("alice").is_err());
    assert!(s.orders.get("o1").is_err());
    assert!(s.orders.get("o2").is_err());
    assert!(s.orders.get("o3").is_ok());
    assert_eq!(s.orders.count(), 1);
    assert_eq!(s.customers.count(), 1);
    // The index entries went with the documents.
    assert!(s
        .orders
        .index_lookup_eq("customer", &json!("alice"))
        .unwrap()
        .is_empty());
    // The children's deletes are announced like any other.
    let mut deleted = Vec::new();
    while let Ok(event) = events.try_recv() {
        deleted.push(event.key);
    }
    deleted.sort();
    assert_eq!(deleted, ["o1", "o2"]);
}

#[test]
fn set_null_clears_the_childrens_field() {
    let s = shop(OnDelete::SetNull);
    s.orders
        .insert(json!({"_key": "o1", "customer": "alice", "total": 3}))
        .unwrap();
    let before = s.orders.get("o1").unwrap();

    s.customers.delete("alice").unwrap();

    let after = s.orders.get("o1").unwrap();
    assert_eq!(after.data["customer"], json!(null));
    assert_eq!(after.data["total"], json!(3));
    assert_ne!(after.revision(), before.revision());
    assert!(s
        .orders
        .index_lookup_eq("customer", &json!("alice"))
        .unwrap()
        .is_empty());
}

#[test]
fn a_cascade_walks_a_tree_in_one_collection() {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    db.create_collection("nodes".to_string(), None).unwrap();
    let nodes = db.get_collection("nodes").unwrap();
    nodes
        .add_foreign_key(ForeignKeySpec::new("parent", "nodes", OnDelete::Cascade))
        .unwrap();

    // A document may refer to itself, and to another of its batch.
    nodes
        .insert(json!({"_key": "self", "parent": "self"}))
        .unwrap();
    nodes
        .insert_batch(vec![
            json!({"_key": "leaf", "parent": "mid"}),
            json!({"_key": "mid", "parent": "root"}),
            json!({"_key": "root"}),
            json!({"_key": "other"}),
        ])
        .unwrap();

    nodes.delete("root").unwrap();
    assert!(nodes.get("mid").is_err());
    assert!(nodes.get("leaf").is_err());
    assert!(nodes.get("other").is_ok());
    nodes.delete("self").unwrap();
    assert_eq!(nodes.count(), 1);

    // The collection refers to itself only: it can be truncated and dropped.
    nodes.truncate().unwrap();
    db.delete_collection("nodes").unwrap();
}

#[test]
fn declaring_a_key_refuses_orphans_and_keeps_nothing() {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    db.create_collection("customers".to_string(), None).unwrap();
    db.create_collection("orders".to_string(), None).unwrap();
    let customers = db.get_collection("customers").unwrap();
    let orders = db.get_collection("orders").unwrap();
    customers.insert(json!({"_key": "alice"})).unwrap();
    orders
        .insert(json!({"_key": "o1", "customer": "alice"}))
        .unwrap();
    orders
        .insert(json!({"_key": "o2", "customer": "ghost"}))
        .unwrap();

    let orphans = orders
        .foreign_key_orphans(&ForeignKeySpec::new(
            "customer",
            "customers",
            OnDelete::Restrict,
        ))
        .unwrap();
    assert_eq!(orphans.count, 1);
    assert_eq!(orphans.sample, [json!({"_key": "o2", "customer": "ghost"})]);

    let refused = orders.add_foreign_key(ForeignKeySpec::new(
        "customer",
        "customers",
        OnDelete::Restrict,
    ));
    assert!(is_violation(&refused), "{:?}", refused);
    assert!(orders.foreign_keys().is_empty());
    assert!(orders.get_all_indexes().is_empty());
    // Not enforced either.
    orders
        .insert(json!({"_key": "o3", "customer": "nobody"}))
        .unwrap();
    orders.delete("o3").unwrap();

    orders.delete("o2").unwrap();
    let key = orders
        .add_foreign_key(ForeignKeySpec::new(
            "customer",
            "customers",
            OnDelete::Restrict,
        ))
        .unwrap();
    assert_eq!(key.name, "fk_customer");
    assert_eq!(key.index, "fk_customer");
    assert_eq!(orders.foreign_keys(), [key]);

    // One key per field, one per name; the parent must exist.
    assert!(orders
        .add_foreign_key(ForeignKeySpec::new(
            "customer",
            "customers",
            OnDelete::Cascade
        ))
        .is_err());
    assert!(matches!(
        orders.add_foreign_key(ForeignKeySpec::new("shop", "shops", OnDelete::Restrict)),
        Err(DbError::CollectionNotFound(_))
    ));
    assert!(orders
        .add_foreign_key(ForeignKeySpec::new("a.b", "customers", OnDelete::Restrict))
        .is_err());
}

#[test]
fn what_a_key_stands_on_cannot_be_removed_under_it() {
    let s = shop(OnDelete::Restrict);
    s.orders
        .insert(json!({"_key": "o1", "customer": "alice"}))
        .unwrap();
    let db = s.engine.get_database("app").unwrap();

    assert!(matches!(
        s.orders.drop_index("fk_customer"),
        Err(DbError::ConflictError(_))
    ));
    assert!(matches!(
        db.delete_collection("customers"),
        Err(DbError::ConflictError(_))
    ));
    assert!(is_violation(&s.customers.truncate()));

    // Truncating the child first leaves nothing that refers to the parent.
    s.orders.truncate().unwrap();
    s.customers.truncate().unwrap();

    // Without the key, all of it goes.
    s.orders.drop_foreign_key("fk_customer").unwrap();
    assert!(s.orders.drop_foreign_key("fk_customer").is_err());
    s.orders.drop_index("fk_customer").unwrap();
    s.orders
        .insert(json!({"_key": "o2", "customer": "nobody"}))
        .unwrap();
    db.delete_collection("customers").unwrap();
}

#[test]
fn dropping_the_child_frees_the_parent() {
    let s = shop(OnDelete::Restrict);
    s.orders
        .insert(json!({"_key": "o1", "customer": "alice"}))
        .unwrap();
    let db = s.engine.get_database("app").unwrap();
    db.delete_collection("orders").unwrap();
    s.customers.delete("alice").unwrap();
    db.delete_collection("customers").unwrap();
}

#[test]
fn an_orphan_from_before_the_key_can_still_be_updated() {
    // `unchanged` references are not looked up again: an update of another
    // field goes through whatever the parent's state.
    let s = shop(OnDelete::Restrict);
    s.orders
        .insert(json!({"_key": "o1", "customer": "alice", "n": 1}))
        .unwrap();
    s.orders.update("o1", json!({"n": 2})).unwrap();
    assert_eq!(s.orders.get("o1").unwrap().data["n"], json!(2));
}

fn in_transaction(
    s: &Shop,
    stage: impl FnOnce(
        &mut solidb::transaction::Transaction,
        &Arc<solidb::transaction::wal::WalWriter>,
        &Arc<solidb::transaction::lock_manager::LockManager>,
    ),
) -> Result<(), DbError> {
    let manager = s.engine.transaction_manager().unwrap();
    let tx_id = manager.begin(IsolationLevel::ReadCommitted).unwrap();
    {
        let tx_arc = manager.get(tx_id).unwrap();
        let mut tx = tx_arc.write().unwrap();
        stage(
            &mut tx,
            &manager.wal().clone(),
            &manager.lock_manager().clone(),
        );
    }
    s.engine.commit_transaction(tx_id)
}

#[test]
fn a_commit_is_checked_as_a_whole() {
    let s = shop(OnDelete::Restrict);

    // Parent and child in one transaction, the child staged first.
    in_transaction(&s, |tx, wal, locks| {
        s.orders
            .insert_tx(tx, wal, locks, json!({"_key": "o1", "customer": "carol"}))
            .unwrap();
        s.customers
            .insert_tx(tx, wal, locks, json!({"_key": "carol"}))
            .unwrap();
    })
    .unwrap();
    assert!(s.orders.get("o1").is_ok());

    // A child without its parent: nothing of the transaction lands.
    let missing = in_transaction(&s, |tx, wal, locks| {
        s.customers
            .insert_tx(tx, wal, locks, json!({"_key": "dave"}))
            .unwrap();
        s.orders
            .insert_tx(tx, wal, locks, json!({"_key": "o2", "customer": "erin"}))
            .unwrap();
    });
    assert!(is_violation(&missing), "{:?}", missing);
    assert!(s.customers.get("dave").is_err());

    // A parent deleted with a child still referring to it.
    let referenced = in_transaction(&s, |tx, wal, locks| {
        s.customers.delete_tx(tx, wal, locks, "carol").unwrap();
    });
    assert!(is_violation(&referenced), "{:?}", referenced);

    // A child inserted for a parent the same transaction deletes.
    let gone = in_transaction(&s, |tx, wal, locks| {
        s.orders
            .insert_tx(tx, wal, locks, json!({"_key": "o3", "customer": "bob"}))
            .unwrap();
        s.customers.delete_tx(tx, wal, locks, "bob").unwrap();
    });
    assert!(is_violation(&gone), "{:?}", gone);
    assert!(s.customers.get("bob").is_ok());

    // Parent and children deleted together.
    in_transaction(&s, |tx, wal, locks| {
        s.customers.delete_tx(tx, wal, locks, "carol").unwrap();
        s.orders.delete_tx(tx, wal, locks, "o1").unwrap();
    })
    .unwrap();
    assert!(s.customers.get("carol").is_err());
    assert_eq!(s.orders.count(), 0);
}

#[test]
fn a_commit_cascades() {
    let s = shop(OnDelete::Cascade);
    s.orders
        .insert(json!({"_key": "o1", "customer": "alice"}))
        .unwrap();
    s.orders
        .insert(json!({"_key": "o2", "customer": "bob"}))
        .unwrap();

    in_transaction(&s, |tx, wal, locks| {
        s.customers.delete_tx(tx, wal, locks, "alice").unwrap();
    })
    .unwrap();

    assert!(s.orders.get("o1").is_err());
    assert!(s.orders.get("o2").is_ok());
    assert_eq!(s.orders.count(), 1);
}

/// Children are inserted for a parent while it is deleted, from several
/// threads: whichever order they land in, no child is left referring to a
/// parent that is gone.
#[test]
fn a_parent_cannot_go_between_a_childs_check_and_its_write() {
    for on_delete in [OnDelete::Restrict, OnDelete::Cascade] {
        let s = Arc::new(shop(on_delete));
        for round in 0..40 {
            let parent = format!("p{}", round);
            s.customers.insert(json!({"_key": parent})).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let written = Arc::new(AtomicUsize::new(0));

            let writers: Vec<_> = (0..4)
                .map(|w| {
                    let (s, stop, written, parent) =
                        (s.clone(), stop.clone(), written.clone(), parent.clone());
                    std::thread::spawn(move || {
                        let mut n = 0;
                        while !stop.load(Ordering::Relaxed) && n < 200 {
                            let key = format!("{}-{}-{}", parent, w, n);
                            let inserted = if n % 2 == 0 {
                                s.orders
                                    .insert(json!({"_key": key, "customer": parent}))
                                    .map(|_| ())
                            } else {
                                s.orders
                                    .insert_batch(vec![json!({"_key": key, "customer": parent})])
                                    .map(|_| ())
                            };
                            if inserted.is_ok() {
                                written.fetch_add(1, Ordering::Relaxed);
                            }
                            n += 1;
                        }
                    })
                })
                .collect();

            // Delete the parent once some children are in.
            while written.load(Ordering::Relaxed) < 3 {
                std::thread::yield_now();
            }
            let deleted = s.customers.delete(&parent);
            stop.store(true, Ordering::Relaxed);
            for writer in writers {
                writer.join().unwrap();
            }

            let children = s
                .orders
                .index_lookup_eq("customer", &json!(parent))
                .unwrap()
                .len();
            match on_delete {
                OnDelete::Restrict => {
                    assert!(is_violation(&deleted), "{:?}", deleted);
                    assert!(s.customers.get(&parent).is_ok());
                    assert_eq!(children, written.load(Ordering::Relaxed));
                }
                _ => {
                    deleted.unwrap();
                    assert!(s.customers.get(&parent).is_err());
                    assert_eq!(children, 0, "children outlived '{}'", parent);
                }
            }
        }
        let orphans = s
            .orders
            .foreign_key_orphans(&ForeignKeySpec::new(
                "customer",
                "customers",
                OnDelete::Restrict,
            ))
            .unwrap();
        assert_eq!(orphans.count, 0, "{:?}", orphans.sample);
    }
}

#[test]
fn an_expired_parent_follows_on_delete() {
    for on_delete in [OnDelete::Restrict, OnDelete::Cascade] {
        let s = shop(on_delete);
        s.customers
            .create_ttl_index("ttl".to_string(), "seen".to_string(), 1)
            .unwrap();
        // Expired long ago; `bob` has no child.
        s.customers.update("alice", json!({"seen": 1000})).unwrap();
        s.customers.update("bob", json!({"seen": 1000})).unwrap();
        s.orders
            .insert(json!({"_key": "o1", "customer": "alice"}))
            .unwrap();

        let reaped = s.customers.cleanup_all_expired_documents();
        match on_delete {
            OnDelete::Restrict => {
                // The chunk is one write, refused for `alice`.
                assert!(is_violation(&reaped), "{:?}", reaped);
                assert!(s.customers.get("alice").is_ok());
                assert!(s.orders.get("o1").is_ok());
            }
            _ => {
                assert_eq!(reaped.unwrap(), 2);
                assert!(s.customers.get("alice").is_err());
                assert!(s.orders.get("o1").is_err());
                assert_eq!(s.customers.count(), 0);
            }
        }
    }
}

#[test]
fn an_edge_cannot_outlive_its_vertices() {
    let s = shop(OnDelete::Restrict);
    let db = s.engine.get_database("app").unwrap();
    db.create_collection("follows".to_string(), Some("edge".to_string()))
        .unwrap();
    let follows = db.get_collection("follows").unwrap();
    s.customers.insert(json!({"_key": "carol"})).unwrap();

    // An end that is already dangling refuses the key.
    follows
        .insert(json!({"_key": "bad", "_from": "customers/alice", "_to": "customers/ghost"}))
        .unwrap();
    let to = ForeignKeySpec::new("_to", "customers", OnDelete::Cascade);
    assert_eq!(follows.foreign_key_orphans(&to).unwrap().count, 1);
    assert!(is_violation(&follows.add_foreign_key(to.clone())));
    follows.delete("bad").unwrap();

    let from = follows
        .add_foreign_key(ForeignKeySpec::new("_from", "customers", OnDelete::Cascade))
        .unwrap();
    // By `_id`, on the index an edge collection already has.
    assert_eq!(
        serde_json::to_value(&from).unwrap(),
        json!({"name": "fk_from", "field": "_from", "by": "id", "references": "customers",
               "on_delete": "cascade", "index": "_edge_from_idx"})
    );
    follows
        .add_foreign_key(ForeignKeySpec::new("_to", "customers", OnDelete::Restrict))
        .unwrap();
    // An edge without an end is not an edge; an end is not a `_key`.
    assert!(follows
        .add_foreign_key(ForeignKeySpec::new("_to", "customers", OnDelete::SetNull))
        .is_err());
    assert!(s
        .orders
        .add_foreign_key(ForeignKeySpec::new(
            "_from",
            "customers",
            OnDelete::Restrict
        ))
        .is_err());

    // Both forms of an `_id` name the vertex.
    follows
        .insert(json!({"_key": "a-b", "_from": "customers/alice", "_to": "app:customers/bob"}))
        .unwrap();
    follows
        .insert(json!({"_key": "b-c", "_from": "app:customers/bob", "_to": "customers/carol"}))
        .unwrap();
    for (from, to) in [
        ("customers/alice", "customers/ghost"),
        ("customers/ghost", "customers/alice"),
        ("orders/alice", "customers/alice"),
        ("other:customers/alice", "customers/alice"),
        ("alice", "customers/alice"),
    ] {
        let refused = follows.insert(json!({"_from": from, "_to": to}));
        assert!(is_violation(&refused), "{from} -> {to}: {:?}", refused);
    }
    assert!(is_violation(
        &follows.update("a-b", json!({"_to": "customers/ghost"}))
    ));

    // `_to` restricts: bob is followed by alice. `_from` cascades: carol
    // follows nobody and is followed by bob, so she stays too.
    assert!(is_violation(&s.customers.delete("bob")));
    assert!(is_violation(&s.customers.delete("carol")));
    // Alice only follows: her edge goes with her.
    s.customers.delete("alice").unwrap();
    assert!(follows.get("a-b").is_err());
    assert!(follows.get("b-c").is_ok());
    // Now nobody follows bob; deleting him takes `b-c`, in either form.
    s.customers.delete("bob").unwrap();
    assert_eq!(follows.count(), 0);
    s.customers.delete("carol").unwrap();
}

#[test]
fn a_field_can_refer_by_id() {
    let s = shop(OnDelete::Restrict);
    let db = s.engine.get_database("app").unwrap();
    db.create_collection("notes".to_string(), None).unwrap();
    let notes = db.get_collection("notes").unwrap();
    let mut spec = ForeignKeySpec::new("about", "customers", OnDelete::SetNull);
    spec.by = Some(solidb::storage::foreign_keys::ReferenceBy::Id);
    notes.add_foreign_key(spec).unwrap();

    notes
        .insert(json!({"_key": "n1", "about": "customers/alice"}))
        .unwrap();
    notes
        .insert(json!({"_key": "n2", "about": "app:customers/alice"}))
        .unwrap();
    assert!(is_violation(&notes.insert(json!({"about": "alice"}))));

    s.customers.delete("alice").unwrap();
    assert_eq!(notes.get("n1").unwrap().data["about"], json!(null));
    assert_eq!(notes.get("n2").unwrap().data["about"], json!(null));
}

#[test]
fn what_a_delete_cascades_into_is_reported() {
    use std::sync::Mutex;
    let s = shop(OnDelete::Cascade);
    let db = s.engine.get_database("app").unwrap();
    db.create_collection("notes".to_string(), None).unwrap();
    let notes = db.get_collection("notes").unwrap();
    notes
        .add_foreign_key(ForeignKeySpec::new("about", "customers", OnDelete::SetNull))
        .unwrap();
    s.orders
        .insert(json!({"_key": "o1", "customer": "alice"}))
        .unwrap();
    notes
        .insert(json!({"_key": "n1", "about": "alice"}))
        .unwrap();

    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    s.engine.observe_cascades(move |writes| {
        sink.lock().unwrap().extend(writes);
    });

    // Nothing cascades: nothing is reported.
    s.customers.delete("bob").unwrap();
    assert!(seen.lock().unwrap().is_empty());

    s.customers.delete("alice").unwrap();
    let mut writes: Vec<_> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|w| (w.collection.clone(), w.key.clone(), w.document.clone()))
        .collect();
    writes.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(writes.len(), 2);
    // The note as it is stored now, the order as deleted. The customer's own
    // delete is the caller's to log.
    assert_eq!(
        (writes[0].0.as_str(), writes[0].1.as_str()),
        ("notes", "n1")
    );
    let note = writes[0].2.as_ref().unwrap();
    assert_eq!(note["about"], json!(null));
    assert_eq!(note["_rev"], json!(notes.get("n1").unwrap().revision()));
    assert_eq!(
        (writes[1].0.as_str(), writes[1].1.as_str(), &writes[1].2),
        ("orders", "o1", &None)
    );
}

#[test]
fn a_replicated_write_is_neither_checked_nor_followed() {
    use solidb::storage::foreign_keys::unchecked;
    let s = shop(OnDelete::Cascade);
    s.orders
        .insert(json!({"_key": "o1", "customer": "alice"}))
        .unwrap();

    // A child whose parent has not arrived yet, a parent whose children's
    // deletes arrive as entries of their own.
    unchecked(|| {
        s.orders
            .upsert_batch(vec![("o2".to_string(), json!({"customer": "later"}))])
            .unwrap();
        s.orders
            .insert_or_replace(json!({"_key": "o3", "customer": "later"}))
            .unwrap();
        s.customers.delete("alice").unwrap();
    });
    assert!(s.orders.get("o1").is_ok());
    assert_eq!(s.orders.count(), 3);

    // Outside, the key is enforced as before.
    assert!(is_violation(
        &s.orders.insert(json!({"_key": "o4", "customer": "later"}))
    ));
    s.customers.delete("bob").unwrap();
}

#[test]
fn a_key_declared_elsewhere_is_stored_as_declared() {
    let s = shop(OnDelete::Restrict);
    let key = s.orders.foreign_keys().remove(0);
    let db = s.engine.get_database("app").unwrap();
    db.create_collection("copies".to_string(), None).unwrap();
    let copies = db.get_collection("copies").unwrap();
    // Documents that may be ahead of their parents are not a reason to
    // refuse a definition another node accepted.
    copies
        .insert(json!({"_key": "c1", "customer": "not-here-yet"}))
        .unwrap();

    copies.apply_foreign_key(&key).unwrap();
    copies.apply_foreign_key(&key).unwrap();
    assert_eq!(copies.foreign_keys(), std::slice::from_ref(&key));
    assert!(copies.get_index_for_field("customer").is_some());
    assert!(is_violation(
        &copies.insert(json!({"_key": "c2", "customer": "nobody"}))
    ));

    copies.apply_foreign_key_drop(&key.name).unwrap();
    copies.apply_foreign_key_drop(&key.name).unwrap();
    assert!(copies.foreign_keys().is_empty());
}
