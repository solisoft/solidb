use super::DriverHandler;
use solidb_client::protocol::{DriverError, Response};

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
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            // If key provided, add it to document; otherwise insert() will auto-generate
            let mut doc_data = document;
            if let Some(k) = key {
                if let Some(obj) = doc_data.as_object_mut() {
                    obj.insert("_key".to_string(), serde_json::json!(k));
                }
            }
            match coll.insert(doc_data) {
                Ok(doc) => Response::ok(doc.to_value()),
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
    match handler.get_collection(&database, &collection) {
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
                Ok(doc) => Response::ok(doc.to_value()),
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
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.delete(&key) {
            Ok(_) => Response::ok_empty(),
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
            // Use scan() which is the correct method for listing documents
            let all_docs = coll.scan(None);
            let total = all_docs.len();

            // Apply pagination
            let offset = offset.unwrap_or(0);
            let limit = limit.unwrap_or(100);
            let docs: Vec<_> = all_docs
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|d| d.to_value())
                .collect();

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
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            // Use batch insert for efficiency
            match coll.insert_batch(documents) {
                Ok(docs) => Response::ok_count(docs.len()),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(e),
    }
}
