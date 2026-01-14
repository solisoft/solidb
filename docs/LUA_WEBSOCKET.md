# Lua WebSocket API Reference

This document describes the WebSocket API available in Lua scripts for building real-time applications with SoliDB.

## Overview

Lua scripts can handle WebSocket connections by registering with the `WS` HTTP method. The WebSocket API provides:

- **Basic WebSocket operations**: Send, receive, close
- **Channel pub/sub**: Subscribe to channels and broadcast messages
- **Presence tracking**: Track who's connected to each channel

## Table of Contents

- [Basic WebSocket](#basic-websocket)
- [Channel Operations](#channel-operations)
- [Presence Tracking](#presence-tracking)
- [Event Handling](#event-handling)
- [Examples](#examples)

---

## Basic WebSocket

### `solidb.ws.send(data)`

Send a message to the connected client.

```lua
solidb.ws.send(json.encode({type = "hello", message = "Welcome!"}))
```

**Parameters:**
- `data` (string): The message to send (usually JSON)

**Returns:** Nothing

---

### `solidb.ws.recv()`

Wait for and receive a message from the client.

```lua
local msg = solidb.ws.recv()
if msg then
    local data = json.decode(msg)
    -- process data
end
```

**Returns:**
- `string`: The received message
- `nil`: If the connection is closed or an error occurred

---

### `solidb.ws.recv_any(timeout_ms)`

Wait for a message from either the WebSocket client OR channel/presence events.

```lua
local msg, msg_type = solidb.ws.recv_any(5000)

if msg == nil then
    -- Timeout or connection closed
elseif msg_type == "ws" then
    -- Client WebSocket message (string)
    local data = json.decode(msg)
elseif msg_type == "channel" then
    -- Channel broadcast message (table)
    -- msg.channel, msg.data, msg.timestamp, msg.sender_id
elseif msg_type == "presence" then
    -- Presence change event (table)
    -- msg.event_type ("join" or "leave"), msg.channel, msg.user_info, msg.connection_id
end
```

**Parameters:**
- `timeout_ms` (number, optional): Maximum wait time in milliseconds. Default: 30000

**Returns:**
- `msg` (string|table|nil): The message or event
- `msg_type` (string|nil): One of "ws", "channel", "presence", or nil if timeout/closed

---

### `solidb.ws.close()`

Close the WebSocket connection gracefully.

```lua
solidb.ws.close()
```

---

## Channel Operations

Channels provide pub/sub messaging between WebSocket connections. Any connection can broadcast to a channel, and all subscribers receive the message.

### `solidb.ws.channel.subscribe(channel_name)`

Subscribe to a channel to receive broadcast messages.

```lua
solidb.ws.channel.subscribe("chat:room-123")
solidb.ws.channel.subscribe("notifications:user-456")
```

**Parameters:**
- `channel_name` (string): The channel name (can be any string)

**Returns:** `true` on success

---

### `solidb.ws.channel.unsubscribe(channel_name)`

Unsubscribe from a channel.

```lua
solidb.ws.channel.unsubscribe("chat:room-123")
```

**Parameters:**
- `channel_name` (string): The channel to unsubscribe from

**Returns:** `true`

---

### `solidb.ws.channel.broadcast(channel_name, data)`

Send a message to all subscribers of a channel.

```lua
solidb.ws.channel.broadcast("chat:room-123", {
    type = "message",
    from = "alice",
    text = "Hello everyone!"
})
```

**Parameters:**
- `channel_name` (string): The target channel
- `data` (table): The data to broadcast (will be JSON-serialized)

**Returns:** `true` on success

**Note:** The sender also receives the broadcast through `recv_any()`.

---

### `solidb.ws.channel.list()`

Get a list of channels this connection is subscribed to.

```lua
local channels = solidb.ws.channel.list()
for _, ch in ipairs(channels) do
    print("Subscribed to: " .. ch)
end
```

**Returns:** Array of channel name strings

---

## Presence Tracking

Presence allows tracking which users are connected to a channel, enabling "who's online" features.

### `solidb.ws.presence.join(channel_name, user_info)`

Join a channel's presence list with user metadata.

```lua
solidb.ws.presence.join("chat:room-123", {
    user_id = "alice",
    name = "Alice",
    avatar = "https://example.com/alice.png",
    status = "online"
})
```

**Parameters:**
- `channel_name` (string): The channel to join
- `user_info` (table): Arbitrary user metadata

**Returns:** `true` on success

**Note:** Other connections subscribed to this channel will receive a presence "join" event.

---

### `solidb.ws.presence.leave(channel_name)`

Leave a channel's presence list.

```lua
solidb.ws.presence.leave("chat:room-123")
```

**Parameters:**
- `channel_name` (string): The channel to leave

**Returns:** `true`

**Note:** Other connections will receive a presence "leave" event.

---

### `solidb.ws.presence.list(channel_name)`

Get a list of all users currently present in a channel.

```lua
local users = solidb.ws.presence.list("chat:room-123")
for _, user in ipairs(users) do
    print(user.user_info.name .. " joined at " .. user.joined_at)
end
```

**Returns:** Array of presence records:
```lua
{
    {
        connection_id = "uuid-string",
        user_info = { ... },  -- The data passed to presence.join()
        joined_at = 1705123456789  -- Unix timestamp in milliseconds
    },
    ...
}
```

---

## Event Handling

When using `recv_any()`, you receive different event types:

### WebSocket Messages (`msg_type == "ws"`)

Direct messages from the connected client.

```lua
local msg, msg_type = solidb.ws.recv_any()
if msg_type == "ws" then
    -- msg is a string (the raw WebSocket message)
    local data = json.decode(msg)
end
```

### Channel Messages (`msg_type == "channel"`)

Broadcast messages from other connections.

```lua
if msg_type == "channel" then
    -- msg is a table:
    -- msg.channel    - The channel name
    -- msg.data       - The broadcast data (table)
    -- msg.timestamp  - Unix timestamp in milliseconds
    -- msg.sender_id  - Connection ID of the sender (may be nil)

    solidb.ws.send(json.encode({
        type = "broadcast",
        from_channel = msg.channel,
        payload = msg.data
    }))
end
```

### Presence Events (`msg_type == "presence"`)

Notifications when users join or leave.

```lua
if msg_type == "presence" then
    -- msg is a table:
    -- msg.event_type    - "join" or "leave"
    -- msg.channel       - The channel name
    -- msg.user_info     - The user's metadata (from presence.join)
    -- msg.connection_id - The connection's unique ID
    -- msg.timestamp     - Unix timestamp in milliseconds

    if msg.event_type == "join" then
        print(msg.user_info.name .. " joined " .. msg.channel)
    else
        print(msg.user_info.name .. " left " .. msg.channel)
    end
end
```

---

## Examples

### Simple Chat Room

```lua
-- Register: POST /_api/custom/_system/chat with method="WS"
-- Connect: ws://localhost:6745/api/custom/_system/chat?room=general&name=Alice

local room = request.query.room or "general"
local channel = "chat:" .. room
local user = {
    name = request.query.name or "Anonymous",
    joined = os.time() * 1000
}

-- Join the room
solidb.ws.channel.subscribe(channel)
solidb.ws.presence.join(channel, user)

-- Send current users
solidb.ws.send(json.encode({
    type = "init",
    users = solidb.ws.presence.list(channel)
}))

-- Message loop
while true do
    local msg, msg_type = solidb.ws.recv_any(30000)
    if msg == nil then break end

    if msg_type == "ws" then
        local data = json.decode(msg)
        if data.type == "message" then
            solidb.ws.channel.broadcast(channel, {
                type = "chat",
                from = user.name,
                text = data.text
            })
        end
    elseif msg_type == "channel" then
        solidb.ws.send(json.encode(msg.data))
    elseif msg_type == "presence" then
        solidb.ws.send(json.encode({
            type = "presence_" .. msg.event_type,
            user = msg.user_info
        }))
    end
end
```

### Multi-Channel Notifications

```lua
-- Subscribe to multiple channels
solidb.ws.channel.subscribe("user:" .. user_id)
solidb.ws.channel.subscribe("team:" .. team_id)
solidb.ws.channel.subscribe("global:announcements")

while true do
    local msg, msg_type = solidb.ws.recv_any()
    if msg == nil then break end

    if msg_type == "channel" then
        solidb.ws.send(json.encode({
            type = "notification",
            channel = msg.channel,
            data = msg.data
        }))
    end
end
```

### Typing Indicator with Presence

```lua
local channel = "doc:" .. doc_id

solidb.ws.channel.subscribe(channel)
solidb.ws.presence.join(channel, {
    user_id = user_id,
    cursor = {line = 0, col = 0}
})

while true do
    local msg, msg_type = solidb.ws.recv_any(100)

    if msg_type == "ws" then
        local data = json.decode(msg)
        if data.type == "cursor" then
            -- Broadcast cursor position to others
            solidb.ws.channel.broadcast(channel, {
                type = "cursor_move",
                user_id = user_id,
                position = data.position
            })
        end
    elseif msg_type == "channel" then
        solidb.ws.send(json.encode(msg.data))
    elseif msg_type == "presence" then
        solidb.ws.send(json.encode({
            type = "user_" .. msg.event_type,
            user = msg.user_info
        }))
    end
end
```

---

## Automatic Cleanup

When a WebSocket connection closes (client disconnect, error, or script completion), the following cleanup happens automatically:

1. The connection is unsubscribed from all channels
2. The connection leaves all presence channels (triggering "leave" events)
3. Empty channels are garbage collected

You don't need to manually clean up - just let your script end or handle the `nil` return from `recv_any()`.

---

## Best Practices

1. **Use `recv_any()` for real-time apps**: It handles both client messages and server-side events in one call.

2. **Structure channel names**: Use prefixes like `chat:`, `user:`, `team:` to organize channels.

3. **Keep presence data minimal**: Only include essential user info to reduce memory usage.

4. **Handle timeouts gracefully**: Use the timeout parameter in `recv_any()` to perform periodic tasks.

5. **Send JSON**: Always use `json.encode()` for outgoing messages and `json.decode()` for incoming ones.

---

## See Also

- [Example Scripts](../examples/lua_scripts/)
  - `chat_room.lua` - Full chat room implementation
  - `presence_demo.lua` - Simple presence tracking
  - `pubsub_demo.lua` - Multi-channel pub/sub system
