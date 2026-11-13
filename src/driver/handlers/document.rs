//! Document commands of the native driver.
//!
//! None of these drops the shared query-result cache itself: every document
//! write does it in the collection, once committed (`Collection::emit_change`
//! and its batch form), whichever protocol it came over. These handlers used
//! to drop it a second time.

use crate::driver::protocol::{DriverError, RawWrite, Response};
use crate::driver::DriverHandler;
use crate::storage::raw_write::{canonical_document, CanonicalDoc};
use crate::sync::protocol::Operation;

pub fn handle_get(
    handler: &DriverHandler,
    database: String,
    collection: String,
    key: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            // The stored bytes copied as the document decoding them and
            // encoding the value would send (`storage::raw_rows`). A missing
            // or unreadable document takes the decode, which reports it.
            let doc_key = [crate::storage::Collection::doc_key(&key)];
            if let Ok(Some(row)) =
                coll.raw_rows_of(&doc_key, &crate::storage::raw_rows::RawShape::Whole)
            {
                return Response::raw_rows(row);
            }
            match coll.get_value(&key) {
                Ok(value) => Response::ok(value),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_insert(
    handler: &DriverHandler,
    database: String,
    collection: String,
    key: Option<String>,
    document: serde_json::Value,
) -> Response {
    match handler.get_collection_for_write(&database, &collection) {
        Ok(coll) => {
            // If key provided, add it to document; otherwise insert() will auto-generate
            let mut doc_data = document;
            if let Some(k) = key {
                if let Some(obj) = doc_data.as_object_mut() {
                    obj.insert("_key".to_string(), serde_json::json!(k));
                }
            }
            match coll.insert_value(doc_data) {
                Ok((key, value)) => {
                    handler.log_replication(
                        &database,
                        &collection,
                        Operation::Insert,
                        &key,
                        Some(&value),
                    );
                    Response::ok(value)
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_update(
    handler: &DriverHandler,
    database: String,
    collection: String,
    key: String,
    document: serde_json::Value,
    merge: bool,
) -> Response {
    match handler.get_collection_for_write(&database, &collection) {
        Ok(coll) => {
            let result = if merge {
                // The stored document's fields with the given ones over them,
                // read and written under the key's lock.
                coll.merge_update(&key, document)
            } else {
                coll.update(&key, document).map(|doc| doc.to_value())
            };

            match result {
                Ok(value) => {
                    handler.log_replication(
                        &database,
                        &collection,
                        Operation::Update,
                        &key,
                        Some(&value),
                    );
                    Response::ok(value)
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_delete(
    handler: &DriverHandler,
    database: String,
    collection: String,
    key: String,
) -> Response {
    match handler.get_collection_for_write(&database, &collection) {
        Ok(coll) => match coll.delete(&key) {
            Ok(_) => {
                handler.log_replication(&database, &collection, Operation::Delete, &key, None);
                Response::ok_empty()
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(e),
    }
}

pub fn handle_list(
    handler: &DriverHandler,
    database: String,
    collection: String,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            // Read only the requested page; the total comes from the
            // collection's own count rather than a full scan.
            let offset = offset.unwrap_or(0);
            let limit = limit.unwrap_or(100);
            let total = coll.count();
            let docs = coll.scan_values_range(offset, Some(limit));

            Response::Ok {
                data: Some(serde_json::json!(docs)),
                count: Some(total),
                tx_id: None,
            }
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_bulk_insert(
    handler: &DriverHandler,
    database: String,
    collection: String,
    documents: Vec<serde_json::Value>,
) -> Response {
    match handler.get_collection_for_write(&database, &collection) {
        Ok(coll) => {
            // Use batch insert for efficiency
            match coll.insert_batch(documents) {
                Ok(docs) => {
                    handler.log_replication_batch(&database, &collection, Operation::Insert, &docs);
                    Response::ok_count(docs.len())
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(e),
    }
}

/// A document write from the client's MessagePack (`storage::raw_write`),
/// ready to run: its collection and the documents in stored form.
pub(crate) struct PreparedWrite {
    database: String,
    collection: String,
    coll: crate::storage::Collection,
    meta: std::sync::Arc<crate::storage::collection::index_meta::IndexMetaSnapshot>,
    kind: PreparedKind,
}

enum PreparedKind {
    Insert {
        key: Option<String>,
        doc: CanonicalDoc,
    },
    BulkInsert {
        docs: Vec<CanonicalDoc>,
    },
    MergeUpdate {
        key: String,
        changes: CanonicalDoc,
    },
}

/// Whether a write to `collection` must be logged for replication: the
/// log then needs the decoded document.
fn logs_writes(handler: &DriverHandler, collection: &str) -> bool {
    !crate::server::handlers::system::is_physical_shard_collection(collection)
        && handler
            .replication
            .as_ref()
            .is_some_and(|log| log.is_active())
}

/// Get `write` ready to run without decoding its documents, or `None` when
/// it must take the decoded path: not authenticated yet, a replication log
/// to feed, a collection whose writes need decoded documents
/// (`Collection::raw_write_ready`), or a document the transcoder refuses —
/// in which case the frame is one the decoder may refuse too, and it is
/// decoded at once, as before.
///
/// Nothing here is visible to the client: the authorization is checked
/// again, with the session as it stands, by [`run_raw`].
pub(crate) fn prepare_raw(handler: &DriverHandler, write: RawWrite<'_>) -> Option<PreparedWrite> {
    handler.authenticated_db.as_ref()?;
    let (database, collection) = match &write {
        RawWrite::Insert {
            database,
            collection,
            ..
        }
        | RawWrite::BulkInsert {
            database,
            collection,
            ..
        }
        | RawWrite::MergeUpdate {
            database,
            collection,
            ..
        } => (database.clone(), collection.clone()),
    };
    if logs_writes(handler, &collection) {
        return None;
    }
    let coll = handler
        .get_collection_for_write(&database, &collection)
        .ok()?;
    let meta = coll.raw_write_ready()?;
    // Documents sit at the frame's nesting level 1 in an insert or update,
    // inside the array at level 2 in a bulk insert, as the decoder counts.
    let kind = match write {
        RawWrite::Insert { key, document, .. } => PreparedKind::Insert {
            key,
            doc: canonical_document(document, 0)?,
        },
        RawWrite::BulkInsert { documents, .. } => PreparedKind::BulkInsert {
            docs: documents
                .iter()
                .map(|doc| canonical_document(doc, 1))
                .collect::<Option<_>>()?,
        },
        RawWrite::MergeUpdate { key, document, .. } => PreparedKind::MergeUpdate {
            key,
            changes: canonical_document(document, 0)?,
        },
    };
    Some(PreparedWrite {
        database,
        collection,
        coll,
        meta,
        kind,
    })
}

/// Run a prepared write; `None` when it must take the decoded path after
/// all (the session lost the right, a replication log turned on, or the
/// stored document cannot be merged this way), nothing having been written.
/// The replies and errors are those of [`handle_insert`],
/// [`handle_bulk_insert`] and [`handle_update`].
pub(crate) fn run_raw(handler: &DriverHandler, write: PreparedWrite) -> Option<Response> {
    let PreparedWrite {
        database,
        collection,
        coll,
        meta,
        kind,
    } = write;
    // The checks `execute_command` and `get_collection_for_write` make; a
    // refusal is left to them, to answer and log.
    crate::server::AuthorizationService::check_permission_raw(
        &handler.session_permissions,
        crate::server::PermissionAction::Write,
        Some(&database),
        handler.session_scoped_databases.as_deref(),
    )
    .ok()?;
    let actor = crate::storage::WriteActor::client(handler.query_principal(&database).can_admin);
    crate::storage::check_write_access(&collection, actor).ok()?;
    if logs_writes(handler, &collection) {
        return None;
    }
    let error =
        |e: crate::error::DbError| Response::error(DriverError::DatabaseError(e.to_string()));
    Some(match kind {
        PreparedKind::Insert { key, doc } => match coll.insert_raw(&meta, key.as_deref(), &doc)? {
            Ok((_, reply)) => Response::raw_rows(reply),
            Err(e) => error(e),
        },
        PreparedKind::BulkInsert { docs } => match coll.insert_batch_raw(&meta, &docs)? {
            Ok(count) => Response::ok_count(count),
            Err(e) => error(e),
        },
        PreparedKind::MergeUpdate { key, changes } => {
            match coll.merge_update_raw(&meta, &key, &changes)? {
                Ok(reply) => Response::raw_rows(reply),
                Err(e) => error(e),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::protocol::encode_response;
    use crate::storage::StorageEngine;
    use serde_json::json;
    use std::sync::Arc;

    /// A get is copied from the stored bytes, and the client receives what
    /// decoding them and encoding the value sends; a missing key is still
    /// reported.
    #[test]
    fn get_sends_the_decoded_documents_bytes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
        engine.create_database("d".to_string()).unwrap();
        let db = engine.get_database("d").unwrap();
        db.create_collection("c".to_string(), None).unwrap();
        let coll = db.get_collection("c").unwrap();
        coll.insert(json!({
            "_key": "k1",
            "n": -3,
            "f": 1.5,
            "s": "é",
            "arr": [1, "two", {"three": null}],
            "obj": {"b": 2, "a": [true]},
        }))
        .unwrap();
        let h = DriverHandler::new(engine.clone(), None);

        let resp = handle_get(&h, "d".into(), "c".into(), "k1".into());
        assert!(matches!(resp, Response::RawRows { .. }), "{resp:?}");
        let ordinary = Response::ok(coll.get_value("k1").unwrap());
        assert_eq!(
            encode_response(&resp).unwrap(),
            encode_response(&ordinary).unwrap()
        );

        let missing = handle_get(&h, "d".into(), "c".into(), "nope".into());
        assert!(
            matches!(&missing, Response::Error { error } if error.to_string().contains("nope")),
            "{missing:?}"
        );
    }

    // ------------------------------------------------------------------
    // The raw write path against the decoded one.

    use crate::driver::protocol::{encode_command, parse_raw_write, Command};
    use crate::storage::keyspace::KsDbExt;
    use crate::storage::kv::IterMode;
    use crate::storage::serializer::{deserialize_doc, serialize_doc, StoredDocRef};
    use crate::storage::IndexType;

    /// An engine with database `d`, collection `c` with every kind of
    /// definition the raw path keeps (vector indexes send it the decoded
    /// way), and an authenticated admin connection on it.
    fn engine_with_indexes() -> (tempfile::TempDir, Arc<StorageEngine>, DriverHandler) {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
        engine.create_database("d".to_string()).unwrap();
        let db = engine.get_database("d").unwrap();
        db.create_collection("c".to_string(), None).unwrap();
        let coll = db.get_collection("c").unwrap();
        for (name, fields, kind, unique) in [
            ("by_status", vec!["status"], IndexType::Persistent, false),
            (
                "by_city",
                vec!["status", "address.city"],
                IndexType::Persistent,
                false,
            ),
            ("by_key", vec!["_key"], IndexType::Hash, false),
            ("by_email", vec!["email"], IndexType::Hash, true),
            ("by_n", vec!["n"], IndexType::Persistent, false),
            ("grams", vec!["note"], IndexType::Trigram, false),
        ] {
            coll.create_index(
                name.to_string(),
                fields.into_iter().map(String::from).collect(),
                kind,
                unique,
            )
            .unwrap();
        }
        coll.create_fulltext_index("ft".to_string(), vec!["note".to_string()], None)
            .unwrap();
        coll.create_geo_index("geo".to_string(), "loc".to_string())
            .unwrap();
        coll.create_ttl_index("ttl".to_string(), "expires".to_string(), 60)
            .unwrap();
        let mut h = DriverHandler::new(engine.clone(), None);
        h.authenticated_db = Some("d".into());
        h.session_permissions
            .insert(crate::server::Permission::global_admin());
        (tmp, engine, h)
    }

    fn frame(cmd: &Command) -> Vec<u8> {
        encode_command(cmd).unwrap()[4..].to_vec()
    }

    /// The command through the raw path; `None` if it went the decoded way.
    fn raw(h: &DriverHandler, cmd: &Command) -> Option<Response> {
        let payload = frame(cmd);
        let write = prepare_raw(h, parse_raw_write(&payload)?)?;
        run_raw(h, write)
    }

    /// The command through the decoded path, as the connection loop runs it.
    fn decoded(h: &DriverHandler, cmd: Command) -> Response {
        match cmd {
            Command::Insert {
                database,
                collection,
                key,
                document,
            } => handle_insert(h, database, collection, key, document),
            Command::BulkInsert {
                database,
                collection,
                documents,
            } => handle_bulk_insert(h, database, collection, documents),
            Command::Update {
                database,
                collection,
                key,
                document,
                merge,
            } => handle_update(h, database, collection, key, document, merge),
            other => panic!("{other:?}"),
        }
    }

    /// Every key and value of the collection: documents (data and the
    /// header fields that do not depend on time or chance), index, fulltext,
    /// geo and TTL entries, the count.
    fn contents(engine: &StorageEngine) -> Vec<(Vec<u8>, Vec<u8>)> {
        let coll = engine
            .get_database("d")
            .unwrap()
            .get_collection("c")
            .unwrap();
        coll.flush_stats();
        let cf = coll.ks.handle(&coll.db).unwrap();
        coll.db
            .iterator_ks(&cf, IterMode::Start)
            .map(|item| {
                let (k, v) = item.unwrap();
                let v = match StoredDocRef::parse(&v) {
                    Some(doc) if k.starts_with(b"doc:") => {
                        // The stored bytes are what the decoded path writes
                        // for this very document, header included.
                        assert_eq!(
                            serialize_doc(&deserialize_doc(&v).unwrap()).unwrap(),
                            v.to_vec()
                        );
                        [doc.key, doc.id, doc.data].concat()
                    }
                    _ => v.to_vec(),
                };
                (k.to_vec(), v)
            })
            .filter(|(k, _)| !k.starts_with(b"_stats:count"))
            .collect()
    }

    /// The reply with the fields that depend on time or chance blanked, and
    /// checked to be byte for byte what encoding its value gives.
    fn reply(resp: Response) -> String {
        let bytes = encode_response(&resp).unwrap();
        let decoded: Response = crate::driver::protocol::decode_message(&bytes[4..]).unwrap();
        assert_eq!(encode_response(&decoded).unwrap(), bytes, "{resp:?}");
        let mut text = format!("{decoded:?}");
        if let Response::Ok {
            data: Some(serde_json::Value::Object(map)),
            ..
        } = &decoded
        {
            for f in ["_rev", "_created_at", "_updated_at"] {
                if let Some(v) = map.get(f) {
                    text = text.replace(&v.to_string(), "<blank>");
                }
            }
        }
        text
    }

    fn orders() -> Vec<serde_json::Value> {
        vec![
            json!({"status": "pending", "email": "a@x", "n": 1, "note": "Leave the parcel",
                   "address": {"zip": "69001", "city": "Lyon"}, "loc": {"lat": 45.7, "lon": 4.8},
                   "expires": 1_900_000_000u64, "tags": ["b", "a"], "_rev": "forged"}),
            json!({"note": "Ring twice ✓", "status": "shipped", "n": -2.5, "z": null,
                   "nested": {"b": [1, {"y": 2, "x": 1}], "a": true}}),
            json!({"big": u64::MAX, "neg": i64::MIN, "f": 1e300, "s": "\u{0}\"\\é𝄞"}),
            json!({}),
            json!({"status": "pending", "n": 0, "_id": "x/y", "_created_at": "bogus"}),
        ]
    }

    /// Inserts, a bulk insert and merge updates leave the same stored
    /// bytes, entries and replies on the raw path as on the decoded one, and
    /// fail the same way.
    #[test]
    fn raw_writes_are_the_decoded_writes() {
        let (_t1, raw_engine, rh) = engine_with_indexes();
        let (_t2, dec_engine, dh) = engine_with_indexes();
        let raw_coll = raw_engine
            .get_database("d")
            .unwrap()
            .get_collection("c")
            .unwrap();
        let mut events = raw_coll.change_sender.subscribe();

        let mut commands = Vec::new();
        for (i, doc) in orders().into_iter().enumerate() {
            let mut doc = doc;
            if i == 1 {
                doc["_key"] = json!("from-doc");
            }
            commands.push(Command::Insert {
                database: "d".into(),
                collection: "c".into(),
                key: (i != 1).then(|| format!("k{i}")),
                document: doc,
            });
        }
        // A conflict, a unique violation.
        commands.push(Command::Insert {
            database: "d".into(),
            collection: "c".into(),
            key: Some("k0".into()),
            document: json!({"a": 1}),
        });
        commands.push(Command::Insert {
            database: "d".into(),
            collection: "c".into(),
            key: Some("dup-email".into()),
            document: json!({"email": "a@x"}),
        });
        commands.push(Command::BulkInsert {
            database: "d".into(),
            collection: "c".into(),
            documents: (0..50)
                .map(|i| {
                    json!({"_key": format!("b{i}"), "status": (["x", "y"][i % 2]), "n": i,
                                "address": {"city": "Paris"}, "note": format!("bulk {i}")})
                })
                .collect(),
        });
        // A conflict inside a bulk insert, a duplicate key, an existing one.
        commands.push(Command::BulkInsert {
            database: "d".into(),
            collection: "c".into(),
            documents: vec![json!({"_key": "q"}), json!({"_key": "q"})],
        });
        commands.push(Command::BulkInsert {
            database: "d".into(),
            collection: "c".into(),
            documents: vec![json!({"_key": "fresh"}), json!({"_key": "b3"})],
        });
        for (key, changes) in [
            (
                "k0",
                json!({"status": "shipped", "n": 5, "_key": "ignored"}),
            ),
            (
                "from-doc",
                json!({"note": "Changed text", "new": {"b": 1, "a": 2}}),
            ),
            ("b7", json!({"address": {"city": "Lyon"}, "email": "b7@x"})),
            ("k3", json!({"status": "pending"})),
            ("k0", json!({"email": "a@x"})),
            ("missing", json!({"status": "x"})),
            ("b8", json!({"email": "a@x"})),
        ] {
            commands.push(Command::Update {
                database: "d".into(),
                collection: "c".into(),
                key: key.into(),
                document: changes,
                merge: true,
            });
        }

        let mut seen = 0;
        for cmd in commands {
            let before: std::collections::HashMap<String, serde_json::Value> = raw_coll
                .all()
                .into_iter()
                .map(|d| (d.key.clone(), d.to_value()))
                .collect();
            let r = raw(&rh, &cmd).unwrap_or_else(|| panic!("{cmd:?} went the decoded way"));
            let d = decoded(&dh, cmd.clone());
            assert_eq!(reply(r), reply(d), "{cmd:?}");
            assert_eq!(
                contents(&raw_engine),
                contents(&dec_engine),
                "after {cmd:?}"
            );

            // Each event carries the documents whole, as stored after the
            // write and before it.
            while let Ok(event) = events.try_recv() {
                let now = raw_coll.get(&event.key).unwrap().to_value();
                assert_eq!(event.data.as_ref(), Some(&now), "{cmd:?}");
                assert_eq!(event.old_data.as_ref(), before.get(&event.key), "{cmd:?}");
                seen += 1;
            }
        }
        assert!(seen > 50, "{seen} events");
        let dec_coll = dec_engine
            .get_database("d")
            .unwrap()
            .get_collection("c")
            .unwrap();
        assert_eq!(raw_coll.count(), dec_coll.count());
    }

    /// What the raw path leaves to the decoded one.
    #[test]
    fn raw_writes_fall_back() {
        let (_t, engine, mut h) = engine_with_indexes();
        let insert = |doc: serde_json::Value| Command::Insert {
            database: "d".into(),
            collection: "c".into(),
            key: None,
            document: doc,
        };
        // Documents it does not read: not an object, a `_key` that is not a
        // string, a binary value.
        assert!(raw(&h, &insert(json!([1, 2]))).is_none());
        assert!(raw(&h, &insert(json!({"_key": 5}))).is_none());
        let mut bin = rmp_serde::to_vec_named(&json!({
            "cmd": "insert", "database": "d", "collection": "c"
        }))
        .unwrap();
        bin[0] += 1; // one more field
        rmp::encode::write_str(&mut bin, "document").unwrap();
        rmp::encode::write_map_len(&mut bin, 1).unwrap();
        rmp::encode::write_str(&mut bin, "b").unwrap();
        rmp::encode::write_bin(&mut bin, b"x").unwrap();
        assert!(prepare_raw(&h, parse_raw_write(&bin).unwrap()).is_none());
        assert!(raw(&h, &insert(json!({"_key": "ok"}))).is_some());

        // Collections whose writes need decoded documents.
        let db = engine.get_database("d").unwrap();
        for name in ["edges", "schemed", "versioned", "vectors"] {
            db.create_collection(
                name.to_string(),
                (name == "edges").then(|| "edge".to_string()),
            )
            .unwrap();
        }
        db.get_collection("schemed")
            .unwrap()
            .set_json_schema(crate::storage::schema::CollectionSchema::new(
                "s".into(),
                json!({"type": "object"}),
                crate::storage::schema::SchemaValidationMode::Strict,
            ))
            .unwrap();
        db.get_collection("versioned")
            .unwrap()
            .enable_versioning()
            .unwrap();
        let config: crate::storage::VectorIndexConfig = serde_json::from_value(json!({
            "name": "emb", "field": "embedding", "dimension": 2
        }))
        .unwrap();
        db.get_collection("vectors")
            .unwrap()
            .create_vector_index(config)
            .unwrap();
        for name in ["edges", "schemed", "versioned", "vectors"] {
            let cmd = Command::Insert {
                database: "d".into(),
                collection: name.into(),
                key: None,
                document: json!({"a": 1}),
            };
            assert!(raw(&h, &cmd).is_none(), "{name}");
        }

        // An active replication log needs the decoded document.
        let log_dir = tempfile::TempDir::new().unwrap();
        let log = crate::sync::log::SyncLog::new("n".into(), log_dir.path().to_str().unwrap(), 10)
            .unwrap();
        h.replication = Some(Arc::new(log));
        assert!(raw(&h, &insert(json!({"a": 1}))).is_none());
        h.replication = None;

        // A session without the right, or not authenticated: the decoded
        // path answers (and logs) it.
        h.session_permissions.clear();
        assert!(raw(&h, &insert(json!({"a": 1}))).is_none());
        h.authenticated_db = None;
        assert!(raw(&h, &insert(json!({"a": 1}))).is_none());
    }

    /// A foreign key on an indexed field is checked on the raw path too,
    /// with the decoded path's answers.
    #[test]
    fn raw_writes_check_foreign_keys() {
        let (_t, engine, h) = engine_with_indexes();
        let db = engine.get_database("d").unwrap();
        db.create_collection("states".to_string(), None).unwrap();
        db.get_collection("states")
            .unwrap()
            .insert(json!({"_key": "open"}))
            .unwrap();
        let coll = db.get_collection("c").unwrap();
        coll.add_foreign_key(
            None,
            "status",
            "states",
            crate::storage::foreign_keys::OnDelete::Restrict,
        )
        .unwrap();
        // Served by the index that was there.
        assert_eq!(coll.foreign_keys()[0].index, "by_status");

        let violation = |r: &Response| {
            matches!(r, Response::Error { error }
                if error.to_string().contains("Foreign key violation"))
        };
        let insert = |key: &str, status: &str| Command::Insert {
            database: "d".into(),
            collection: "c".into(),
            key: Some(key.into()),
            document: json!({"status": status}),
        };
        let bulk = |keys: [&str; 2], status: &str| Command::BulkInsert {
            database: "d".into(),
            collection: "c".into(),
            documents: keys
                .iter()
                .map(|k| json!({"_key": k, "status": status}))
                .collect(),
        };
        let update = |status: &str| Command::Update {
            database: "d".into(),
            collection: "c".into(),
            key: "a".into(),
            document: json!({"status": status}),
            merge: true,
        };

        // Still the raw path, and it writes what has a parent.
        let ok = raw(&h, &insert("a", "open")).expect("the raw path");
        assert!(!matches!(ok, Response::Error { .. }), "{ok:?}");
        let ok = raw(&h, &bulk(["b", "c"], "open")).expect("the raw path");
        assert!(!matches!(ok, Response::Error { .. }), "{ok:?}");

        for cmd in [
            insert("x", "closed"),
            bulk(["y", "z"], "closed"),
            update("closed"),
        ] {
            let refused = raw(&h, &cmd).expect("the raw path");
            assert!(violation(&refused), "{refused:?}");
            let decoded = decoded(&h, cmd);
            assert!(violation(&decoded), "{decoded:?}");
        }
        assert_eq!(coll.count(), 3);
        assert_eq!(coll.get("a").unwrap().data["status"], json!("open"));
    }
}
