//! Document commands of the native driver.
//!
//! None of these drops the shared query-result cache itself: every document
//! write does it in the collection, once committed (`Collection::emit_change`
//! and its batch form), whichever protocol it came over. These handlers used
//! to drop it a second time.

use crate::driver::protocol::{DriverError, Response};
use crate::driver::DriverHandler;
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
}
