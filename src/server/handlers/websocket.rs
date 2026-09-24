use super::cluster::{collect_sysinfo, generate_cluster_status};
use super::system::AppState;
use crate::{server::handlers::auth::AuthParams, storage::StorageEngine};
use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query as AxumQuery, State,
    },
    http::HeaderMap,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::Arc;

/// Maximum WebSocket message size (1 MB) - prevents OOM attacks
const MAX_WS_MESSAGE_SIZE: usize = 1024 * 1024;

/// Validate the `Origin` header against `SOLIDB_CORS_ALLOWED_ORIGINS`.
/// Mirrors the HTTP CORS policy in `routes.rs`: empty allowlist = deny any
/// cross-origin request. Non-browser clients (no `Origin` header) are allowed.
/// Returns Ok(()) when the request may proceed, Err(()) to reject with 403.
fn validate_ws_origin(headers: &HeaderMap) -> Result<(), ()> {
    let origin = match headers.get("origin").and_then(|o| o.to_str().ok()) {
        Some(o) => o,
        None => return Ok(()), // No Origin header — non-browser client.
    };
    let allowed_raw = std::env::var("SOLIDB_CORS_ALLOWED_ORIGINS").unwrap_or_default();
    if allowed_raw == "*" {
        return Ok(());
    }
    if allowed_raw.is_empty() {
        tracing::warn!(
            "WebSocket: rejecting Origin '{}' — SOLIDB_CORS_ALLOWED_ORIGINS not set",
            origin
        );
        return Err(());
    }
    let allowed = allowed_raw
        .split(',')
        .map(str::trim)
        .any(|a| a == origin || a == "*");
    if allowed {
        Ok(())
    } else {
        tracing::warn!("WebSocket: rejecting disallowed Origin '{}'", origin);
        Err(())
    }
}

fn forbidden_response() -> Response {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(Body::empty())
        .expect("Valid status code should not fail")
        .into_response()
}

fn unauthorized_response() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(Body::empty())
        .expect("Valid status code should not fail")
        .into_response()
}

/// Apply the frame limits to an upgrade. Without them axum buffers up to its
/// 64 MB default before the `MAX_WS_MESSAGE_SIZE` check in the read loop ever
/// runs (Audit M1).
fn limit_upgrade(ws: WebSocketUpgrade) -> WebSocketUpgrade {
    ws.max_message_size(MAX_WS_MESSAGE_SIZE)
        .max_frame_size(MAX_WS_MESSAGE_SIZE)
}

/// How often an open socket's credential is re-checked (Audit L2).
const WS_REVALIDATE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Validate a WebSocket bearer token the way `auth_middleware` validates an
/// HTTP one: signature and expiry, then roles re-resolved from storage so a
/// revoked role or a deleted user takes effect on the socket too (Audit H7).
/// Skipping the refresh let `enforce()` resolve the token's stale roles and
/// cache them under `sub`, handing revoked privileges back to the user's HTTP
/// requests as well.
///
/// `allow_livequery` is true only for the changefeed, the one endpoint a
/// live-query token may be presented to.
async fn authenticate_ws_token(
    token: &str,
    storage: &Arc<StorageEngine>,
    allow_livequery: bool,
) -> Option<crate::server::auth::Claims> {
    let claims = crate::server::auth::AuthService::validate_token(token).ok()?;
    if claims.livequery == Some(true) && !allow_livequery {
        tracing::warn!("livequery token presented to a non-changefeed WebSocket");
        return None;
    }
    let storage = storage.clone();
    tokio::task::spawn_blocking(move || {
        let claims = crate::server::auth::refresh_jwt_roles(claims, &storage)?;
        // `refresh_jwt_roles` passes live-query and API-key subjects through
        // untouched; the credential check below covers those.
        check_ws_credential(&claims, &storage)
            .map_err(|reason| {
                tracing::warn!(
                    target: "audit",
                    user = %claims.sub,
                    "rejecting WebSocket token: {}",
                    reason
                );
            })
            .ok()?;
        Some(claims)
    })
    .await
    .ok()
    .flatten()
}

