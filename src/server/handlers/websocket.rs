use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, Query as AxumQuery, State},
    response::{IntoResponse, Response},
    http::HeaderMap,
    http::StatusCode,
    body::Body,
};
use serde::{Deserialize};
use crate::{
    error::DbError,
    server::handlers::auth::AuthParams,
    storage::StorageEngine,
};
use super::system::AppState;
use super::cluster::generate_cluster_status;
use std::sync::Arc;
use futures::{SinkExt, StreamExt};

// ==================== Cluster Status WebSocket ====================

/// WebSocket handler for real-time cluster status updates
pub async fn cluster_status_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_cluster_ws(socket, state))
}

/// Handle the WebSocket connection for cluster status
async fn handle_cluster_ws(mut socket: WebSocket, state: AppState) {
    use tokio::time::{interval, Duration};

    let mut ticker = interval(Duration::from_secs(1));

    // We use the shared system monitor from AppState to avoid expensive initialization
    // and to ensure CPU usage is calculated correctly (delta since last refresh).

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Generate status using shared logic and persistent sys
                let status = {
                    let mut sys = state.system_monitor.lock().unwrap();
                    generate_cluster_status(&state, &mut *sys)
                };

                let json = match serde_json::to_string(&status) {
                    Ok(j) => j,
                    Err(_) => continue,
                };

                if socket.send(Message::Text(json.into())).await.is_err() {
                    break; // Client disconnected
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        // Respond to ping with pong
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    _ => {} // Ignore other messages
                }
            }
        }
    }
}

// ==================== System Monitoring WebSocket ====================

pub async fn monitor_ws_handler(
    ws: WebSocketUpgrade,
    AxumQuery(params): AxumQuery<AuthParams>,
    State(state): State<AppState>,
) -> Response {
    if let Err(_) = crate::server::auth::AuthService::validate_token(&params.token) {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::empty())
            .expect("Valid status code should not fail")
            .into_response();
    }

    ws.on_upgrade(|socket| handle_monitor_socket(socket, state))
}

async fn handle_monitor_socket(mut socket: WebSocket, state: AppState) {
    use std::sync::atomic::Ordering;

    tracing::info!("Monitor WS: Client connected");

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));

    loop {
        // Wait for next tick
        interval.tick().await;

        let stats = {
            let mut sys = state.system_monitor.lock().unwrap();

            // Refresh specific stats
            sys.refresh_cpu();
            sys.refresh_memory();

            let cpu = sys.global_cpu_info().cpu_usage();
            let mem_used = sys.used_memory();
            let mem_total = sys.total_memory();
            let up = sysinfo::System::uptime();
            let name = sysinfo::System::name().unwrap_or_else(|| "Unknown".to_string());
            let version =
                sysinfo::System::kernel_version().unwrap_or_else(|| "Unknown".to_string());
            let host = sysinfo::System::host_name().unwrap_or_else(|| "Unknown".to_string());
            let cores = sys.cpus().len();

            serde_json::json!({
                "cpu_usage": cpu,
                "memory_usage": mem_used,
                "memory_total": mem_total,
                "uptime": up,
                "os_name": name,
                "os_version": version,
                "hostname": host,
                "num_cpus": cores,
                "pid": std::process::id(),
                "active_scripts": state.script_stats.active_scripts.load(Ordering::Relaxed),
                "active_ws": state.script_stats.active_ws.load(Ordering::Relaxed)
            })
        };

        let msg = match serde_json::to_string(&stats) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if socket.send(Message::Text(msg.into())).await.is_err() {
            // Client disconnected
            break;
        }
    }
}

// ==================== Real-time Changefeeds ====================

#[derive(Debug, Deserialize)]
pub struct ChangefeedRequest {
    #[serde(rename = "type")]
    pub type_: String,
    pub collection: Option<String>,
    pub database: Option<String>,
    pub key: Option<String>,
    pub local: Option<bool>,
    /// SDBQL query for live_query mode
    pub query: Option<String>,
    /// Optional Client ID to identify the subscription/query in responses
    pub id: Option<String>,
}

