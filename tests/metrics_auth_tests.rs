//! `/metrics` authentication.
//!
//! The documented way to scrape is `Authorization: Bearer $SOLIDB_METRICS_TOKEN`
//! (getting-started, the Monitoring page's Prometheus example). It answered
//! 401 from 1.0 to 2.2: the route sat under `permissive_auth_middleware`,
//! which takes any `Bearer` value for a JWT and rejects the request before the
//! handler, which knows about the metrics token, ever runs.
//!
//! One test, because `SOLIDB_METRICS_TOKEN` is process-wide: tests that set it
//! differently in parallel would race.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

async fn scrape(app: &axum::Router, header: Option<(&str, &str)>) -> (StatusCode, String) {
    let mut request = Request::builder().method("GET").uri("/metrics");
    if let Some((name, value)) = header {
        request = request.header(name, value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn metrics_token_is_accepted_in_either_documented_header() {
    std::env::set_var("SOLIDB_METRICS_TOKEN", "scrape-token");
    std::env::remove_var("SOLIDB_METRICS_PUBLIC");

    let tmp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    let app = create_router(
        engine.clone(),
        None,
        None,
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );

    let (status, body) = scrape(&app, Some(("Authorization", "Bearer scrape-token"))).await;
    assert_eq!(status, StatusCode::OK, "Bearer metrics token refused");
    assert!(body.contains("# TYPE solidb_query_duration_seconds histogram"));

    let (status, _) = scrape(&app, Some(("X-Metrics-Token", "scrape-token"))).await;
    assert_eq!(status, StatusCode::OK, "X-Metrics-Token refused");

    let (status, _) = scrape(&app, Some(("Authorization", "Bearer wrong-token"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a wrong token got in");

    let (status, _) = scrape(&app, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "no credentials got in");
}
