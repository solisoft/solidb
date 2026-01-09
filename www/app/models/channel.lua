local Model = require("model")

local Channel = Model.create("channels", {
  permitted_fields = { "name", "type", "members", "created_by", "active_call_participants" },
  validations = {
    name = { presence = true, length = { between = {1, 50} } }
  }
})

-- Channel types
Channel.TYPES = {
  SYSTEM = "system",
  STANDARD = "standard",
  PRIVATE = "private",
  DM = "dm"
}

-- Find channel by key or name
function Channel.find_by_key_or_name(key_or_name)
  local result = Sdb:Sdbql(
    "FOR c IN channels FILTER c._key == @key OR c.name == @key LIMIT 1 RETURN c",
    { key = key_or_name }
  )
  if result and result.result and result.result[1] then
    return Channel:new(result.result[1])
  end
  return nil
end

-- Check if user has access to this channel
function Channel:user_has_access(user_key)
  local channel_type = self.type or self.data.type
  local members = self.members or self.data.members

  -- Public channels are accessible to all
  if channel_type == Channel.TYPES.SYSTEM or channel_type == Channel.TYPES.STANDARD then
    return true
  end

  -- Private/DM channels require membership
  if members then
    for _, m in ipairs(members) do
      if m == user_key then
        return true
      end
    end
  end

  return false
end

-- Get channel with access control
function Channel.find_for_user(key_or_name, user_key)
  local channel = Channel.find_by_key_or_name(key_or_name)
  if not channel then return nil end

  if not channel:user_has_access(user_key) then
    return nil
  end

  return channel
end

-- Get messages for this channel (non-threaded)
function Channel:messages(limit)
  limit = limit or 100
  local Message = require("models.message")
  -- Use _id (full document ID like "channels/general") for consistency with stored messages
  local channel_id = self._id or self.data._id
  return Message.for_channel(channel_id, limit)
end

-- Get the other user in a DM channel
function Channel:dm_other_user(current_user_key)
  local channel_type = self.type or self.data.type
  local members = self.members or self.data.members

  if channel_type ~= Channel.TYPES.DM or not members then
    return nil
  end

  for _, m in ipairs(members) do
    if m ~= current_user_key then
      local result = Sdb:Sdbql(
        "FOR u IN users FILTER u._key == @key LIMIT 1 RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname }",
        { key = m }
      )
      if result and result.result and result.result[1] then
        return result.result[1]
      end
    end
  end

  return nil
end

-- Get DM user display name
function Channel:dm_user_name(current_user_key)
  local other = self:dm_other_user(current_user_key)
  if not other then return nil end

  local name = other.firstname or "User"
  if other.lastname then
    name = name .. " " .. other.lastname
  end
  return name
end

-- Find standard and private channels for user
function Channel.for_user(user_key)
  local result = Sdb:Sdbql([[
    FOR c IN channels
    FILTER c.type == 'standard' OR c.type == 'system' OR (c.type == 'private' AND @me IN c.members)
    SORT c.type DESC, c.name ASC
    RETURN c
  ]], { me = user_key })

  local channels = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(channels, Channel:new(doc))
    end
  end
  return channels
end

-- Find DM channels for user
function Channel.dms_for_user(user_key)
  local result = Sdb:Sdbql([[
    FOR c IN channels
    FILTER c.type == 'dm' AND POSITION(c.members, @me)
    
    -- Find the other user
    LET other_key = (
      FOR m IN c.members 
      FILTER m != @me 
      LIMIT 1 
      RETURN m
    )[0]
    
    -- Get other user details
    LET other_user = (
      FOR u IN users
      FILTER u._key == other_key
      RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname }
    )[0]
    
    RETURN MERGE(c, { other_user: other_user })
  ]], { me = user_key })

  local channels = {}
  if result and result.result then
    -- Sort by name (which usually isn't great for DMs) or potentially recent activity if we had it
    -- For now, let's sort by other user's name in Lua or just return as is
    for _, doc in ipairs(result.result) do
      local channel = Channel:new(doc)
      -- Store the joined data
      channel.other_user = doc.other_user
      table.insert(channels, channel)
    end
    -- Sort by other user's name
    table.sort(channels, function(a, b)
      local name_a = a.other_user and a.other_user.firstname or ""
      local name_b = b.other_user and b.other_user.firstname or ""
      return name_a < name_b
    end)
  end
  return channels
end

-- Find existing DM channel between two users
function Channel.find_dm(user1_key, user2_key)
  local result = Sdb:Sdbql([[
    FOR c IN channels
    FILTER c.type == 'dm'
      AND POSITION(c.members, @user1) >= 0
      AND POSITION(c.members, @user2) >= 0
    LIMIT 1
    RETURN c
  ]], { user1 = user1_key, user2 = user2_key })

  if result and result.result and result.result[1] then
    return Channel:new(result.result[1])
  end
  return nil
end

-- Create or get existing DM channel
function Channel.find_or_create_dm(user1_key, user2_key)
  local existing = Channel.find_dm(user1_key, user2_key)
  if existing then return existing end

  return Channel:create({
    name = "dm_" .. user1_key .. "_" .. user2_key,
    type = Channel.TYPES.DM,
    members = { user1_key, user2_key },
    created_at = os.time()
  })
end

-- Join call
function Channel:join_call(user_key)
  local key = self._key or self.data._key

  -- Get current participants using parameterized query
  local current = Sdb:Sdbql(
    "FOR c IN channels FILTER c._key == @key RETURN c.active_call_participants",
    { key = key }
  )

  local participants = {}
  if current and current.result and current.result[1] then
    for _, p in ipairs(current.result[1]) do
      if p ~= user_key then
        table.insert(participants, p)
      end
    end
  end
  table.insert(participants, user_key)

  -- Use parameterized update with JSON-encoded array
  local result = Sdb:Sdbql(
    "FOR c IN channels FILTER c._key == @key UPDATE c WITH { active_call_participants: @participants } IN channels RETURN NEW.active_call_participants",
    { key = key, participants = participants }
  )

  return result
end

-- Leave call
function Channel:leave_call(user_key)
  local key = self._key or self.data._key

  -- Get current participants using parameterized query
  local current = Sdb:Sdbql(
    "FOR c IN channels FILTER c._key == @key RETURN c.active_call_participants",
    { key = key }
  )

  local participants = {}
  if current and current.result and current.result[1] then
    for _, p in ipairs(current.result[1]) do
      if p ~= user_key then
        table.insert(participants, p)
      end
    end
  end

  -- Use parameterized update
  Sdb:Sdbql(
    "FOR c IN channels FILTER c._key == @key UPDATE c WITH { active_call_participants: @participants } IN channels",
    { key = key, participants = participants }
  )
end

-- Normalize channel name
function Channel.normalize_name(name)
  return name:lower():gsub("[^a-z0-9-]", "-"):gsub("-+", "-"):gsub("^-", ""):gsub("-$", "")
end

return Channel
