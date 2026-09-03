use crate::driver::protocol::{DriverError, Response};
use crate::driver::DriverHandler;
use crate::storage::query_cache;
use crate::sync::protocol::Operation;

/// Drop memoized read results for `collection`.
///
/// The HTTP document handlers do this on every insert/update/delete; without it
/// a write over the driver leaves the shared query-result cache serving stale
/// rows to both the driver and `/cursor` paths. The driver query handler
/// started using that cache, so missing invalidation here is a correctness bug
/// rather than a pure performance gap.
fn invalidate_query_cache(collection: &str) {
    query_cache::get_query_cache().invalidate_collection(collection);
}

pub fn handle_get(
    handler: &DriverHandler,
    database: String,
    collection: String,
    key: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.get(&key) {
            Ok(doc) => Response::ok(doc.to_value()),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
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
            match coll.insert(doc_data) {
                Ok(doc) => {
                    let value = doc.to_value();
                    handler.log_replication(
                        &database,
                        &collection,
                        Operation::Insert,
                        &doc.key,
                        Some(&value),
                    );
                    invalidate_query_cache(&collection);
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
                // Merge update: get existing doc and merge
                match coll.get(&key) {
                    Ok(existing) => {
                        let mut merged = existing.data.clone();
                        if let (Some(base), Some(updates)) =
                            (merged.as_object_mut(), document.as_object())
                        {
                            for (k, v) in updates {
                                base.insert(k.clone(), v.clone());
                            }
                        }
                        coll.update(&key, merged)
                    }
                    Err(e) => Err(e),
                }
            } else {
                coll.update(&key, document)
            };

            match result {
                Ok(doc) => {
                    let value = doc.to_value();
                    handler.log_replication(
                        &database,
                        &collection,
                        Operation::Update,
                        &doc.key,
                        Some(&value),
                    );
                    invalidate_query_cache(&collection);
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
                invalidate_query_cache(&collection);
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
                    invalidate_query_cache(&collection);
                    Response::ok_count(docs.len())
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(e),
    }
}
