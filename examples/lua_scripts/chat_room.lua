-- Chat Room WebSocket Handler
-- Demonstrates channel subscriptions and presence tracking
--
-- To use this script:
-- 1. Register the script: POST /_api/custom/_system/chatroom with method="WS" and this code
-- 2. Connect via WebSocket: ws://localhost:6745/api/custom/_system/chatroom?room=general&name=Alice
-- 3. Send messages: {"type": "message", "text": "Hello everyone!"}
-- 4. Watch for presence events and broadcasts from other users

local room_id = request.query.room or "general"
local channel = "chat:" .. room_id

-- Get user info from request
local user_info = {
    user_id = request.user and request.user.username or ("anon_" .. math.random(10000)),
    name = request.query.name or "Anonymous",
    joined_at = os.time() * 1000
}

-- Subscribe to the channel and join presence
solidb.ws.channel.subscribe(channel)
solidb.ws.presence.join(channel, user_info)

-- Send current presence state to the new user
local current_users = solidb.ws.presence.list(channel)
solidb.ws.send(json.encode({
    type = "presence_state",
    channel = channel,
    users = current_users,
    you = user_info
}))

-- Notify the user they've connected
solidb.ws.send(json.encode({
    type = "connected",
    message = "Welcome to " .. room_id .. "!",
    user_count = #current_users
}))

-- Main message loop
while true do
    local msg, msg_type = solidb.ws.recv_any(30000)

    if msg == nil then
        -- Connection closed or timeout with no message
        break
    end

    if msg_type == "ws" then
        -- Client sent a message
        local ok, parsed = pcall(json.decode, msg)
        if ok and parsed then
            if parsed.type == "message" and parsed.text then
                -- Broadcast message to all users in the room
                solidb.ws.channel.broadcast(channel, {
                    type = "chat_message",
                    from = user_info,
                    text = parsed.text,
                    timestamp = os.time() * 1000
                })
            elseif parsed.type == "typing" then
                -- Broadcast typing indicator
                solidb.ws.channel.broadcast(channel, {
                    type = "typing",
                    from = user_info
                })
            elseif parsed.type == "get_users" then
                -- Return current user list
                solidb.ws.send(json.encode({
                    type = "user_list",
                    users = solidb.ws.presence.list(channel)
                }))
            end
        else
            -- Invalid JSON, send error
            solidb.ws.send(json.encode({
                type = "error",
                message = "Invalid JSON message"
            }))
        end

    elseif msg_type == "channel" then
        -- Message from another user in the channel
        solidb.ws.send(json.encode(msg.data))

    elseif msg_type == "presence" then
        -- Someone joined or left
        solidb.ws.send(json.encode({
            type = "presence_" .. msg.event_type,
            channel = msg.channel,
            user = msg.user_info,
            timestamp = msg.timestamp
        }))
    end
end

-- Connection closed, cleanup happens automatically via ConnectionGuard
