use super::DriverHandler;
use crate::driver::protocol::{DriverError, Response};
use crate::storage::{VectorIndexConfig, VectorMetric};

// ==================== Standard Index Operations ====================

pub fn handle_create_index(
    handler: &DriverHandler,
    database: String,
    collection: String,
    name: String,
    fields: Vec<String>,
    unique: bool,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            // Default to Persistent index type
            let index_type = crate::storage::IndexType::Persistent;
            match coll.create_index(name, fields, index_type, unique) {
                Ok(stats) => Response::ok(serde_json::to_value(stats).unwrap_or_default()),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_delete_index(
    handler: &DriverHandler,
    database: String,
    collection: String,
    name: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            // Try dropping as standard index first
            if coll.drop_index(&name).is_ok() {
                return Response::ok_empty();
            }
            // Try dropping as fulltext index
            if coll.drop_fulltext_index(&name).is_ok() {
                return Response::ok_empty();
            }
            // Try dropping as geo index
            if coll.drop_geo_index(&name).is_ok() {
                return Response::ok_empty();
            }
            // Try dropping as TTL index
            if coll.drop_ttl_index(&name).is_ok() {
                return Response::ok_empty();
            }
            Response::error(DriverError::DatabaseError(format!(
                "Index '{}' not found",
                name
            )))
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_list_indexes(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            let indexes = coll.list_indexes();
            Response::ok(serde_json::to_value(indexes).unwrap_or_default())
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_rebuild_indexes(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.rebuild_all_indexes() {
            Ok(stats) => Response::ok(serde_json::to_value(stats).unwrap_or_default()),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(e),
    }
}

// ==================== Geo Index Operations ====================

pub fn handle_create_geo_index(
    handler: &DriverHandler,
    database: String,
    collection: String,
    name: String,
    field: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.create_geo_index(name, field) {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(e),
    }
}

pub fn handle_list_geo_indexes(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            let indexes = coll.list_geo_indexes();
            Response::ok(serde_json::to_value(indexes).unwrap_or_default())
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_delete_geo_index(
    handler: &DriverHandler,
    database: String,
    collection: String,
    name: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.drop_geo_index(&name) {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(e),
    }
}

pub fn handle_geo_near(
    handler: &DriverHandler,
    database: String,
    collection: String,
    field: String,
    latitude: f64,
    longitude: f64,
    radius: Option<f64>,
    limit: Option<i32>,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            let limit_val = limit.map(|l| l.max(0) as usize).unwrap_or(10);

            let results_opt = if let Some(r) = radius {
                coll.geo_within(&field, latitude, longitude, r)
                    .map(|mut res| {
                        if limit_val < res.len() {
                            res.truncate(limit_val);
                        }
                        res
                    })
            } else {
                coll.geo_near(&field, latitude, longitude, limit_val)
            };

            match results_opt {
                Some(results) => Response::ok(serde_json::json!(results)),
                None => Response::error(DriverError::DatabaseError(
                    "Geo index not found".to_string(),
                )),
            }
        }
        Err(e) => Response::error(e),
    }
}

// ==================== Vector Index Operations ====================

pub fn handle_create_vector_index(
    handler: &DriverHandler,
    database: String,
    collection: String,
    name: String,
    field: String,
    dimensions: i32,
    metric: Option<String>,
    ef_construction: Option<i32>,
    m: Option<i32>,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            let mut config = VectorIndexConfig::new(name, field, dimensions as usize);

            if let Some(m_str) = metric {
                if let Ok(val) =
                    serde_json::from_value::<VectorMetric>(serde_json::Value::String(m_str))
                {
                    config = config.with_metric(val);
                }
            }

            if let Some(ef) = ef_construction {
                config = config.with_ef_construction(ef as usize);
            }

            if let Some(m_val) = m {
                config = config.with_m(m_val as usize);
            }

            match coll.create_vector_index(config) {
                Ok(_) => Response::ok_empty(),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_list_vector_indexes(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            let indexes = coll.list_vector_indexes();
            Response::ok(serde_json::to_value(indexes).unwrap_or_default())
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_delete_vector_index(
    handler: &DriverHandler,
    database: String,
    collection: String,
    name: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.drop_vector_index(&name) {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(e),
    }
}

pub fn handle_vector_search(
    handler: &DriverHandler,
    database: String,
    collection: String,
    index_name: String,
    vector: Vec<f32>,
    limit: Option<i32>,
    ef_search: Option<i32>,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            // TODO: Implement filter support when Collection::vector_search supports it
            match coll.vector_search(
                &index_name,
                &vector,
                limit.map(|l| l.max(0) as usize).unwrap_or(10),
                ef_search.map(|v| v.max(0) as usize),
            ) {
                Ok(results) => Response::ok(serde_json::json!(results)),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_quantize_vector_index(
    handler: &DriverHandler,
    database: String,
    collection: String,
    index_name: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.quantize_vector_index(&index_name, crate::storage::index::VectorQuantization::Scalar) {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(e),
    }
}

pub fn handle_dequantize_vector_index(
    handler: &DriverHandler,
    database: String,
    collection: String,
    index_name: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.dequantize_vector_index(&index_name) {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(e),
    }
}

// ==================== TTL Index Operations ====================

pub fn handle_create_ttl_index(
    handler: &DriverHandler,
    database: String,
    collection: String,
    name: String,
    field: String,
    expire_after_seconds: i64,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.create_ttl_index(name, field, expire_after_seconds as u64) {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(e),
    }
}

pub fn handle_list_ttl_indexes(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => {
            let indexes = coll.list_ttl_indexes();
            Response::ok(serde_json::to_value(indexes).unwrap_or_default())
        }
        Err(e) => Response::error(e),
    }
}

pub fn handle_delete_ttl_index(
    handler: &DriverHandler,
    database: String,
    collection: String,
    name: String,
) -> Response {
    match handler.get_collection(&database, &collection) {
        Ok(coll) => match coll.drop_ttl_index(&name) {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(e),
    }
}
