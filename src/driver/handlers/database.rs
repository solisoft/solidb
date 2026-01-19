use crate::driver::protocol::{DriverError, Response};
use crate::driver::DriverHandler;
use crate::storage::CollectionSchema;
use serde_json::Value;

pub fn handle_list_databases(handler: &DriverHandler) -> Response {
    let dbs = handler.storage.list_databases();
    Response::ok(serde_json::json!(dbs))
}

pub fn handle_create_database(handler: &DriverHandler, name: String) -> Response {
    match handler.storage.create_database(name) {
        Ok(_) => Response::ok_empty(),
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_delete_database(handler: &DriverHandler, name: String) -> Response {
    match handler.storage.delete_database(&name) {
        Ok(_) => Response::ok_empty(),
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_list_collections(handler: &DriverHandler, database: String) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => {
            let collections = db.list_collections();
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
        Ok(db) => match db.create_collection(name, collection_type) {
            Ok(_) => Response::ok_empty(),
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
            Ok(_) => Response::ok_empty(),
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
            Ok(_) => Response::ok_empty(),
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

pub fn handle_export_collection(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            let docs: Vec<_> = coll.scan(None).into_iter().map(|d| d.to_value()).collect();
            Response::ok(serde_json::json!(docs))
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_import_collection(
    handler: &DriverHandler,
    database: String,
    collection: String,
    documents: Vec<serde_json::Value>,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.insert_batch(documents) {
            Ok(docs) => Response::ok_count(docs.len()),
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
