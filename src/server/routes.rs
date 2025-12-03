use axum::{
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;
use std::time::Duration;
use tower_http::trace::TraceLayer;

use crate::storage::StorageEngine;
use crate::server::cursor_store::CursorStore;
use super::handlers::*;

pub fn create_router(storage: StorageEngine) -> Router {
    let state = AppState {
        storage: Arc::new(storage),
        cursor_store: CursorStore::new(Duration::from_secs(300)),
    };

    Router::new()
        // Database routes
        .route("/_api/database", post(create_database))
        .route("/_api/databases", get(list_databases))
        .route("/_api/database/:name", delete(delete_database))

        // Collection routes
        .route("/_api/database/:db/collection", post(create_collection))
        .route("/_api/database/:db/collection", get(list_collections))
        .route("/_api/database/:db/collection/:name", delete(delete_collection))
        .route("/_api/database/:db/collection/:name/truncate", put(truncate_collection))

        // Document routes
        .route("/_api/database/:db/document/:collection", post(insert_document))
        .route("/_api/database/:db/document/:collection/:key", get(get_document))
        .route("/_api/database/:db/document/:collection/:key", put(update_document))
        .route("/_api/database/:db/document/:collection/:key", delete(delete_document))

        // Query routes
        .route("/_api/database/:db/cursor", post(execute_query))
        .route("/_api/cursor/:id", put(get_next_batch))
        .route("/_api/cursor/:id", delete(delete_cursor))
        .route("/_api/database/:db/explain", post(explain_query))

        // Index routes
        .route("/_api/database/:db/index/:collection", post(create_index))
        .route("/_api/database/:db/index/:collection", get(list_indexes))
        .route("/_api/database/:db/index/:collection/:name", delete(delete_index))

        // Geo index routes
        .route("/_api/database/:db/geo/:collection", post(create_geo_index))
        .route("/_api/database/:db/geo/:collection", get(list_geo_indexes))
        .route("/_api/database/:db/geo/:collection/:name", delete(delete_geo_index))
        .route("/_api/database/:db/geo/:collection/:field/near", post(geo_near))
        .route("/_api/database/:db/geo/:collection/:field/within", post(geo_within))

        .with_state(state)
        .layer(TraceLayer::new_for_http())
}
