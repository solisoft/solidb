local Model = require("model")

local Message = Model.create("messages", {
  permitted_fields = { "channel_id", "text", "user_key", "sender_name", "timestamp", "reactions", "parent_id", "thread_count", "last_reply_at" },
  validations = {
    channel_id = { presence = true },
    text = { presence = true }
  }
})

-- Get sender info
function Message:sender_info()
  -- Check cache first
  if self.data.sender and next(self.data.sender) then
    return self.data.sender
  end

  local user_key = self.user_key or self.data.user_key
  if not user_key then return {} end

  local result = Sdb:Sdbql(
    "FOR u IN users FILTER u._key == @key LIMIT 1 RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname }",
    { key = user_key }
  )
  if result and result.result and result.result[1] then
    self.data.sender = result.result[1] -- Cache result
    return result.result[1]
  end
  return {}
end

-- Get the database-prefixed channel ID
local function get_full_channel_id(channel_id)
  if not channel_id then return nil end

  local channel_id_str = tostring(channel_id)
  local db_name = Sdb._db_config and Sdb._db_config.db_name or "_system"

  -- If already has db prefix, return as-is
  if channel_id_str:match("^" .. db_name .. ":") then
    return channel_id_str
  end

  -- Ensure it has channels/ prefix
  if not channel_id_str:match("^channels/") then
    channel_id_str = "channels/" .. channel_id_str
  end

  -- Add database prefix
  return db_name .. ":" .. channel_id_str
end

-- Bulk fetch sender info for a list of messages
local function bulk_fetch_senders(messages)
  if #messages == 0 then return end

  local user_keys = {}
  local unique_keys = {}
  
  for _, msg in ipairs(messages) do
    local key = msg.user_key or msg.data.user_key
    if key and not unique_keys[key] then
      unique_keys[key] = true
      table.insert(user_keys, key)
    end
  end

  if #user_keys == 0 then return end

  local result = Sdb:Sdbql([[
    FOR u IN users
    FILTER u._key IN @keys
    RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname }
  ]], { keys = user_keys })

  local user_map = {}
  if result and result.result then
    for _, u in ipairs(result.result) do
      user_map[u._key] = u
    end
  end

  for _, msg in ipairs(messages) do
    local key = msg.user_key or msg.data.user_key
    msg.data.sender = user_map[key] or {}
  end
end

-- Get messages for a channel (non-threaded, chronological)
function Message.for_channel(channel_id, limit)
  limit = limit or 100

  local channel_id_full = get_full_channel_id(channel_id)
  if not channel_id_full then return {} end

  local result = Sdb:Sdbql([[
    FOR m IN messages
    FILTER m.channel_id == @channel_id AND (m.parent_id == null OR m.parent_id == "")
    SORT m.timestamp DESC
    LIMIT @limit
    RETURN m
  ]], { channel_id = channel_id_full, limit = limit })

  local messages = {}
  if result and result.result then
    -- Reverse to get chronological order (oldest first)
    for i = #result.result, 1, -1 do
      local msg = Message:new(result.result[i])
      table.insert(messages, msg)
    end
    -- Bulk fetch senders
    bulk_fetch_senders(messages)
  end
  return messages
end

-- Get thread replies
function Message:replies()
  local message_key = self._key or self.data._key
  local parent_id = "messages/" .. message_key

  local result = Sdb:Sdbql([[
    FOR m IN messages
    FILTER m.parent_id == @parent_id
    SORT m.timestamp ASC
    RETURN m
  ]], { parent_id = parent_id })

  local replies = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      local reply = Message:new(doc)
      table.insert(replies, reply)
    end
    -- Bulk fetch senders
    bulk_fetch_senders(replies)
  end
  return replies
end

-- Create reply to this message
function Message:create_reply(user, text)
  local message_key = self._key or self.data._key
  local channel_id = self.channel_id or self.data.channel_id
  -- Ensure channel_id has db prefix
  local channel_id_full = get_full_channel_id(channel_id)

  local reply = Message:create({
    channel_id = channel_id_full,
    parent_id = "messages/" .. message_key,
    text = text,
    user_key = user._key,
    sender_name = user.firstname or user.email,
    timestamp = os.time(),
    reactions = {}
  })

  -- Update thread count on parent
  local thread_count = (self.thread_count or self.data.thread_count or 0) + 1
  self:update({
    thread_count = thread_count,
    last_reply_at = os.time()
  })

  return reply
end

-- Toggle reaction
function Message:toggle_reaction(user_key, emoji)
  local reactions = self.reactions or self.data.reactions or {}
  local users = reactions[emoji] or {}

  local found = false
  local new_users = {}
  for _, u in ipairs(users) do
    if u == user_key then
      found = true
    else
      table.insert(new_users, u)
    end
  end

  if not found then
    table.insert(new_users, user_key)
  end

  reactions[emoji] = #new_users > 0 and new_users or nil

  self:update({ reactions = reactions })
  self.data.reactions = reactions
end

-- Delete message (only by owner)
function Message:delete_by_user(user_key)
  local msg_user_key = self.user_key or self.data.user_key
  if msg_user_key ~= user_key then
    return false
  end
  self:destroy()
  return true
end

-- Send a new message to a channel
function Message.send(channel_id, user, text)
  -- Ensure channel_id is in full format with db prefix
  local channel_id_full = get_full_channel_id(channel_id)

  local msg = Message:create({
    channel_id = channel_id_full,
    text = text,
    user_key = user._key,
    sender_name = user.firstname or user.email,
    timestamp = os.time(),
    reactions = {}
  })

  msg.data.sender = { firstname = user.firstname, lastname = user.lastname }
  return msg
end

return Message