/// Whether the principal behind an open socket still holds the access it was
/// admitted with. `Err` carries the reason the socket must be closed.
///
/// A live-query token is valid for two seconds *to connect*; what it stands
/// for afterwards is its subject, so that is what gets re-checked. A role
/// change of either kind closes the socket: its subscriptions were
/// authorized against the old roles, and the client reconnects to have them
/// re-authorized.
fn check_ws_credential(
    claims: &crate::server::auth::Claims,
    storage: &StorageEngine,
) -> Result<(), &'static str> {
    if claims.livequery != Some(true) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as usize)
            .unwrap_or(usize::MAX);
        if claims.exp <= now {
            return Err("token expired");
        }
    }
    let system_db = storage
        .get_database("_system")
        .map_err(|_| "system database unavailable")?;

    if let Some(name) = claims.sub.strip_prefix("api-key:") {
        let coll = system_db
            .system_collection(crate::server::auth::API_KEYS_COLL)
            .map_err(|_| "API key revoked")?;
        let now = chrono::Utc::now();
        let alive = coll
            .scan(None)
            .into_iter()
            .filter_map(|d| {
                serde_json::from_value::<crate::server::auth::ApiKey>(d.to_value()).ok()
            })
            .any(|k| {
                k.name == name
                    && !k
                        .expires_at
                        .as_deref()
                        .and_then(|e| chrono::DateTime::parse_from_rfc3339(e).ok())
                        .is_some_and(|e| e < now)
            });
        return if alive {
            Ok(())
        } else {
            Err("API key revoked or expired")
        };
    }

    let admins = system_db
        .system_collection(crate::server::auth::ADMIN_COLL)
        .map_err(|_| "user no longer exists")?;
    if admins.get(&claims.sub).is_err() {
        return Err("user no longer exists");
    }
    let normalized = |roles: Option<Vec<String>>| {
        let mut roles = roles.unwrap_or_default();
        roles.sort();
        roles.dedup();
        roles
    };
    let current = crate::server::auth::AuthService::get_user_roles(storage, &claims.sub);
    if normalized(current) != normalized(claims.roles.clone()) {
        return Err("roles changed");
    }
    Ok(())
}

/// `check_ws_credential` off the async runtime (it reads `_system`).
async fn ws_credential_still_valid(
    claims: &crate::server::auth::Claims,
    storage: &Arc<StorageEngine>,
) -> bool {
    let claims = claims.clone();
    let storage = storage.clone();
    tokio::task::spawn_blocking(move || match check_ws_credential(&claims, &storage) {
        Ok(()) => true,
        Err(reason) => {
            tracing::warn!(
                target: "audit",
                user = %claims.sub,
                "closing WebSocket: {}",
                reason
            );
            false
        }
    })
    .await
    .unwrap_or(false)
}

fn session_ended_message() -> Message {
    Message::Text(
        serde_json::json!({
            "type": "error",
            "error": "Session no longer valid; reconnect with a fresh token"
        })
        .to_string()
        .into(),
    )
}

// ==================== Cluster Status WebSocket ====================

/// WebSocket handler for real-time cluster status updates
pub async fn cluster_status_ws(
    ws: WebSocketUpgrade,
    AxumQuery(params): AxumQuery<AuthParams>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let Some(claims) = authenticate_ws_token(&params.token, &state.storage, false).await else {
        return unauthorized_response();
    };
    if crate::server::authz_middleware::enforce(
        &claims,
        &state,
        crate::server::authorization::PermissionAction::Admin,
        None,
    )
    .await
    .is_err()
    {
        return forbidden_response();
    }

    if validate_ws_origin(&headers).is_err() {
        return forbidden_response();
    }

    limit_upgrade(ws).on_upgrade(|socket| handle_cluster_ws(socket, state, claims))
}

/// Handle the WebSocket connection for cluster status
async fn handle_cluster_ws(
    mut socket: WebSocket,
    state: AppState,
    claims: crate::server::auth::Claims,
) {
    use tokio::time::{interval, Duration};

    let mut ticker = interval(Duration::from_secs(1));
    let mut validated_at = tokio::time::Instant::now();

    // We use the shared system monitor from AppState to avoid expensive initialization
    // and to ensure CPU usage is calculated correctly (delta since last refresh).

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if validated_at.elapsed() >= WS_REVALIDATE_INTERVAL {
                    if !ws_credential_still_valid(&claims, &state.storage).await {
                        let _ = socket.send(session_ended_message()).await;
                        break;
                    }
                    validated_at = tokio::time::Instant::now();
                }
                // Extract sysinfo under a short lock, then generate status without holding it
                let sysinfo = {
                    let mut sys = state.system_monitor.lock().unwrap();
                    collect_sysinfo(&mut sys)
                };
                let status = generate_cluster_status(&state, &sysinfo);

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
                    #[allow(clippy::collapsible_match)]
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
    headers: HeaderMap,
) -> Response {
    // The token was validated but never authorized, so any principal with a
    // valid token — a read-only viewer included — received the host name,
    // kernel version and live CPU/memory of the server. Monitoring is an
    // operator view: require global admin, as the cluster-status socket
    // already does.
    let Some(claims) = authenticate_ws_token(&params.token, &state.storage, false).await else {
        return unauthorized_response();
    };

    if crate::server::authz_middleware::enforce(
        &claims,
        &state,
        crate::server::authorization::PermissionAction::Admin,
        None,
    )
    .await
    .is_err()
    {
        return forbidden_response();
    }

    if validate_ws_origin(&headers).is_err() {
        return forbidden_response();
    }

    limit_upgrade(ws).on_upgrade(|socket| handle_monitor_socket(socket, state, claims))
}