/// WebSocket handler for real-time changefeeds
pub async fn ws_changefeed_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    AxumQuery(params): AxumQuery<AuthParams>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // Check for cluster-internal authentication (bypasses normal JWT validation)
    let is_cluster_internal = {
        let cluster_secret = state.cluster_secret();
        let provided_secret = headers
            .get("X-Cluster-Secret")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");

        // Use constant-time comparison to prevent timing attacks
        !cluster_secret.is_empty()
            && crate::server::auth::constant_time_eq(
                cluster_secret.as_bytes(),
                provided_secret.as_bytes(),
            )
    };

    // If not cluster-internal, validate the JWT token
    if !is_cluster_internal {
        if let Err(_) = crate::server::auth::AuthService::validate_token(&params.token) {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .expect("Valid status code should not fail")
                .into_response();
        }
    }

    // Check if HTMX mode is requested
    let use_htmx = params.htmx.map(|s| s == "true").unwrap_or(false);

    ws.on_upgrade(move |socket| handle_socket(socket, state, use_htmx))
}

async fn handle_socket(socket: WebSocket, state: AppState, use_htmx: bool) {
    // Split socket into sender and receiver
    let (mut sender, mut receiver) = socket.split();

    // Unified channel for sending messages to the client
    // All subscription tasks and live queries will send ready-to-emit Messages to this channel
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Message>(1000);

    // Spawn writer task that forwards messages from the channel to the WebSocket
    let send_task = tokio::spawn(async move {
        // Heartbeat: Send a Ping every 30 seconds to keep the connection alive
        let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(30));

        loop {
            tokio::select! {
                // Send heartbeat
                _ = heartbeat_interval.tick() => {
                    if sender.send(Message::Ping(vec![].into())).await.is_err() {
                        tracing::debug!("[WS] Failed to send ping, closing writer");
                        break;
                    }
                }
                // Forward messages
                Some(msg) = rx.recv() => {
                    if sender.send(msg).await.is_err() {
                        tracing::debug!("[WS] Failed to send message, closing writer");
                        break;
                    }
                }
                else => break,
            }
        }
    });

    // Main Receiver Loop
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                let req_result = serde_json::from_str::<ChangefeedRequest>(&text);
                match req_result {
                    Ok(req) if req.type_ == "subscribe" => {
                        let tx_clone = tx.clone();
                        let state_clone = state.clone();

                        // Spawn a dedicated task for this subscription
                        tokio::spawn(async move {
                            handle_subscribe_request(req, state_clone, tx_clone, use_htmx).await;
                        });
                    }
                    Ok(req) if req.type_ == "live_query" => {
                        let tx_clone = tx.clone();
                        let state_clone = state.clone();

                        // Spawn a dedicated task for this live query
                        tokio::spawn(async move {
                            handle_live_query_request(req, state_clone, tx_clone).await;
                        });
                    }
                    _ => {
                        let _ = tx
                            .send(Message::Text(
                                serde_json::json!({
                                    "error": "Invalid subscription request or unknown type"
                                })
                                .to_string()
                                .into(),
                            ))
                            .await;
                    }
                }
            }
            Message::Close(_) => break,
            Message::Ping(_) => {
                // Auto-replied with Pong by axum usually, but we can ignore
            }
            Message::Pong(_) => {
                // Heartbeat response, ignore
            }
            _ => {}
        }
    }

    // When log out, abort the sender task
    send_task.abort();
}

