-- Presence Tracking Demo
-- Simple demonstration of presence tracking without message broadcasting
--
-- To use this script:
-- 1. Register: POST /_api/custom/_system/presence with method="WS" and this code
-- 2. Connect via WebSocket: ws://localhost:6745/api/custom/_system/presence?room=lobby&user_id=alice
-- 3. Open multiple browser tabs to see presence updates

local room = request.query.room or "lobby"
local user_id = request.query.user_id or ("user_" .. math.random(10000))
local user_name = request.query.name or user_id

local user_info = {
    id = user_id,
    name = user_name,
    status = "online",
    joined_at = os.time() * 1000
}

-- Join presence in the room
solidb.ws.presence.join(room, user_info)

-- Send initial presence list
local users = solidb.ws.presence.list(room)
solidb.ws.send(json.encode({
    type = "init",
    room = room,
    users = users,
    your_id = user_id
}))

-- Main loop - just wait for presence events
while true do
    local msg, msg_type = solidb.ws.recv_any(60000)

    if msg == nil then
        -- Timeout or connection closed
        break
    end

    if msg_type == "ws" then
        -- Handle client commands
        local ok, cmd = pcall(json.decode, msg)
        if ok and cmd then
            if cmd.type == "status" and cmd.status then
                -- Update user status
                user_info.status = cmd.status
                -- Re-join with updated info (this broadcasts the update)
                solidb.ws.presence.leave(room)
                solidb.ws.presence.join(room, user_info)
            elseif cmd.type == "list" then
                -- Return current presence list
                solidb.ws.send(json.encode({
                    type = "presence_list",
                    users = solidb.ws.presence.list(room)
                }))
            elseif cmd.type == "ping" then
                solidb.ws.send(json.encode({type = "pong"}))
            end
        end

    elseif msg_type == "presence" then
        -- Forward presence events to client
        solidb.ws.send(json.encode({
            type = "presence_" .. msg.event_type,
            user = msg.user_info,
            room = msg.channel,
            timestamp = msg.timestamp
        }))
    end
end

-- Cleanup is automatic
