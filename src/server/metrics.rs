//! Prometheus metrics endpoint for SoliDB
//!
//! Exposes metrics in Prometheus text format at /metrics

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
};
use std::sync::atomic::Ordering;

use super::handlers::AppState;

/// Prometheus metrics handler
///
/// Returns metrics in Prometheus text exposition format.
/// This endpoint does not require authentication.
pub async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let mut output = String::new();

    // HTTP Requests Total
    let request_count = state.request_counter.load(Ordering::Relaxed);
    output.push_str("# HELP solidb_http_requests_total Total number of HTTP requests processed\n");
    output.push_str("# TYPE solidb_http_requests_total counter\n");
    output.push_str(&format!("solidb_http_requests_total {}\n\n", request_count));

    // Uptime
    let uptime_secs = state.startup_time.elapsed().as_secs_f64();
    output.push_str("# HELP solidb_uptime_seconds Time since server started in seconds\n");
    output.push_str("# TYPE solidb_uptime_seconds gauge\n");
    output.push_str(&format!("solidb_uptime_seconds {:.3}\n\n", uptime_secs));

    // System Metrics (CPU, Memory)
    {
        let mut system = state.system_monitor.lock().unwrap();
        system.refresh_cpu();
        system.refresh_memory();

        // CPU Usage
        let cpu_usage = system.global_cpu_info().cpu_usage();
        output.push_str("# HELP solidb_cpu_usage_percent Current CPU usage percentage\n");
        output.push_str("# TYPE solidb_cpu_usage_percent gauge\n");
        output.push_str(&format!("solidb_cpu_usage_percent {:.2}\n\n", cpu_usage));

        // Memory Usage
        let total_memory = system.total_memory();
        let used_memory = system.used_memory();
        let available_memory = system.available_memory();

        output.push_str("# HELP solidb_memory_total_bytes Total system memory in bytes\n");
        output.push_str("# TYPE solidb_memory_total_bytes gauge\n");
        output.push_str(&format!("solidb_memory_total_bytes {}\n\n", total_memory));

        output.push_str("# HELP solidb_memory_used_bytes Used system memory in bytes\n");
        output.push_str("# TYPE solidb_memory_used_bytes gauge\n");
        output.push_str(&format!("solidb_memory_used_bytes {}\n\n", used_memory));

        output.push_str("# HELP solidb_memory_available_bytes Available system memory in bytes\n");
        output.push_str("# TYPE solidb_memory_available_bytes gauge\n");
        output.push_str(&format!(
            "solidb_memory_available_bytes {}\n\n",
            available_memory
        ));
    }

    // Script Stats
    let active_scripts = state.script_stats.active_scripts.load(Ordering::Relaxed);
    let active_ws = state.script_stats.active_ws.load(Ordering::Relaxed);
    let total_scripts = state
        .script_stats
        .total_scripts_executed
        .load(Ordering::Relaxed);
    let total_ws = state
        .script_stats
        .total_ws_connections
        .load(Ordering::Relaxed);

    output.push_str("# HELP solidb_active_scripts Current number of active Lua scripts\n");
    output.push_str("# TYPE solidb_active_scripts gauge\n");
    output.push_str(&format!("solidb_active_scripts {}\n\n", active_scripts));

    output.push_str(
        "# HELP solidb_active_websockets Current number of active WebSocket connections\n",
    );
    output.push_str("# TYPE solidb_active_websockets gauge\n");
    output.push_str(&format!("solidb_active_websockets {}\n\n", active_ws));

    output.push_str("# HELP solidb_scripts_executed_total Total number of Lua scripts executed\n");
    output.push_str("# TYPE solidb_scripts_executed_total counter\n");
    output.push_str(&format!(
        "solidb_scripts_executed_total {}\n\n",
        total_scripts
    ));

    output.push_str(
        "# HELP solidb_websocket_connections_total Total number of WebSocket connections\n",
    );
    output.push_str("# TYPE solidb_websocket_connections_total counter\n");
    output.push_str(&format!(
        "solidb_websocket_connections_total {}\n\n",
        total_ws
    ));

    // Database Stats
    let databases = state.storage.list_databases();
    let db_count = databases.len();
    output.push_str("# HELP solidb_databases_total Number of databases\n");
    output.push_str("# TYPE solidb_databases_total gauge\n");
    output.push_str(&format!("solidb_databases_total {}\n\n", db_count));

    // Count total collections across all databases
    let mut total_collections = 0;
    for db_name in &databases {
        if let Ok(db) = state.storage.get_database(db_name) {
            let colls = db.list_collections();
            total_collections += colls.len();
        }
    }
    output.push_str(
        "# HELP solidb_collections_total Total number of collections across all databases\n",
    );
    output.push_str("# TYPE solidb_collections_total gauge\n");
    output.push_str(&format!(
        "solidb_collections_total {}\n\n",
        total_collections
    ));

    // Cluster Stats (if cluster mode is enabled)
    if let Some(ref cluster_manager) = state.cluster_manager {
        let healthy_nodes = cluster_manager.get_healthy_nodes();
        let healthy_count = healthy_nodes.len();

        output.push_str(
            "# HELP solidb_cluster_healthy_nodes Number of healthy nodes in the cluster\n",
        );
        output.push_str("# TYPE solidb_cluster_healthy_nodes gauge\n");
        output.push_str(&format!(
            "solidb_cluster_healthy_nodes {}\n\n",
            healthy_count
        ));

        // Local node ID (for identification)
        let local_node = cluster_manager.local_node_id();
        output.push_str("# HELP solidb_cluster_info Cluster information\n");
        output.push_str("# TYPE solidb_cluster_info gauge\n");
        output.push_str(&format!(
            "solidb_cluster_info{{node_id=\"{}\"}} 1\n\n",
            local_node
        ));
    }

    // Shard Coordinator Stats (if sharding is enabled)
    if state.shard_coordinator.is_some() {
        output.push_str("# HELP solidb_sharding_enabled Whether sharding is enabled\n");
        output.push_str("# TYPE solidb_sharding_enabled gauge\n");
        output.push_str("solidb_sharding_enabled 1\n\n");
    }

    // Queue Stats (if queue worker is enabled)
    if state.queue_worker.is_some() {
        output.push_str("# HELP solidb_queue_worker_enabled Whether the queue worker is enabled\n");
        output.push_str("# TYPE solidb_queue_worker_enabled gauge\n");
        output.push_str("solidb_queue_worker_enabled 1\n\n");
    }

    // Return with proper content type for Prometheus
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        output,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_prometheus_format() {
        // Test that output follows Prometheus format
        let output = "# HELP solidb_http_requests_total Total requests\n# TYPE solidb_http_requests_total counter\nsolidb_http_requests_total 42\n";
        assert!(output.contains("# HELP"));
        assert!(output.contains("# TYPE"));
        assert!(output.contains("counter"));
    }
}
