use axum::{
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

use crate::storage::StorageEngine;
use super::handlers::*;

pub fn create_router(storage: StorageEngine) -> Router {
    let state = Arc::new(storage);

    Router::new()
        // Collection routes
        .route("/_api/collection", post(create_collection))
        .route("/_api/collection", get(list_collections))
        .route("/_api/collection/:name", delete(delete_collection))

        // Document routes
        .route("/_api/document/:collection", post(insert_document))
        .route("/_api/document/:collection/:key", get(get_document))
        .route("/_api/document/:collection/:key", put(update_document))
        .route("/_api/document/:collection/:key", delete(delete_document))

        // Query routes
        .route("/_api/cursor", post(execute_query))
        .route("/_api/explain", post(explain_query))

        // Index routes
        .route("/_api/index/:collection", post(create_index))
        .route("/_api/index/:collection", get(list_indexes))
        .route("/_api/index/:collection/:name", delete(delete_index))

        // Geo index routes
        .route("/_api/geo/:collection", post(create_geo_index))
        .route("/_api/geo/:collection", get(list_geo_indexes))
        .route("/_api/geo/:collection/:name", delete(delete_geo_index))
        .route("/_api/geo/:collection/:field/near", post(geo_near))
        .route("/_api/geo/:collection/:field/within", post(geo_within))

        // Add state and middleware
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}
