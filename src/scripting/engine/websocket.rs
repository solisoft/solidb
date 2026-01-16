use std::sync::atomic::Ordering;
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use mlua::{Lua, Value as LuaValue};
use tokio::sync::mpsc;
use tracing;

use crate::error::DbError;
use crate::scripting::channel_manager::{ChannelEvent, ChannelManager, ConnectionId};
use crate::scripting::conversion::{json_to_lua, lua_value_to_json};
use crate::scripting::types::{Script, ScriptContext};

use super::ScriptEngine;

pub async fn execute_ws(
    engine: &ScriptEngine,
    script: &Script,
    db_name: &str,
    context: &ScriptContext,
    ws: axum::extract::ws::WebSocket,
) -> Result<(), DbError> {
    engine.stats.active_ws.fetch_add(1, Ordering::SeqCst);
    engine
        .stats
        .total_ws_connections
        .fetch_add(1, Ordering::SeqCst);

    // Ensure active counter is decremented even on panic or early return
    struct ActiveWsGuard(Arc<crate::scripting::types::ScriptStats>);
    impl Drop for ActiveWsGuard {
        fn drop(&mut self) {
            self.0.active_ws.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _guard = ActiveWsGuard(engine.stats.clone());

    // Register connection with channel manager for pub/sub and presence
    let channel_manager = engine.channel_manager.clone();
    let (conn_id, event_rx): (ConnectionId, mpsc::Receiver<ChannelEvent>) =
        if let Some(cm) = &channel_manager {
            cm.register_connection(db_name)
        } else {
            // Create a dummy receiver if no channel manager
            let (_tx, rx) = mpsc::channel(1);
            (uuid::Uuid::new_v4().to_string(), rx)
        };

    // Guard for automatic connection cleanup
    struct ConnectionGuard {
        conn_id: ConnectionId,
        channel_manager: Option<Arc<ChannelManager>>,
    }
    impl Drop for ConnectionGuard {
        fn drop(&mut self) {
            if let Some(cm) = &self.channel_manager {
                cm.unregister_connection(&self.conn_id);
            }
        }
    }
    let _conn_guard = ConnectionGuard {
        conn_id: conn_id.clone(),
        channel_manager: channel_manager.clone(),
    };

    let lua = Lua::new();

    // Secure environment: Remove unsafe standard libraries and functions
    let globals = lua.globals();
    globals
        .set("os", LuaValue::Nil)
        .map_err(|e| DbError::InternalError(format!("Failed to secure os: {}", e)))?;
    globals
        .set("io", LuaValue::Nil)
        .map_err(|e| DbError::InternalError(format!("Failed to secure io: {}", e)))?;
    globals
        .set("debug", LuaValue::Nil)
        .map_err(|e| DbError::InternalError(format!("Failed to secure debug: {}", e)))?;
    globals
        .set("package", LuaValue::Nil)
        .map_err(|e| DbError::InternalError(format!("Failed to secure package: {}", e)))?;
    globals
        .set("dofile", LuaValue::Nil)
        .map_err(|e| DbError::InternalError(format!("Failed to secure dofile: {}", e)))?;
    globals
        .set("load", LuaValue::Nil)
        .map_err(|e| DbError::InternalError(format!("Failed to secure load: {}", e)))?;
    globals
        .set("loadfile", LuaValue::Nil)
        .map_err(|e| DbError::InternalError(format!("Failed to secure loadfile: {}", e)))?;
    globals
        .set("require", LuaValue::Nil)
        .map_err(|e| DbError::InternalError(format!("Failed to secure require: {}", e)))?;

    // Set up the Lua environment
    engine.setup_lua_globals(&lua, db_name, context, Some((&script.key, &script.name)))?;

    // Set up WebSocket specific globals
    let ws_table = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create ws table: {}", e)))?;

    // Split WebSocket into sink and stream
    let (mut sink, receiver) = ws.split();
    let (tx, mut rx) = mpsc::channel::<axum::extract::ws::Message>(100);

    // Define type alias for readability and inference
    type WsStream = futures::stream::SplitStream<axum::extract::ws::WebSocket>;

    // Task to forward messages from channel to WebSocket sink
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Heartbeat task: Send a Ping every 30 seconds to keep the connection alive
    let tx_heartbeat = tx.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        // First tick happens immediately
        interval.tick().await;
        loop {
            interval.tick().await;
            if tx_heartbeat
                .send(axum::extract::ws::Message::Ping(vec![].into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let receiver_arc = Arc::new(tokio::sync::Mutex::<WsStream>::new(receiver));
    let event_rx_arc = Arc::new(tokio::sync::Mutex::new(event_rx));

    // ws.send(data)
    let tx_send = tx.clone();
    let send_fn = lua
        .create_async_function(move |_, data: String| {
            let tx = tx_send.clone();
            async move {
                tx.send(axum::extract::ws::Message::Text(data.into()))
                    .await
                    .map_err(|e| mlua::Error::RuntimeError(format!("WS send error: {}", e)))?;
                Ok(())
            }
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create ws.send: {}", e)))?;
    ws_table
        .set("send", send_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set ws.send: {}", e)))?;

    // ws.recv() -> string or nil
    let ws_recv_clone = receiver_arc.clone();
    let recv_fn = lua
        .create_async_function(move |lua, (): ()| {
            let stream_inner = ws_recv_clone.clone();
            async move {
                let mut stream: tokio::sync::MutexGuard<'_, WsStream> = stream_inner.lock().await;
                loop {
                    match stream.next().await {
                        Some(Ok(axum::extract::ws::Message::Text(t))) => {
                            return Ok(LuaValue::String(lua.create_string(t.as_bytes())?))
                        }
                        Some(Ok(axum::extract::ws::Message::Binary(b))) => {
                            return Ok(LuaValue::String(lua.create_string(b.as_ref())?))
                        }
                        Some(Ok(axum::extract::ws::Message::Close(_))) | None | Some(Err(_)) => {
                            return Ok(LuaValue::Nil)
                        }
                        Some(Ok(axum::extract::ws::Message::Pong(_)))
                        | Some(Ok(axum::extract::ws::Message::Ping(_))) => continue, // Ignore heartbeats
                    }
                }
            }
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create ws.recv: {}", e)))?;
    ws_table
        .set("recv", recv_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set ws.recv: {}", e)))?;

    // ws.recv_any(timeout_ms) -> (msg, type) or nil
    // Returns messages from both WebSocket and channel events
    let ws_recv_any_clone = receiver_arc.clone();
    let event_rx_clone = event_rx_arc.clone();
    let recv_any_fn = lua
        .create_async_function(move |lua, timeout_ms: Option<u64>| {
            let ws_stream = ws_recv_any_clone.clone();
            let event_rx = event_rx_clone.clone();
            async move {
                let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(30000));

                tokio::select! {
                    biased;

                    // Check channel events first (they're usually more important for real-time)
                    result = async {
                        event_rx.lock().await.recv().await
                    } => {
                        match result {
                            Some(ChannelEvent::Message(msg)) => {
                                let msg_table = lua.create_table()?;
                                msg_table.set("channel", msg.channel.as_str())?;
                                msg_table.set("data", json_to_lua(&lua, &msg.data)?)?;
                                msg_table.set("timestamp", msg.timestamp)?;
                                if let Some(sender) = &msg.sender_id {
                                    msg_table.set("sender_id", sender.as_str())?;
                                }
                                let result_table = lua.create_table()?;
                                result_table.set(1, msg_table)?;
                                result_table.set(2, "channel")?;
                                Ok(LuaValue::Table(result_table))
                            }
                            Some(ChannelEvent::Presence(event)) => {
                                let event_table = lua.create_table()?;
                                event_table.set("event_type", event.event_type.to_string())?;
                                event_table.set("channel", event.channel.as_str())?;
                                event_table.set("user_info", json_to_lua(&lua, &event.user_info)?)?;
                                event_table.set("connection_id", event.connection_id.as_str())?;
                                event_table.set("timestamp", event.timestamp)?;
                                let result_table = lua.create_table()?;
                                result_table.set(1, event_table)?;
                                result_table.set(2, "presence")?;
                                Ok(LuaValue::Table(result_table))
                            }
                            None => Ok(LuaValue::Nil),
                        }
                    }

                    // WebSocket message
                    result = async {
                        let mut stream = ws_stream.lock().await;
                        stream.next().await
                    } => {
                        match result {
                            Some(Ok(axum::extract::ws::Message::Text(t))) => {
                                let result_table = lua.create_table()?;
                                result_table.set(1, lua.create_string(t.as_bytes())?)?;
                                result_table.set(2, "ws")?;
                                Ok(LuaValue::Table(result_table))
                            }
                            Some(Ok(axum::extract::ws::Message::Binary(b))) => {
                                let result_table = lua.create_table()?;
                                result_table.set(1, lua.create_string(b.as_ref())?)?;
                                result_table.set(2, "ws")?;
                                Ok(LuaValue::Table(result_table))
                            }
                            Some(Ok(axum::extract::ws::Message::Close(_)))
                            | None
                            | Some(Err(_)) => Ok(LuaValue::Nil),
                            Some(Ok(axum::extract::ws::Message::Pong(_)))
                            | Some(Ok(axum::extract::ws::Message::Ping(_))) => {
                                // Ignore heartbeats, return nil to indicate no user message
                                Ok(LuaValue::Nil)
                            }
                        }
                    }

                    // Timeout
                    _ = tokio::time::sleep(timeout) => {
                        Ok(LuaValue::Nil)
                    }
                }
            }
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create ws.recv_any: {}", e)))?;
    ws_table
        .set("recv_any", recv_any_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set ws.recv_any: {}", e)))?;

    // ws.close()
    let tx_close = tx.clone();
    let close_fn = lua
        .create_async_function(move |_, (): ()| {
            let tx = tx_close.clone();
            async move {
                let _ = tx.send(axum::extract::ws::Message::Close(None)).await;
                Ok(())
            }
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create ws.close: {}", e)))?;
    ws_table
        .set("close", close_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set ws.close: {}", e)))?;

    // ==================== Channel Operations ====================
    if let Some(cm) = &channel_manager {
        let channel_table = lua.create_table().map_err(|e| {
            DbError::InternalError(format!("Failed to create channel table: {}", e))
        })?;

        // ws.channel.subscribe(channel_name)
        let cm_subscribe = cm.clone();
        let conn_id_sub = conn_id.clone();
        let subscribe_fn = lua
            .create_function(move |_, channel: String| {
                cm_subscribe
                    .subscribe(&conn_id_sub, &channel)
                    .map_err(|e| mlua::Error::RuntimeError(format!("Subscribe error: {}", e)))?;
                Ok(true)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create channel.subscribe: {}", e))
            })?;
        channel_table.set("subscribe", subscribe_fn).map_err(|e| {
            DbError::InternalError(format!("Failed to set channel.subscribe: {}", e))
        })?;

        // ws.channel.unsubscribe(channel_name)
        let cm_unsub = cm.clone();
        let conn_id_unsub = conn_id.clone();
        let unsubscribe_fn = lua
            .create_function(move |_, channel: String| {
                cm_unsub.unsubscribe(&conn_id_unsub, &channel);
                Ok(true)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create channel.unsubscribe: {}", e))
            })?;
        channel_table
            .set("unsubscribe", unsubscribe_fn)
            .map_err(|e| {
                DbError::InternalError(format!("Failed to set channel.unsubscribe: {}", e))
            })?;

        // ws.channel.broadcast(channel_name, data)
        let cm_broadcast = cm.clone();
        let conn_id_bc = conn_id.clone();
        let broadcast_fn = lua
            .create_function(move |_, (channel, data): (String, mlua::Value)| {
                let json_data = lua_value_to_json(&data)?;
                cm_broadcast
                    .broadcast(&channel, json_data, Some(&conn_id_bc))
                    .map_err(|e| mlua::Error::RuntimeError(format!("Broadcast error: {}", e)))?;
                Ok(true)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create channel.broadcast: {}", e))
            })?;
        channel_table.set("broadcast", broadcast_fn).map_err(|e| {
            DbError::InternalError(format!("Failed to set channel.broadcast: {}", e))
        })?;

        // ws.channel.list() -> table of subscribed channels
        let cm_list = cm.clone();
        let conn_id_list = conn_id.clone();
        let list_fn = lua
            .create_function(move |lua, ()| {
                let channels = cm_list.list_subscriptions(&conn_id_list);
                let table = lua.create_table()?;
                for (i, ch) in channels.iter().enumerate() {
                    table.set(i + 1, ch.as_str())?;
                }
                Ok(table)
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create channel.list: {}", e)))?;
        channel_table
            .set("list", list_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set channel.list: {}", e)))?;

        ws_table
            .set("channel", channel_table)
            .map_err(|e| DbError::InternalError(format!("Failed to set ws.channel: {}", e)))?;

        // ==================== Presence Operations ====================
        let presence_table = lua.create_table().map_err(|e| {
            DbError::InternalError(format!("Failed to create presence table: {}", e))
        })?;

        // ws.presence.join(channel, user_info)
        let cm_join = cm.clone();
        let conn_id_join = conn_id.clone();
        let join_fn = lua
            .create_function(move |_, (channel, user_info): (String, mlua::Value)| {
                let json_info = lua_value_to_json(&user_info)?;
                cm_join
                    .presence_join(&conn_id_join, &channel, json_info)
                    .map_err(|e| {
                        mlua::Error::RuntimeError(format!("Presence join error: {}", e))
                    })?;
                Ok(true)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create presence.join: {}", e))
            })?;
        presence_table
            .set("join", join_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set presence.join: {}", e)))?;

        // ws.presence.leave(channel)
        let cm_leave = cm.clone();
        let conn_id_leave = conn_id.clone();
        let leave_fn = lua
            .create_function(move |_, channel: String| {
                cm_leave.presence_leave(&conn_id_leave, &channel);
                Ok(true)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create presence.leave: {}", e))
            })?;
        presence_table
            .set("leave", leave_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set presence.leave: {}", e)))?;

        // ws.presence.list(channel) -> table of users
        let cm_plist = cm.clone();
        let list_presence_fn = lua
            .create_function(move |lua, channel: String| {
                let users = cm_plist.presence_list(&channel);
                let table = lua.create_table()?;
                for (i, user) in users.iter().enumerate() {
                    let user_table = lua.create_table()?;
                    user_table.set("connection_id", user.connection_id.as_str())?;
                    user_table.set("user_info", json_to_lua(lua, &user.user_info)?)?;
                    user_table.set("joined_at", user.joined_at)?;
                    table.set(i + 1, user_table)?;
                }
                Ok(table)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create presence.list: {}", e))
            })?;
        presence_table
            .set("list", list_presence_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set presence.list: {}", e)))?;

        ws_table
            .set("presence", presence_table)
            .map_err(|e| DbError::InternalError(format!("Failed to set ws.presence: {}", e)))?;
    }

    let solidb: mlua::Table = globals
        .get("solidb")
        .map_err(|e| DbError::InternalError(format!("Failed to get solidb table: {}", e)))?;
    solidb
        .set("ws", ws_table)
        .map_err(|e| DbError::InternalError(format!("Failed to set solidb.ws: {}", e)))?;

    // Execute the script
    let chunk = lua.load(&script.code);
    let result = match chunk.eval_async::<LuaValue>().await {
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::error!("WebSocket Lua script error: {}", e);
            // Also try to notify the client of the error if possible
            let _ = tx
                .send(axum::extract::ws::Message::Text(
                    format!("Lua Error: {}", e).into(),
                ))
                .await;
            Err(DbError::InternalError(format!("Lua error: {}", e)))
        }
    };

    // Cleanup
    heartbeat_task.abort();
    result
}
