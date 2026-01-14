-- Pub/Sub Notification System Demo
-- Demonstrates subscribing to multiple channels for notifications
--
-- To use this script:
-- 1. Register: POST /_api/custom/_system/notifications with method="WS" and this code
-- 2. Connect: ws://localhost:6745/api/custom/_system/notifications?user_id=alice
-- 3. Subscribe to channels via client message: {"type": "subscribe", "channel": "alerts"}
-- 4. From another script or API, broadcast: solidb.ws.channel.broadcast("alerts", {msg: "Hello!"})

local user_id = request.query.user_id or request.user and request.user.username or "anonymous"

-- Track subscriptions
local subscriptions = {}

-- Send welcome message
solidb.ws.send(json.encode({
    type = "connected",
    user_id = user_id,
    message = "Ready to receive notifications. Send {type: 'subscribe', channel: 'name'} to subscribe."
}))

-- Main loop
while true do
    local msg, msg_type = solidb.ws.recv_any(30000)

    if msg == nil then
        break
    end

    if msg_type == "ws" then
        local ok, cmd = pcall(json.decode, msg)
        if ok and cmd then
            if cmd.type == "subscribe" and cmd.channel then
                -- Subscribe to a new channel
                if not subscriptions[cmd.channel] then
                    solidb.ws.channel.subscribe(cmd.channel)
                    subscriptions[cmd.channel] = true
                    solidb.ws.send(json.encode({
                        type = "subscribed",
                        channel = cmd.channel
                    }))
                else
                    solidb.ws.send(json.encode({
                        type = "error",
                        message = "Already subscribed to " .. cmd.channel
                    }))
                end

            elseif cmd.type == "unsubscribe" and cmd.channel then
                -- Unsubscribe from a channel
                if subscriptions[cmd.channel] then
                    solidb.ws.channel.unsubscribe(cmd.channel)
                    subscriptions[cmd.channel] = nil
                    solidb.ws.send(json.encode({
                        type = "unsubscribed",
                        channel = cmd.channel
                    }))
                end

            elseif cmd.type == "publish" and cmd.channel and cmd.data then
                -- Publish to a channel (for testing)
                local ok, err = pcall(function()
                    solidb.ws.channel.broadcast(cmd.channel, cmd.data)
                end)
                if ok then
                    solidb.ws.send(json.encode({
                        type = "published",
                        channel = cmd.channel
                    }))
                else
                    solidb.ws.send(json.encode({
                        type = "error",
                        message = "Failed to publish: " .. tostring(err)
                    }))
                end

            elseif cmd.type == "list" then
                -- List current subscriptions
                local channels = solidb.ws.channel.list()
                solidb.ws.send(json.encode({
                    type = "subscriptions",
                    channels = channels
                }))

            elseif cmd.type == "ping" then
                solidb.ws.send(json.encode({type = "pong"}))
            end
        end

    elseif msg_type == "channel" then
        -- Forward channel messages as notifications
        solidb.ws.send(json.encode({
            type = "notification",
            channel = msg.channel,
            data = msg.data,
            timestamp = msg.timestamp,
            sender = msg.sender_id
        }))

    elseif msg_type == "presence" then
        -- Forward presence events if subscribed
        solidb.ws.send(json.encode({
            type = "presence_" .. msg.event_type,
            channel = msg.channel,
            user = msg.user_info
        }))
    end
end
