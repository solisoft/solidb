use crate::driver::protocol::{DriverError, Response};
use crate::driver::DriverHandler;
use crate::storage::CollectionSchema;
use serde_json::Value;

pub fn handle_list_databases(handler: &DriverHandler) -> Response {
    let dbs = handler.storage.list_databases();
    Response::ok(serde_json::json!(dbs))
}

pub fn handle_create_database(handler: &DriverHandler, name: String) -> Response {
    match handler.storage.create_database(name.clone()) {
        Ok(_) => {
            if let Some(ref log) = handler.replication {
                log.append(crate::sync::log::LogEntry::new_op(
                    name,
                    "",
                    crate::sync::protocol::Operation::CreateDatabase,
                    "",
                    None,
                ));
            }
            Response::ok_empty()
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_delete_database(handler: &DriverHandler, name: String) -> Response {
    match handler.storage.delete_database(&name) {
        Ok(_) => {
            if let Some(ref log) = handler.replication {
                log.append(crate::sync::log::LogEntry::new_op(
                    name,
                    "",
                    crate::sync::protocol::Operation::DeleteDatabase,
                    "",
                    None,
                ));
            }
            Response::ok_empty()
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_list_collections(handler: &DriverHandler, database: String) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => {
            // Credential collections are hidden here for the same reason the
            // HTTP listing hides them (SEC-176): a driver client cannot read
            // `_env`, `_admins` or `_api_keys` — every read path refuses the
            // name — so listing them only discloses that a database holds
            // provider keys. The two listings would otherwise disagree.
            let collections: Vec<String> = db
                .list_collections()
                .into_iter()
                .filter(|name| !crate::storage::is_protected_collection(name))
                .collect();
            Response::ok(serde_json::json!(collections))
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_create_collection(
    handler: &DriverHandler,
    database: String,
    name: String,
    collection_type: Option<String>,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.create_collection(name.clone(), collection_type.clone()) {
            Ok(_) => {
                if let Some(ref log) = handler.replication {
                    let metadata = serde_json::json!({
                        "type": collection_type.unwrap_or_else(|| "document".to_string()),
                        "shardConfig": None::<serde_json::Value>,
                    });
                    log.append(crate::sync::log::LogEntry::new_op(
                        database,
                        name,
                        crate::sync::protocol::Operation::CreateCollection,
                        "",
                        serde_json::to_vec(&metadata).ok(),
                    ));
                }
                Response::ok_empty()
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_delete_collection(
    handler: &DriverHandler,
    database: String,
    name: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.delete_collection(&name) {
            Ok(_) => {
                if let Some(ref log) = handler.replication {
                    log.append(crate::sync::log::LogEntry::new_op(
                        database,
                        name,
                        crate::sync::protocol::Operation::DeleteCollection,
                        "",
                        None,
                    ));
                }
                Response::ok_empty()
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_collection_stats(
    handler: &DriverHandler,
    database: String,
    name: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection(&name) {
            Ok(coll) => {
                let stats = coll.stats();
                Response::ok(serde_json::to_value(stats).unwrap_or_default())
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_truncate_collection(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.truncate() {
            Ok(_) => {
                // Same rule as the HTTP handler: replicas must replay the
                // truncate or they keep the old documents forever.
                if !crate::server::handlers::system::is_physical_shard_collection(&collection) {
                    if let Some(ref log) = handler.replication {
                        log.log_truncate(&database, &collection);
                    }
                }
                Response::ok_empty()
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(e),
    }
}

pub fn handle_compact_collection(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            coll.compact();
            Response::ok_empty()
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_recount_collection(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            let count = coll.recalculate_count();
            Response::ok_count(count)
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_prune_collection(
    handler: &DriverHandler,
    database: String,
    collection: String,
    older_than: Option<String>,
) -> Response {
    let Some(older_than) = older_than else {
        return Response::error(DriverError::InvalidCommand(
            "PruneCollection requires older_than (RFC 3339)".to_string(),
        ));
    };
    let ts = match chrono::DateTime::parse_from_rfc3339(&older_than) {
        Ok(dt) if dt.timestamp_millis() >= 0 => dt.timestamp_millis() as u64,
        _ => {
            return Response::error(DriverError::InvalidCommand(
                "older_than must be a non-negative RFC 3339 timestamp".to_string(),
            ))
        }
    };
    match handler.get_collection_for_write(&database, &collection) {
        Ok(coll) => match coll.prune_older_than(ts) {
            Ok(n) => {
                crate::storage::query_cache::get_query_cache().invalidate_collection(&collection);
                Response::ok_count(n)
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(e),
    }
}

pub async fn handle_repair_collection(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    let Some(coordinator) = handler.shard_coordinator.clone() else {
        return Response::error(DriverError::DatabaseError(
            "Shard coordinator not available".to_string(),
        ));
    };
    match coordinator.repair_collection(&database, &collection).await {
        Ok(report) => Response::ok(serde_json::json!({ "status": "repaired", "report": report })),
        Err(e) => Response::error(DriverError::DatabaseError(e)),
    }
}

pub fn handle_get_collection_sharding(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.get_shard_config() {
            Some(cfg) => Response::ok(serde_json::json!(cfg)),
            None => Response::ok(serde_json::Value::Null),
        },
        Err(e) => Response::error(e),
    }
}

/// Budget for an export reply's encoded documents, a little under the frame
/// cap so the response envelope still fits. A reply over the frame cap fails
/// to encode and drops the connection, so stop before building it.
const EXPORT_MAX_BYTES: usize = crate::driver::protocol::MAX_MESSAGE_SIZE - 64 * 1024;

pub async fn handle_export_collection(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    let coll = match handler.get_collection(&database, &collection) {
        Ok(coll) => coll,
        Err(e) => return Response::error(e),
    };
    // Audit P10: the scan used to run on the async worker and materialise the
    // whole collection (twice) before the frame cap rejected it.
    tokio::task::spawn_blocking(move || export_bounded(&coll, EXPORT_MAX_BYTES))
        .await
        .unwrap_or_else(|e| {
            Response::error(DriverError::DatabaseError(format!(
                "Task join error: {}",
                e
            )))
        })
}

/// Stream the collection's documents, giving up as soon as their encoded
/// size passes `max_bytes`.
fn export_bounded(coll: &crate::storage::Collection, max_bytes: usize) -> Response {
    use crate::storage::keyspace::KsDbExt;
    use rust_rocksdb::{Direction, IteratorMode};

    let Some(cf) = coll.ks.handle(&coll.db) else {
        return Response::ok(serde_json::json!([]));
    };
    let prefix = crate::storage::collection::DOC_PREFIX.as_bytes();
    let iter = coll
        .db
        .iterator_ks(&cf, IteratorMode::From(prefix, Direction::Forward));

    let mut docs = Vec::new();
    let mut bytes = 0usize;
    for (key, value) in iter.flatten() {
        if !key.starts_with(prefix) {
            break;
        }
        let Ok(doc) = crate::storage::serializer::deserialize_doc_as_value(&value) else {
            continue;
        };
        bytes += rmp_serde::to_vec_named(&doc).map(|v| v.len()).unwrap_or(0);
        if bytes > max_bytes {
            return Response::error(DriverError::DatabaseError(format!(
                "Collection too large to export in one reply (over {} bytes); \
                 use a paged List or an SDBQL query with LIMIT",
                max_bytes
            )));
        }
        docs.push(doc);
    }
    Response::ok(Value::Array(docs))
}

pub fn handle_import_collection(
    handler: &DriverHandler,
    database: String,
    collection: String,
    documents: Vec<serde_json::Value>,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.insert_batch(documents) {
            Ok(docs) => {
                handler.log_replication_batch(
                    &database,
                    &collection,
                    crate::sync::protocol::Operation::Insert,
                    &docs,
                );
                Response::ok_count(docs.len())
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(e),
    }
}

pub fn handle_set_collection_schema(
    handler: &DriverHandler,
    database: String,
    collection: String,
    schema: serde_json::Value,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match serde_json::from_value::<CollectionSchema>(schema) {
            Ok(s) => match coll.set_json_schema(s) {
                Ok(_) => Response::ok_empty(),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::InvalidCommand(format!(
                "Invalid schema: {}",
                e
            ))),
        },
        Err(e) => Response::error(e),
    }
}

pub fn handle_get_collection_schema(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.get_json_schema() {
            Some(schema) => Response::ok(serde_json::to_value(schema).unwrap_or_default()),
            None => Response::ok(serde_json::json!(null)),
        },
        Err(e) => Response::error(e),
    }
}

pub fn handle_delete_collection_schema(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.remove_json_schema() {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(e),
    }
}

// Columnar Operations
pub fn handle_create_columnar(
    handler: &DriverHandler,
    database: String,
    name: String,
    columns: Vec<Value>,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.create_columnar(name, columns) {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_list_columnar(handler: &DriverHandler, database: String) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => {
            let collections = db.list_columnar();
            Response::ok(serde_json::json!(collections))
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_get_columnar(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_columnar(&collection) {
            Ok(info) => Response::ok(serde_json::to_value(info).unwrap_or_default()),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_delete_columnar(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.delete_columnar(&collection) {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_insert_columnar(
    handler: &DriverHandler,
    database: String,
    collection: String,
    rows: Vec<serde_json::Value>,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.insert_columnar(&collection, rows) {
            Ok(count) => Response::ok_count(count),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_aggregate_columnar(
    handler: &DriverHandler,
    database: String,
    collection: String,
    aggregations: Vec<Value>,
    group_by: Option<Vec<String>>,
    filter: Option<String>,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.aggregate_columnar(&collection, aggregations, group_by, filter) {
            Ok(results) => Response::ok(serde_json::json!(results)),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_query_columnar(
    handler: &DriverHandler,
    database: String,
    collection: String,
    columns: Option<Vec<String>>,
    filter: Option<String>,
    order_by: Option<String>,
    limit: Option<i32>,
) -> Response {
    let limit_usize = limit.map(|l| l as usize);
    match handler.storage.get_database(&database) {
        Ok(db) => match db.query_columnar(&collection, columns, filter, order_by, limit_usize) {
            Ok(results) => Response::ok(serde_json::json!(results)),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_create_columnar_index(
    handler: &DriverHandler,
    database: String,
    collection: String,
    column: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.create_columnar_index(&collection, &column) {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_list_columnar_indexes(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.list_columnar_indexes(&collection) {
            Ok(indexes) => Response::ok(serde_json::json!(indexes)),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_delete_columnar_index(
    handler: &DriverHandler,
    database: String,
    collection: String,
    column: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.delete_columnar_index(&collection, &column) {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageEngine;
    use std::sync::Arc;

    fn handler() -> (tempfile::TempDir, DriverHandler) {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
        engine.create_database("d".to_string()).unwrap();
        engine
            .get_database("d")
            .unwrap()
            .create_collection("c".to_string(), None)
            .unwrap();
        (tmp, DriverHandler::new(engine, None))
    }

    #[test]
    fn prune_requires_a_valid_cutoff() {
        let (_t, h) = handler();
        for bad in [None, Some("yesterday".to_string())] {
            let r = handle_prune_collection(&h, "d".into(), "c".into(), bad);
            assert!(matches!(r, Response::Error { .. }), "{:?}", r);
        }
    }

    #[test]
    fn prune_deletes_only_older_documents() {
        let (_t, h) = handler();
        let coll = h.get_collection("d", "c").unwrap();
        // Auto-generated keys are UUIDv7, the only ones prune can date.
        coll.insert(serde_json::json!({"v": 1})).unwrap();
        // A cutoff in the past removes nothing; one in the future removes all.
        let r = handle_prune_collection(
            &h,
            "d".into(),
            "c".into(),
            Some("2000-01-01T00:00:00Z".into()),
        );
        assert!(matches!(r, Response::Ok { count: Some(0), .. }), "{:?}", r);
        let r = handle_prune_collection(
            &h,
            "d".into(),
            "c".into(),
            Some("2999-01-01T00:00:00Z".into()),
        );
        assert!(matches!(r, Response::Ok { count: Some(1), .. }), "{:?}", r);
    }

    #[tokio::test]
    async fn repair_needs_a_coordinator() {
        let (_t, h) = handler();
        let r = handle_repair_collection(&h, "d".into(), "c".into()).await;
        assert!(
            matches!(&r, Response::Error { error } if error.to_string().contains("coordinator")),
            "{:?}",
            r
        );
    }

    #[test]
    fn sharding_of_unsharded_collection_is_null() {
        let (_t, h) = handler();
        let r = handle_get_collection_sharding(&h, "d".into(), "c".into());
        assert!(
            matches!(&r, Response::Ok { data: Some(d), .. } if d.is_null())
                || matches!(&r, Response::Ok { data: None, .. }),
            "{:?}",
            r
        );
    }

    #[test]
    fn columnar_filter_order_and_aggregate() {
        let (_t, h) = handler();
        let db = h.storage.get_database("d").unwrap();
        db.create_columnar(
            "t".to_string(),
            vec![
                serde_json::json!({"name": "region", "data_type": "string", "nullable": false}),
                serde_json::json!({"name": "n", "data_type": "int64", "nullable": false}),
            ],
        )
        .unwrap();
        db.insert_columnar(
            "t",
            vec![
                serde_json::json!({"region": "eu", "n": 1}),
                serde_json::json!({"region": "eu", "n": 5}),
                serde_json::json!({"region": "us", "n": 10}),
            ],
        )
        .unwrap();

        let rows = db
            .query_columnar(
                "t",
                Some(vec!["region".into(), "n".into()]),
                Some(r#"{"column":"n","op":">=","value":5}"#.into()),
                Some("n DESC".into()),
                None,
            )
            .unwrap();
        let ns: Vec<_> = rows.iter().map(|r| r["n"].as_i64().unwrap()).collect();
        assert_eq!(ns, vec![10, 5]);

        let agg = |filter: Option<&str>, group: Option<Vec<String>>| {
            db.aggregate_columnar(
                "t",
                vec![serde_json::json!({"column": "n", "op": "SUM"})],
                group,
                filter.map(String::from),
            )
        };
        let eu = r#"{"and":[{"column":"region","op":"EQ","value":"eu"}]}"#;
        assert_eq!(agg(Some(eu), None).unwrap()[0]["n_sum"], 6.0);
        let grouped = agg(
            Some(r#"{"column":"n","op":"<","value":10}"#),
            Some(vec!["region".into()]),
        )
        .unwrap();
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0]["_agg"], 6.0);

        assert!(agg(Some("not json"), None).is_err());
        assert!(db
            .aggregate_columnar("t", vec![serde_json::json!({"column": "n"})], None, None)
            .is_err());
        assert!(db
            .query_columnar(
                "t",
                Some(vec!["n".into()]),
                None,
                Some("region".into()),
                None
            )
            .is_err());
    }
}