async fn handle_monitor_socket(
    mut socket: WebSocket,
    state: AppState,
    claims: crate::server::auth::Claims,
) {
    use std::sync::atomic::Ordering;

    tracing::info!("Monitor WS: Client connected");

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
    let mut validated_at = tokio::time::Instant::now();

    loop {
        // Wait for next tick
        interval.tick().await;

        if validated_at.elapsed() >= WS_REVALIDATE_INTERVAL {
            if !ws_credential_still_valid(&claims, &state.storage).await {
                let _ = socket.send(session_ended_message()).await;
                break;
            }
            validated_at = tokio::time::Instant::now();
        }

        let stats = {
            let mut sys = state.system_monitor.lock().unwrap();

            // Refresh specific stats
            sys.refresh_cpu_all();
            sys.refresh_memory();

            let cpu = sys.global_cpu_usage();
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

    // If not cluster-internal, validate the JWT token. Keep the claims:
    // each subscription is authorized against the database it targets.
    let claims = if is_cluster_internal {
        crate::server::auth::Claims {
            sub: "cluster-internal".to_string(),
            exp: usize::MAX,
            livequery: None,
            roles: Some(vec!["admin".to_string()]),
            scoped_databases: None,
        }
    } else {
        match authenticate_ws_token(&params.token, &state.storage, true).await {
            Some(claims) => claims,
            None => return unauthorized_response(),
        }
    };

    if validate_ws_origin(&headers).is_err() {
        return forbidden_response();
    }

    // Check if HTMX mode is requested
    let use_htmx = params.htmx.map(|s| s == "true").unwrap_or(false);

    limit_upgrade(ws).on_upgrade(move |socket| {
        handle_socket(socket, state, claims, use_htmx, is_cluster_internal)
    })
}

async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    claims: crate::server::auth::Claims,
    use_htmx: bool,
    is_cluster_internal: bool,
) {
    // Split socket into sender and receiver
    let (mut sender, mut receiver) = socket.split();

    // Unified channel for sending messages to the client
    // All subscription tasks and live queries will send ready-to-emit Messages to this channel
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Message>(1000);

    // Spawn writer task that forwards messages from the channel to the WebSocket
    let mut send_task = tokio::spawn(async move {
        // Heartbeat: Send a Ping every 30 seconds to keep the connection alive.
        // Start the interval 30s in the future — `tokio::time::interval` would
        // otherwise fire its first tick immediately, racing the first response
        // frame and surfacing a stray Ping to clients that only ever expect a
        // reply to what they just sent.
        let heartbeat = std::time::Duration::from_secs(30);
        let mut heartbeat_interval =
            tokio::time::interval_at(tokio::time::Instant::now() + heartbeat, heartbeat);

        loop {
            tokio::select! {
                // Send heartbeat
                _ = heartbeat_interval.tick() => {
                    if sender.send(Message::Ping(vec![].into())).await.is_err() {
                        tracing::debug!("[WS] Failed to send ping, closing writer");
                        break;
                    }
                }
                // Forward messages; stop once every sender is gone and the
                // queue is drained.
                msg = rx.recv() => {
                    let Some(msg) = msg else { break };
                    if sender.send(msg).await.is_err() {
                        tracing::debug!("[WS] Failed to send message, closing writer");
                        break;
                    }
                }
            }
        }
    });

    // Each subscription spawns a task with its own change-event buffers;
    // nothing bounded how many one socket could open.
    let mut subscriptions = 0usize;

    // Every subscription and live query runs in this set, and each owns a
    // JoinSet of its forwarders, so dropping it when the socket ends aborts
    // the whole tree. Forwarders used to notice a gone client only when a
    // send failed — never, on a quiet collection or a key filter that never
    // matched — so reconnect loops piled them up (Audit M1).
    let mut tasks = tokio::task::JoinSet::new();

    let mut revalidate = tokio::time::interval_at(
        tokio::time::Instant::now() + WS_REVALIDATE_INTERVAL,
        WS_REVALIDATE_INTERVAL,
    );

    // Main Receiver Loop
    loop {
        let msg = tokio::select! {
            next = receiver.next() => match next {
                Some(Ok(msg)) => msg,
                _ => break,
            },
            // The writer is gone (client stopped reading): nothing more can
            // be delivered.
            _ = tx.closed() => break,
            _ = revalidate.tick() => {
                if !is_cluster_internal
                    && !ws_credential_still_valid(&claims, &state.storage).await
                {
                    let _ = tx.send(session_ended_message()).await;
                    break;
                }
                continue;
            }
            // Reap finished subscription tasks so the set does not grow.
            Some(_) = tasks.join_next(), if !tasks.is_empty() => continue,
        };
        // Security: Check message size to prevent OOM attacks
        let msg_len = match &msg {
            Message::Text(text) => text.len(),
            Message::Binary(data) => data.len(),
            Message::Ping(data) => data.len(),
            Message::Pong(data) => data.len(),
            _ => 0,
        };

        if msg_len > MAX_WS_MESSAGE_SIZE {
            tracing::warn!(
                "[WS] Message size {} exceeds limit {}, closing connection",
                msg_len,
                MAX_WS_MESSAGE_SIZE
            );
            let _ = tx
                .send(Message::Text(
                    serde_json::json!({
                        "error": "Message too large"
                    })
                    .to_string()
                    .into(),
                ))
                .await;
            break;
        }

        match msg {
            Message::Text(text) => {
                let req_result = serde_json::from_str::<ChangefeedRequest>(&text);
                match req_result {
                    Ok(req) if req.type_ == "subscribe" => {
                        if subscriptions >= MAX_SUBSCRIPTIONS_PER_CONNECTION {
                            let _ = tx.send(subscription_limit_message()).await;
                            continue;
                        }
                        subscriptions += 1;
                        let tx_clone = tx.clone();
                        let state_clone = state.clone();
                        let claims_clone = claims.clone();

                        // Spawn a dedicated task for this subscription
                        tasks.spawn(async move {
                            handle_subscribe_request(
                                req,
                                state_clone,
                                claims_clone,
                                tx_clone,
                                use_htmx,
                            )
                            .await;
                        });
                    }
                    Ok(req) if req.type_ == "live_query" => {
                        if subscriptions >= MAX_SUBSCRIPTIONS_PER_CONNECTION {
                            let _ = tx.send(subscription_limit_message()).await;
                            continue;
                        }
                        subscriptions += 1;
                        let tx_clone = tx.clone();
                        let state_clone = state.clone();
                        let claims_clone = claims.clone();

                        // Spawn a dedicated task for this live query
                        tasks.spawn(async move {
                            handle_live_query_request(req, state_clone, claims_clone, tx_clone)
                                .await;
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

    // Stop every subscription (and, through their JoinSets, every
    // forwarder and remote cluster stream) before the writer.
    tasks.shutdown().await;
    // Give the writer a moment to flush what is queued (such as the reason
    // for closing), then stop it regardless.
    drop(tx);
    if tokio::time::timeout(std::time::Duration::from_secs(1), &mut send_task)
        .await
        .is_err()
    {
        send_task.abort();
    }
}

/// Handle a single subscription request
/// Subscriptions and live queries one WebSocket connection may open.
const MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 64;

fn subscription_limit_message() -> Message {
    Message::Text(
        serde_json::json!({
            "type": "error",
            "error": format!(
                "Subscription limit reached ({} per connection)",
                MAX_SUBSCRIPTIONS_PER_CONNECTION
            ),
        })
        .to_string()
        .into(),
    )
}

/// Forward one collection's broadcast change events into a subscription's
/// aggregate channel until the subscription goes away.
///
/// Selecting on `out.closed()` matters: waiting on `rx.recv()` alone meant a
/// forwarder on a quiet collection never learned its subscriber had left.
async fn forward_changes(
    mut rx: tokio::sync::broadcast::Receiver<crate::storage::collection::ChangeEvent>,
    out: tokio::sync::mpsc::Sender<crate::storage::collection::ChangeEvent>,
    key: Option<String>,
    source: String,
    lag_notice: Option<tokio::sync::mpsc::Sender<Message>>,
) {
    use tokio::sync::broadcast::error::RecvError;
    loop {
        let received = tokio::select! {
            _ = out.closed() => break,
            received = rx.recv() => received,
        };
        match received {
            Ok(event) => {
                if key.as_ref().is_some_and(|k| &event.key != k) {
                    continue;
                }
                if out.send(event).await.is_err() {
                    break;
                }
            }
            Err(RecvError::Lagged(skipped)) => {
                // The subscriber fell behind the broadcast buffer; those
                // events are gone. Say so rather than silently continuing.
                tracing::warn!(
                    "[WS] changefeed on '{}' lagged: {} events dropped",
                    source,
                    skipped
                );
                if let Some(client) = &lag_notice {
                    let _ = client.try_send(Message::Text(
                        serde_json::json!({
                            "type": "lagged",
                            "collection": source,
                            "skipped": skipped,
                        })
                        .to_string()
                        .into(),
                    ));
                }
            }
            Err(RecvError::Closed) => break,
        }
    }
}

/// Forward a remote node's change stream for one collection.
async fn forward_remote_changes(
    node_addr: String,
    db_name: String,
    coll_name: String,
    secret: String,
    out: tokio::sync::mpsc::Sender<crate::storage::collection::ChangeEvent>,
) {
    use crate::cluster::ClusterWebsocketClient;
    let connect = ClusterWebsocketClient::connect(&node_addr, &db_name, &coll_name, true, &secret);
    let stream = tokio::select! {
        _ = out.closed() => return,
        stream = connect => match stream {
            Ok(stream) => stream,
            Err(_) => return,
        },
    };
    tokio::pin!(stream);
    loop {
        let next = tokio::select! {
            _ = out.closed() => break,
            next = stream.next() => next,
        };
        match next {
            Some(Ok(event)) => {
                if out.send(event).await.is_err() {
                    break;
                }
            }
            _ => break,
        }
    }
}

/// Whether `doc` passes the collection's row policy for `principal` — the
/// same evaluation `apply_row_policy` does for a scan, so a changefeed shows
/// a non-admin exactly the rows `/cursor` would (Audit H2). An unparsable
/// policy or an evaluation error hides the row.
fn row_policy_allows(
    storage: &StorageEngine,
    db_name: String,
    collection: &str,
    principal: &crate::sdbql::QueryPrincipal,
    policy: &str,
    doc: serde_json::Value,
) -> bool {
    let Ok(mut parser) = crate::sdbql::parser::Parser::new(policy) else {
        return false;
    };
    let Ok(expr) = parser.parse_expression() else {
        return false;
    };
    let executor = crate::sdbql::executor::QueryExecutor::with_database(storage, db_name)
        .with_principal(principal.clone())
        .with_timeout(std::time::Duration::from_secs(5));
    let mut ctx = std::collections::HashMap::new();
    ctx.insert(collection.to_string(), doc.clone());
    ctx.insert("doc".to_string(), doc);
    ctx.insert(
        "CURRENT_USER".to_string(),
        serde_json::Value::String(principal.user.clone()),
    );
    executor
        .evaluate_expr_with_context(&expr, &ctx)
        .map(|v| crate::sdbql::executor::to_bool(&v))
        .unwrap_or(false)
}

/// Row-policy gate for one changefeed event.
///
/// The policy is re-read per event so a policy set after the subscription
/// opened still applies. Inserts and updates are judged on the new document
/// (an update that moves a row out of view is not announced, rather than
/// leaking its new content); deletes on the old one. A truncate names no
/// row and passes.
async fn changefeed_event_visible(
    storage: &Arc<StorageEngine>,
    db_name: &str,
    collection: &crate::storage::Collection,
    principal: &crate::sdbql::QueryPrincipal,
    event: &crate::storage::collection::ChangeEvent,
) -> bool {
    use crate::storage::collection::ChangeType;
    if principal.can_admin {
        return true;
    }
    let Some(policy) = collection.get_row_policy() else {
        return true;
    };
    let row = match event.type_ {
        ChangeType::Truncate => return true,
        ChangeType::Insert | ChangeType::Update => event.data.clone(),
        ChangeType::Delete => event.old_data.clone().or_else(|| event.data.clone()),
    };
    let Some(row) = row else {
        return false;
    };
    let storage = storage.clone();
    let db_name = db_name.to_string();
    let coll_name = collection.name.clone();
    let principal = principal.clone();
    tokio::task::spawn_blocking(move || {
        row_policy_allows(&storage, db_name, &coll_name, &principal, &policy, row)
    })
    .await
    .unwrap_or(false)
}

async fn handle_subscribe_request(
    req: ChangefeedRequest,
    state: AppState,
    claims: crate::server::auth::Claims,
    tx: tokio::sync::mpsc::Sender<Message>,
    use_htmx: bool,
) {
    let db_name = req.database.clone().unwrap_or("_system".to_string());

    // A changefeed exposes every document change in the collection; require
    // read permission on the target database before subscribing.
    if let Err(e) = crate::server::authz_middleware::enforce(
        &claims,
        &state,
        crate::server::authorization::PermissionAction::Read,
        Some(&db_name),
    )
    .await
    {
        let mut response = serde_json::json!({ "error": e.to_string() });
        if let Some(req_id) = &req.id {
            response["id"] = serde_json::Value::String(req_id.clone());
        }
        let _ = tx.send(Message::Text(response.to_string().into())).await;
        return;
    }

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
            let lag_notice = if use_htmx { None } else { Some(tx.clone()) };
            // Owned by this task: aborting the subscription aborts these.
            let mut forwarders = tokio::task::JoinSet::new();

            // 1. Subscribe to local logical collection
            forwarders.spawn(forward_changes(
                collection.change_sender.subscribe(),
                sub_tx.clone(),
                req_key.clone(),
                coll_name.clone(),
                lag_notice.clone(),
            ));

            // 2. Subscribe to PHYSICAL SHARDS (if sharded)
            if let Some(shard_config) = collection.get_shard_config() {
                if shard_config.num_shards > 0 {
                    if let Ok(database) = state.storage.get_database(&db_name) {
                        for shard_id in 0..shard_config.num_shards {
                            let physical_name = format!("{}_s{}", coll_name, shard_id);
                            if let Ok(physical_coll) = database.get_collection(&physical_name) {
                                forwarders.spawn(forward_changes(
                                    physical_coll.change_sender.subscribe(),
                                    sub_tx.clone(),
                                    req_key.clone(),
                                    physical_name,
                                    lag_notice.clone(),
                                ));
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
                            forwarders.spawn(forward_remote_changes(
                                node_addr,
                                db_name.clone(),
                                coll_name.clone(),
                                cluster_secret.clone(),
                                sub_tx.clone(),
                            ));
                        }
                    }
                }
            }

            // Drop original sub_tx so we don't hold the channel open forever if all producers die
            drop(sub_tx);

            let principal = crate::server::handlers::query::principal_from_claims(&claims);

            // Forward aggregated events to the main socket channel
            loop {
                let event = tokio::select! {
                    _ = tx.closed() => break,
                    event = sub_rx.recv() => match event {
                        Some(event) => event,
                        None => break,
                    },
                };
                // Double check filter (especially for remote events)
                if let Some(ref target_key) = req.key {
                    if &event.key != target_key {
                        continue;
                    }
                }
                // Remote and shard events are filtered against the logical
                // collection's policy here too.
                if !changefeed_event_visible(
                    &state.storage,
                    &db_name,
                    &collection,
                    &principal,
                    &event,
                )
                .await
                {
                    continue;
                }

                // Format message
                let msg_text = if use_htmx {
                    use crate::storage::collection::ChangeType;
                    let op_type = match event.type_ {
                        ChangeType::Insert => "INSERT",
                        ChangeType::Update => "UPDATE",
                        ChangeType::Delete => "DELETE",
                        ChangeType::Truncate => "TRUNCATE",
                    };
                    let status_class = match event.type_ {
                        ChangeType::Insert => "bg-success/10 text-success",
                        ChangeType::Update => "bg-warning/10 text-warning",
                        ChangeType::Delete => "bg-error/10 text-error",
                        ChangeType::Truncate => "bg-error/10 text-error",
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
    claims: crate::server::auth::Claims,
    tx: tokio::sync::mpsc::Sender<Message>,
) {
    if let Some(query_str) = req.query {
        let db_name = req.database.clone().unwrap_or("_system".to_string());

        // Live queries re-execute against the database on every change;
        // require read permission before registering the subscription.
        if let Err(e) = crate::server::authz_middleware::enforce(
            &claims,
            &state,
            crate::server::authorization::PermissionAction::Read,
            Some(&db_name),
        )
        .await
        {
            let mut response = serde_json::json!({ "error": e.to_string() });
            if let Some(req_id) = &req.id {
                response["id"] = serde_json::Value::String(req_id.clone());
            }
            let _ = tx.send(Message::Text(response.to_string().into())).await;
            return;
        }

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
                if tx
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .is_err()
                {
                    return;
                }

                // 2. Setup aggregated change channel for dependencies
                let (dep_tx, mut dep_rx) =
                    tokio::sync::mpsc::channel::<crate::storage::collection::ChangeEvent>(1000);

                // Owned by this task: aborting the live query aborts these.
                let mut forwarders = tokio::task::JoinSet::new();

                // 3. Subscribe to ALL dependencies
                for coll_name in &dependencies {
                    let coll_name = coll_name.clone();

                    if let Ok(collection) = state
                        .storage
                        .get_database(&db_name)
                        .and_then(|db| db.get_collection(&coll_name))
                    {
                        // A. Subscribe to local logical. Events only trigger a
                        // re-run, so a lag needs no client notice: the events
                        // still buffered trigger the next one.
                        forwarders.spawn(forward_changes(
                            collection.change_sender.subscribe(),
                            dep_tx.clone(),
                            None,
                            coll_name.clone(),
                            None,
                        ));

                        // B. Subscribe to local physical shards
                        if let Some(shard_config) = collection.get_shard_config() {
                            if shard_config.num_shards > 0 {
                                if let Ok(database) = state.storage.get_database(&db_name) {
                                    for shard_id in 0..shard_config.num_shards {
                                        let physical_name = format!("{}_s{}", coll_name, shard_id);
                                        if let Ok(physical_coll) =
                                            database.get_collection(&physical_name)
                                        {
                                            forwarders.spawn(forward_changes(
                                                physical_coll.change_sender.subscribe(),
                                                dep_tx.clone(),
                                                None,
                                                physical_name,
                                                None,
                                            ));
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
                                        forwarders.spawn(forward_remote_changes(
                                            node_addr,
                                            db_name.clone(),
                                            coll_name.clone(),
                                            cluster_secret.clone(),
                                            dep_tx.clone(),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }

                drop(dep_tx); // Close original sender

                // The query runs as the caller, exactly as `/cursor` would run
                // it: CURRENT_USER, CAN() and row policies all need the
                // principal, and a live query used to run with none (Audit H2).
                let principal = crate::server::handlers::query::principal_from_claims(&claims);

                // 5. Initial Execution
                if !execute_live_query_step(
                    &tx,
                    state.storage.clone(),
                    query_str.clone(),
                    db_name.clone(),
                    state.shard_coordinator.clone(),
                    principal.clone(),
                    req.id.clone(),
                )
                .await
                {
                    return;
                }

                // 6. Reactive Loop
                // Coalesce change bursts: a bulk write of N documents used to
                // re-run the query N times per subscriber. After the first
                // event, keep absorbing events until the stream is quiet for
                // DEBOUNCE — capped at MAX_DELAY from the first event so a
                // continuous write stream can't starve the subscriber of
                // updates. Events arriving while the query re-runs stay
                // buffered in the channel and start the next cycle.
                const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);
                const MAX_DELAY: std::time::Duration = std::time::Duration::from_millis(500);
                'reactive: loop {
                    tokio::select! {
                        _ = tx.closed() => break 'reactive,
                        first = dep_rx.recv() => {
                            if first.is_none() {
                                break 'reactive; // all forwarders gone
                            }
                        }
                    }
                    let deadline = tokio::time::Instant::now() + MAX_DELAY;
                    loop {
                        tokio::select! {
                            more = dep_rx.recv() => {
                                if more.is_none() {
                                    break 'reactive; // all forwarders gone
                                }
                                if tokio::time::Instant::now() >= deadline {
                                    break; // burst still going — run anyway
                                }
                            }
                            _ = tokio::time::sleep(DEBOUNCE) => break,
                        }
                    }
                    // If the client is gone, the send returns an error and we
                    // bail so the forwarder tasks above can shut down (they
                    // break once dep_rx is dropped here).
                    if !execute_live_query_step(
                        &tx,
                        state.storage.clone(),
                        query_str.clone(),
                        db_name.clone(),
                        state.shard_coordinator.clone(),
                        principal.clone(),
                        req.id.clone(),
                    )
                    .await
                    {
                        break;
                    }
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

// Helper for live query execution. Returns false if the client channel has
// been closed (so the caller should stop the reactive loop and let the
// forwarder tasks shut down).
async fn execute_live_query_step(
    tx: &tokio::sync::mpsc::Sender<Message>,
    storage: Arc<StorageEngine>,
    query_str: String,
    db_name: String,
    shard_coordinator: Option<Arc<crate::sharding::ShardCoordinator>>,
    principal: crate::sdbql::QueryPrincipal,
    req_id: Option<String>,
) -> bool {
    // Execute SDBQL
    let exec_result = tokio::task::spawn_blocking(move || {
        match crate::sdbql::parser::parse(&query_str) {
            Ok(parsed) => {
                // Live-query subscriptions are authorized as Read, and this
                // query is re-executed on every matching change, so it must be
                // a read. The check used to be a hand-rolled match over
                // `body_clauses` for Insert/Update/Remove only, which let
                // UPSERT, CREATE STREAM, CREATE/REFRESH MATERIALIZED VIEW,
                // mutating set-operation operands, mutating CTEs and
                // mutations nested in subqueries through — all with a
                // read-only key. `has_mutations()` is the single definition of
                // "this query writes", shared with `/cursor` and the
                // transactional query endpoint.
                if parsed.has_mutations() {
                    return Err(crate::error::DbError::ExecutionError(
                        "Live queries are read-only".to_string(),
                    ));
                }

                let mut executor =
                    crate::sdbql::executor::QueryExecutor::with_database(&storage, db_name)
                        .with_principal(principal)
                        .with_timeout(std::time::Duration::from_secs(30));
                if let Some(coord) = shard_coordinator {
                    executor = executor.with_shard_coordinator(coord);
                }
                executor.execute(&parsed)
            }
            Err(e) => Err(crate::error::DbError::ParseError(e.to_string())),
        }
    })
    .await
    // A panic inside the executor is a failed evaluation of this live
    // query, not a reason to take the connection down with a second panic.
    .unwrap_or_else(|e| {
        Err(crate::error::DbError::InternalError(format!(
            "Live query task failed: {}",
            e
        )))
    });

    match exec_result {
        Ok(results) => {
            let mut response = serde_json::json!({
                "type": "query_result",
                "result": results
            });
            if let Some(id) = req_id {
                response["id"] = serde_json::Value::String(id);
            }
            tx.send(Message::Text(response.to_string().into()))
                .await
                .is_ok()
        }
        Err(e) => {
            let mut response = serde_json::json!({
                "type": "error",
                "error": e.to_string()
            });
            if let Some(id) = req_id {
                response["id"] = serde_json::Value::String(id);
            }
            tx.send(Message::Text(response.to_string().into()))
                .await
                .is_ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::auth::Claims;

    fn engine() -> (StorageEngine, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let engine = StorageEngine::new(tmp.path().to_str().unwrap()).expect("engine");
        (engine, tmp)
    }

    fn claims(sub: &str, exp: usize, livequery: Option<bool>) -> Claims {
        Claims {
            sub: sub.to_string(),
            exp,
            livequery,
            roles: None,
            scoped_databases: None,
        }
    }

    #[test]
    fn row_policy_filters_changefeed_rows_by_principal() {
        let (engine, _tmp) = engine();
        engine.create_database("app".to_string()).unwrap();
        let alice = crate::sdbql::QueryPrincipal::from_roles("alice", vec!["viewer".into()]);
        let policy = "doc.owner == CURRENT_USER";
        let own = serde_json::json!({"_key": "1", "owner": "alice"});
        let other = serde_json::json!({"_key": "2", "owner": "bob"});

        assert!(row_policy_allows(
            &engine,
            "app".into(),
            "orders",
            &alice,
            policy,
            own
        ));
        assert!(!row_policy_allows(
            &engine,
            "app".into(),
            "orders",
            &alice,
            policy,
            other.clone()
        ));
        // An unparsable policy hides the row rather than exposing it.
        assert!(!row_policy_allows(
            &engine,
            "app".into(),
            "orders",
            &alice,
            "((",
            other
        ));
    }

    #[test]
    fn ws_credential_follows_user_existence_and_expiry() {
        let (engine, _tmp) = engine();
        engine.create_database("_system".to_string()).unwrap();
        let system = engine.get_database("_system").unwrap();
        system
            .create_collection(crate::server::auth::ADMIN_COLL.to_string(), None)
            .unwrap();
        system
            .system_collection(crate::server::auth::ADMIN_COLL)
            .unwrap()
            .insert(serde_json::json!({"_key": "ws_cred_alice", "password_hash": "x"}))
            .unwrap();

        let far = usize::MAX - 1;
        assert!(check_ws_credential(&claims("ws_cred_alice", far, None), &engine).is_ok());
        // Deleted (here: never created) user.
        assert!(check_ws_credential(&claims("ws_cred_bob", far, None), &engine).is_err());
        // Expired session token.
        assert!(check_ws_credential(&claims("ws_cred_alice", 1, None), &engine).is_err());
        // A live-query token only had to be fresh to connect; its subject is
        // what keeps the socket alive.
        assert!(check_ws_credential(&claims("ws_cred_alice", 1, Some(true)), &engine).is_ok());
        assert!(check_ws_credential(&claims("ws_cred_bob", 1, Some(true)), &engine).is_err());
        // Roles that no longer match the current assignment close the socket.
        let mut stale = claims("ws_cred_alice", far, None);
        stale.roles = Some(vec!["admin".to_string()]);
        assert!(check_ws_credential(&stale, &engine).is_err());
        // An API-key subject with no such key.
        assert!(check_ws_credential(&claims("api-key:ws_cred_gone", far, None), &engine).is_err());
    }
}