/// Handle a single subscription request
async fn handle_subscribe_request(
    req: ChangefeedRequest,
    state: AppState,
    tx: tokio::sync::mpsc::Sender<Message>,
    use_htmx: bool,
) {
    let db_name = req.database.clone().unwrap_or("_system".to_string());

    let coll_name = match req.collection.clone() {
        Some(c) => c,
        None => {
            // Try to infer from SDBQL query
            if let Some(query_str) = &req.query {
                if let Ok(query_ast) = crate::sdbql::parser::parse(query_str) {
                    // Check explicit FOR clauses first
                    if let Some(first_for) = query_ast.for_clauses.first() {
                        first_for.collection.clone()
                    } else {
                        // Check body clauses
                        query_ast
                            .body_clauses
                            .iter()
                            .find_map(|c| {
                                if let crate::sdbql::ast::BodyClause::For(f) = c {
                                    Some(f.collection.clone())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_default()
                    }
                } else {
                    "".to_string()
                }
            } else {
                "".to_string()
            }
        }
    };

    if coll_name.is_empty() {
        let _ = tx
            .send(Message::Text(
                serde_json::json!({
                    "error": "Collection required for subscribe mode (could not infer from query)"
                })
                .to_string()
                .into(),
            ))
            .await;
        return;
    }

    // Try to get collection from specific database or fallback
    let collection_result = state
        .storage
        .get_database(&db_name)
        .and_then(|db| db.get_collection(&coll_name));

    match collection_result {
        Ok(collection) => {
            // Send confirmation
            let msg = if use_htmx {
                format!(
                    r#"<div id="connection-status" hx-swap-oob="innerHTML" class="inline-flex items-center gap-2 px-3 py-1.5 rounded-full text-sm bg-success/10 text-success">
                    <span class="w-2 h-2 rounded-full bg-success animate-pulse"></span>
                    <span>Connected: {}</span>
                </div>
                <div id="no-subscriptions" hx-swap-oob="true" class="hidden"></div>
                <div id="subscriptions-list" hx-swap-oob="beforeend">
                    <div class="px-4 py-3 border-b border-border/20 last:border-0 flex items-center justify-between">
                        <div class="flex items-center gap-3">
                        <span class="w-2 h-2 rounded-full bg-success animate-pulse"></span>
                        <div>
                            <span class="font-medium text-text">{}</span>
                        </div>
                        </div>
                    </div>
                </div>"#,
                    coll_name, coll_name
                )
            } else {
                serde_json::json!({
                    "type": "subscribed",
                    "collection": coll_name
                })
                .to_string()
            };
            if tx.send(Message::Text(msg.into())).await.is_err() {
                return;
            }

            // Set up our OWN internal channel to aggregate events for THIS subscription
            // Then we format them and send to the main `tx`
            let (sub_tx, mut sub_rx) =
                tokio::sync::mpsc::channel::<crate::storage::collection::ChangeEvent>(1000);
            let req_key = req.key.clone();

            // 1. Subscribe to local logical collection
            let mut local_rx = collection.change_sender.subscribe();
            let sub_tx_local = sub_tx.clone();
            let req_key_local = req_key.clone();

            tokio::spawn(async move {
                loop {
                    match local_rx.recv().await {
                        Ok(event) => {
                            if let Some(ref target_key) = req_key_local {
                                if &event.key != target_key {
                                    continue;
                                }
                            }
                            if sub_tx_local.send(event).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });

            // 2. Subscribe to PHYSICAL SHARDS (if sharded)
            if let Some(shard_config) = collection.get_shard_config() {
                if shard_config.num_shards > 0 {
                    if let Ok(database) = state.storage.get_database(&db_name) {
                        for shard_id in 0..shard_config.num_shards {
                            let physical_name = format!("{}_s{}", coll_name, shard_id);
                            if let Ok(physical_coll) = database.get_collection(&physical_name) {
                                let mut shard_rx = physical_coll.change_sender.subscribe();
                                let sub_tx_shard = sub_tx.clone();
                                let req_key_shard = req_key.clone();

                                tokio::spawn(async move {
                                    loop {
                                        match shard_rx.recv().await {
                                            Ok(event) => {
                                                if let Some(ref target_key) = req_key_shard {
                                                    if &event.key != target_key {
                                                        continue;
                                                    }
                                                }
                                                if sub_tx_shard.send(event).await.is_err() {
                                                    break;
                                                }
                                            }
                                            Err(
                                                tokio::sync::broadcast::error::RecvError::Lagged(_),
                                            ) => continue,
                                            Err(_) => break,
                                        }
                                    }
                                });
                            }
                        }
                    }
                }
            }

            // 3. Connect to REMOTE nodes
            let is_local_only = req.local.unwrap_or(false);

            if !is_local_only {
                if let Some(shard_config) = collection.get_shard_config() {
                    if let Some(coordinator) = &state.shard_coordinator {
                        let my_addr = coordinator.my_address();
                        let all_nodes = coordinator.get_collection_nodes(&shard_config);
                        let cluster_secret = state.cluster_secret();

                        let mut remote_nodes = std::collections::HashSet::new();
                        for node_addr in all_nodes {
                            if node_addr != my_addr {
                                remote_nodes.insert(node_addr);
                            }
                        }

                        for node_addr in remote_nodes {
                            let sub_tx_remote = sub_tx.clone();
                            let db_name_remote = db_name.clone();
                            let coll_name_remote = coll_name.clone();
                            let node_addr_clone = node_addr.clone();
                            let secret_clone = cluster_secret.clone();

                            tokio::spawn(async move {
                                use crate::cluster::ClusterWebsocketClient;
                                match ClusterWebsocketClient::connect(
                                    &node_addr_clone,
                                    &db_name_remote,
                                    &coll_name_remote,
                                    true,
                                    &secret_clone,
                                )
                                .await
                                {
                                    Ok(stream) => {
                                        tokio::pin!(stream);
                                        while let Some(result) = stream.next().await {
                                            match result {
                                                Ok(event) => {
                                                    if sub_tx_remote.send(event).await.is_err() {
                                                        break;
                                                    }
                                                }
                                                Err(_) => break,
                                            }
                                        }
                                    }
                                    Err(_) => {}
                                }
                            });
                        }
                    }
                }
            }

            // Drop original sub_tx so we don't hold the channel open forever if all producers die
            drop(sub_tx);

            // Forward aggregated events to the main socket channel
            loop {
                match sub_rx.recv().await {
                    Some(event) => {
                        // Double check filter (especially for remote events)
                        if let Some(ref target_key) = req.key {
                            if &event.key != target_key {
                                continue;
                            }
                        }

                        // Format message
                        let msg_text = if use_htmx {
                            use crate::storage::collection::ChangeType;
                            let op_type = match event.type_ {
                                ChangeType::Insert => "INSERT",
                                ChangeType::Update => "UPDATE",
                                ChangeType::Delete => "DELETE",
                            };
                            let status_class = match event.type_ {
                                ChangeType::Insert => "bg-success/10 text-success",
                                ChangeType::Update => "bg-warning/10 text-warning",
                                ChangeType::Delete => "bg-error/10 text-error",
                            };
                            let data_str = event
                                .data
                                .as_ref()
                                .map(|v| v.to_string())
                                .unwrap_or_default();

                            format!(
                                r#"<div hx-swap-oob="afterbegin:#events-container">
                                <div class="px-4 py-2 border-b border-border/10 last:border-0 font-mono text-sm hover:bg-white/5 transition-colors">
                                    <div class="flex items-center gap-2 mb-1">
                                        <span class="px-1.5 py-0.5 rounded text-xs {}">{}</span>
                                        <span class="text-text-dim text-xs">{}</span>
                                        <span class="text-text-dim text-xs ml-auto">{}</span>
                                    </div>
                                    <pre class="text-text-muted text-xs overflow-x-auto">{}</pre>
                                </div>
                            </div>"#,
                                status_class,
                                op_type,
                                coll_name,
                                chrono::Local::now().format("%H:%M:%S"),
                                data_str
                            )
                        } else {
                            serde_json::json!({
                                "operation": event.type_,
                                "collection": coll_name,
                                "key": event.key,
                                "data": event.data
                            })
                            .to_string()
                        };

                        if tx.send(Message::Text(msg_text.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break, // All producers gone
                }
            }
        }
        Err(_) => {
            let _ = tx
                .send(Message::Text(
                    serde_json::json!({
                        "error": format!("Collection '{}' not found", coll_name)
                    })
                    .to_string()
                    .into(),
                ))
                .await;
        }
    }
}

/// Handle a live query request
async fn handle_live_query_request(
    req: ChangefeedRequest,
    state: AppState,
    tx: tokio::sync::mpsc::Sender<Message>,
) {
    if let Some(query_str) = req.query {
        let db_name = req.database.clone().unwrap_or("_system".to_string());

        // 1. Parse query to identify dependencies
        match crate::sdbql::parser::parse(&query_str) {
            Ok(query) => {
                let mut dependencies = std::collections::HashSet::new();
                for clause in &query.for_clauses {
                    dependencies.insert(clause.collection.clone());
                }

                if dependencies.is_empty() {
                    let _ = tx
                        .send(Message::Text(
                            serde_json::json!({
                                "error": "Live query must reference at least one collection"
                            })
                            .to_string()
                            .into(),
                        ))
                        .await;
                    return;
                }

                // Send confirmation
                let mut response = serde_json::json!({
                    "type": "subscribed",
                    "mode": "live_query",
                    "collections": dependencies
                });
                if let Some(req_id) = &req.id {
                    response["id"] = serde_json::Value::String(req_id.clone());
                }
                let _ = tx.send(Message::Text(response.to_string().into())).await;

                // 2. Setup aggregated change channel for dependencies
                let (dep_tx, mut dep_rx) =
                    tokio::sync::mpsc::channel::<crate::storage::collection::ChangeEvent>(1000);

                // 3. Subscribe to ALL dependencies
                for coll_name in &dependencies {
                    let coll_name = coll_name.clone();

                    if let Ok(collection) = state
                        .storage
                        .get_database(&db_name)
                        .and_then(|db| db.get_collection(&coll_name))
                    {
                        // A. Subscribe to local logical
                        let mut local_rx = collection.change_sender.subscribe();
                        let tx_local = dep_tx.clone();
                        tokio::spawn(async move {
                            loop {
                                match local_rx.recv().await {
                                    Ok(event) => {
                                        if tx_local.send(event).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                        continue
                                    }
                                    Err(_) => break,
                                }
                            }
                        });

                        // B. Subscribe to local physical shards
                        if let Some(shard_config) = collection.get_shard_config() {
                            if shard_config.num_shards > 0 {
                                if let Ok(database) = state.storage.get_database(&db_name) {
                                    for shard_id in 0..shard_config.num_shards {
                                        let physical_name = format!("{}_s{}", coll_name, shard_id);
                                        if let Ok(physical_coll) =
                                            database.get_collection(&physical_name)
                                        {
                                            let mut shard_rx =
                                                physical_coll.change_sender.subscribe();
                                            let tx_shard = dep_tx.clone();
                                            tokio::spawn(async move {
                                                loop {
                                                    match shard_rx.recv().await {
                                                        Ok(event) => { if tx_shard.send(event).await.is_err() { break; } },
                                                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                                        Err(_) => break,
                                                    }
                                                }
                                            });
                                        }
                                    }
                                }
                            }
                        }

                        // C. Subscribe to REMOTE nodes
                        let is_local_only = req.local.unwrap_or(false);
                        if !is_local_only {
                            if let Some(shard_config) = collection.get_shard_config() {
                                if let Some(coordinator) = &state.shard_coordinator {
                                    let my_addr = coordinator.my_address();
                                    let all_nodes = coordinator.get_collection_nodes(&shard_config);
                                    let cluster_secret = state.cluster_secret();
                                    let mut remote_nodes = std::collections::HashSet::new();
                                    for node_addr in all_nodes {
                                        if node_addr != my_addr {
                                            remote_nodes.insert(node_addr);
                                        }
                                    }

                                    for node_addr in remote_nodes {
                                        let tx_remote = dep_tx.clone();
                                        let db_remote = db_name.clone();
                                        let c_remote = coll_name.clone();
                                        let n_addr = node_addr.clone();
                                        let secret_clone = cluster_secret.clone();

                                        tokio::spawn(async move {
                                            use crate::cluster::ClusterWebsocketClient;
                                            match ClusterWebsocketClient::connect(
                                                &n_addr,
                                                &db_remote,
                                                &c_remote,
                                                true,
                                                &secret_clone,
                                            )
                                            .await
                                            {
                                                Ok(stream) => {
                                                    tokio::pin!(stream);
                                                    while let Some(result) = stream.next().await {
                                                        if let Ok(event) = result {
                                                            if tx_remote.send(event).await.is_err()
                                                            {
                                                                break;
                                                            }
                                                        } else {
                                                            break;
                                                        }
                                                    }
                                                }
                                                Err(_) => {}
                                            }
                                        });
                                    }
                                }
                            }
                        }
                    }
                }

                drop(dep_tx); // Close original sender

                // 5. Initial Execution
                execute_live_query_step(
                    &tx,
                    state.storage.clone(),
                    query_str.clone(),
                    db_name.clone(),
                    state.shard_coordinator.clone(),
                    req.id.clone(),
                )
                .await;

                // 6. Reactive Loop
                while dep_rx.recv().await.is_some() {
                    // On ANY change to ANY dependency, re-run query
                    execute_live_query_step(
                        &tx,
                        state.storage.clone(),
                        query_str.clone(),
                        db_name.clone(),
                        state.shard_coordinator.clone(),
                        req.id.clone(),
                    )
                    .await;
                }
            }
            Err(e) => {
                let _ = tx
                    .send(Message::Text(
                        serde_json::json!({
                            "error": format!("Invalid SDBQL query: {}", e)
                        })
                        .to_string()
                        .into(),
                    ))
                    .await;
            }
        }
    } else {
        let _ = tx
            .send(Message::Text(
                serde_json::json!({
                    "error": "Missing 'query' field for live_query"
                })
                .to_string()
                .into(),
            ))
            .await;
    }
}

// Helper for live query execution
async fn execute_live_query_step(
    tx: &tokio::sync::mpsc::Sender<Message>,
    storage: Arc<StorageEngine>,
    query_str: String,
    db_name: String,
    shard_coordinator: Option<Arc<crate::sharding::ShardCoordinator>>,
    req_id: Option<String>,
) {
    // Execute SDBQL
    let exec_result = tokio::task::spawn_blocking(move || {
        match crate::sdbql::parser::parse(&query_str) {
            Ok(parsed) => {
                // Security check
                for clause in &parsed.body_clauses {
                    match clause {
                        crate::sdbql::BodyClause::Insert(_)
                        | crate::sdbql::BodyClause::Update(_)
                        | crate::sdbql::BodyClause::Remove(_) => {
                            return Err(crate::error::DbError::ExecutionError(
                                "Live queries are read-only".to_string(),
                            ));
                        }
                        _ => {}
                    }
                }

                let mut executor =
                    crate::sdbql::executor::QueryExecutor::with_database(&storage, db_name);
                if let Some(coord) = shard_coordinator {
                    executor = executor.with_shard_coordinator(coord);
                }
                executor.execute(&parsed)
            }
            Err(e) => Err(crate::error::DbError::ParseError(e.to_string())),
        }
    })
    .await
    .unwrap();

    match exec_result {
        Ok(results) => {
            let mut response = serde_json::json!({
                "type": "query_result",
                "result": results
            });
            if let Some(id) = req_id {
                response["id"] = serde_json::Value::String(id);
            }
            let _ = tx.send(Message::Text(response.to_string().into())).await;
        }
        Err(e) => {
            let mut response = serde_json::json!({
                "type": "error",
                "error": e.to_string()
            });
            if let Some(id) = req_id {
                response["id"] = serde_json::Value::String(id);
            }
            let _ = tx.send(Message::Text(response.to_string().into())).await;
        }
    }
}
