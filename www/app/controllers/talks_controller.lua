local Controller = require("controller")
local TalksController = Controller:extend()
local AuthHelper = require("helpers.auth_helper")
local TextHelper = require("helpers.text_helper")
local Channel = require("models.channel")
local Message = require("models.message")
local Signal = require("models.signal")

-- Get current user (middleware ensures user is authenticated)
local function get_current_user()
  return AuthHelper.get_current_user()
end

-- Get all users for user lists
local function get_all_users()
  local result = Sdb:Sdbql([[
    FOR u IN users
    RETURN { _key: u._key, _id: u._id, firstname: u.firstname, lastname: u.lastname, email: u.email, status: u.status, connection_count: u.connection_count }
  ]])
  return (result and result.result) or {}
end

-- Main index page
function TalksController:index()
  local current_user = get_current_user()
  local channel_key = self.params.channel or "general"

  -- Get current channel with access control
  local channel = Channel.find_for_user(channel_key, current_user._key)
  if not channel and channel_key ~= "general" then
    return self:redirect("/talks?channel=general")
  end

  -- For DM channels, get the other user's info (single query)
  local dm_user_name = nil
  local dm_user_key = nil
  if channel then
    local dm_other = channel:dm_other_user(current_user._key)
    if dm_other then
      dm_user_key = dm_other._key
      local name = dm_other.firstname or "User"
      if dm_other.lastname then
        name = name .. " " .. dm_other.lastname
      end
      dm_user_name = name
    end
  end

  -- Load messages (avoid second request via LiveQuery)
  local messages = {}
  if channel then
    messages = channel:messages(100)
  end

  local view_data = {
    current_user = current_user,
    channel = channel,
    messages = messages,
    dm_user_name = dm_user_name,
    dm_user_key = dm_user_key,
    db_name = Sdb._db_config and Sdb._db_config.db_name or "_system",
    TextHelper = TextHelper
  }

  -- HTMX partial update
  if self:is_htmx_request() then
    self.layout = false
    return self:render("talks/index", view_data)
  end

  -- Full page load
  self.layout = "talks"
  self:render("talks/index", view_data)
end

-- Sidebar: Channels list
function TalksController:sidebar_channels()
  local current_user = get_current_user()
  local current_channel = self.params.channel or "general"
  local channels = Channel.for_user(current_user._key)

  self.layout = false
  self:render("talks/_channels_list", {
    channels = channels,
    current_channel = current_channel
  })
end

-- Sidebar: DMs list
function TalksController:sidebar_dms()
  local current_user = get_current_user()
  local current_channel = self.params.channel
  local dm_channels = Channel.dms_for_user(current_user._key)

  -- Enrich DM channels with other user info
  for _, dm in ipairs(dm_channels) do
    dm.other_user = dm:dm_other_user(current_user._key)
  end

  self.layout = false
  self:render("talks/_dm_list", {
    dm_channels = dm_channels,
    current_channel = current_channel,
    current_user = current_user
  })
end

-- Sidebar: Users list
function TalksController:sidebar_users()
  local current_user = get_current_user()

  self.layout = false
  self:render("talks/_users_list", {
    users = get_all_users(),
    current_user = current_user
  })
end

-- Messages for a channel
function TalksController:messages()
  local current_user = get_current_user()
  local channel_key = self.params.channel or "general"

  local channel = Channel.find_for_user(channel_key, current_user._key)
  if not channel then
    return self:html('<p class="text-center text-text-dim py-4">Channel not found</p>')
  end

  local messages = channel:messages(100)

  self.layout = false
  self:render("talks/_messages", {
    messages = messages,
    current_user = current_user,
    TextHelper = TextHelper
  })
end

-- Single message (for LiveQuery updates)
function TalksController:show_message()
  local current_user = get_current_user()
  local msg = Message:find(self.params.key)
  if not msg then return self:html("") end

  msg.data.sender = msg:sender_info()

  self.layout = false
  self:render("talks/_message", {
    msg = msg,
    current_user = current_user,
    TextHelper = TextHelper
  })
end

-- Send message
function TalksController:send_message()
  local current_user = get_current_user()
  local channel_id = self.params.channel_id
  local text = self.params.text

  if not text or text == "" then return self:html("") end

  local msg = Message.send(channel_id, current_user, text)

  self.layout = false
  self:render("talks/_message", {
    msg = msg,
    current_user = current_user,
    TextHelper = TextHelper
  })
end

-- Delete message
function TalksController:delete_message()
  local current_user = get_current_user()
  local msg = Message:find(self.params.key)
  if msg then
    msg:delete_by_user(current_user._key)
  end

  return self:html("")
end

-- Toggle reaction
function TalksController:toggle_reaction()
  local current_user = get_current_user()
  local message_key = self.params.message_key
  local emoji = self.params.emoji

  if not message_key or not emoji then return self:html("") end

  local msg = Message:find(message_key)
  if not msg then return self:html("") end

  msg:toggle_reaction(current_user._key, emoji)
  msg.data.sender = msg:sender_info()

  self.layout = false
  self:render("talks/_message", {
    msg = msg,
    current_user = current_user,
    TextHelper = TextHelper
  })
end

-- Emoji picker
function TalksController:emoji_picker()
  self.layout = false
  self:render("talks/_emoji_picker", {
    message_key = self.params.key
  })
end

-- Thread view
function TalksController:thread()
  local current_user = get_current_user()
  local message_key = self.params.message_id

  local parent = Message:find(message_key)
  if not parent then
    return self:html('<p class="text-center text-text-dim py-4">Message not found</p>')
  end

  parent.data.sender = parent:sender_info()
  local replies = parent:replies()

  self.layout = false
  self:render("talks/_thread_panel", {
    parent_message = parent,
    replies = replies,
    current_user = current_user,
    TextHelper = TextHelper
  })
end

-- Thread reply
function TalksController:thread_reply()
  local current_user = get_current_user()
  local parent_key = self.params.message_id
  local text = self.params.text

  if not text or text == "" then
    return self:thread()
  end

  local parent = Message:find(parent_key)
  if not parent then return self:thread() end

  parent:create_reply(current_user, text)

  return self:thread()
end

-- Channel modal
function TalksController:channel_modal()
  local current_user = get_current_user()

  self.layout = false
  self:render("talks/_channel_modal", {
    current_user = current_user
  })
end

-- User selector for private channels
function TalksController:channel_users()
  local current_user = get_current_user()
  local result = Sdb:Sdbql("FOR u IN users RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname, email: u.email }")

  self.layout = false
  self:render("talks/_user_selector", {
    users = result and result.result or {},
    current_user = current_user
  })
end

-- Create channel
function TalksController:create_channel()
  local current_user = get_current_user()
  local name = self.params.name
  local channel_type = self.params.type or "standard"
  local members = self.params.members

  if not name or name == "" then
    return self:sidebar_channels()
  end

  -- Normalize name
  name = Channel.normalize_name(name)

  local channel_data = {
    name = name,
    type = channel_type,
    created_by = current_user._key,
    created_at = os.time()
  }

  -- Add members for private channels
  if channel_type == "private" then
    local member_list = { current_user._key }
    if type(members) == "table" then
      for _, m in ipairs(members) do
        table.insert(member_list, m)
      end
    elseif type(members) == "string" and members ~= "" then
      table.insert(member_list, members)
    end
    channel_data.members = member_list
  end

  Channel:create(channel_data)

  return self:sidebar_channels()
end

-- Group modal
function TalksController:group_modal()
  local current_user = get_current_user()

  self.layout = false
  self:render("talks/_group_modal", {
    current_user = current_user
  })
end

-- Create group
function TalksController:create_group()
  local current_user = get_current_user()
  local name = self.params.name
  local members = self.params.members

  if not name or name == "" then
    return self:sidebar_channels()
  end

  -- Normalize name
  name = Channel.normalize_name(name)

  -- Build member list (always include creator)
  local member_list = { current_user._key }
  if type(members) == "table" then
    for _, m in ipairs(members) do
      if m ~= current_user._key then
        table.insert(member_list, m)
      end
    end
  elseif type(members) == "string" and members ~= "" and members ~= current_user._key then
    table.insert(member_list, members)
  end

  local new_channel = Channel:create({
    name = name,
    type = Channel.TYPES.PRIVATE,
    members = member_list,
    created_by = current_user._key,
    created_at = os.time()
  })

  if new_channel and new_channel._key and self:is_htmx_request() then
    self:set_header("HX-Redirect", "/talks?channel=" .. new_channel._key)
    return self:html("")
  end

  return self:sidebar_channels()
end

-- Start DM
function TalksController:dm_start()
  local current_user = get_current_user()
  local other_user_key = self.params.user_key

  if not other_user_key or other_user_key == current_user._key then
    return self:redirect("/talks")
  end

  local dm_channel = Channel.find_or_create_dm(current_user._key, other_user_key)

  if dm_channel then
    local channel_key = dm_channel._key or dm_channel.data._key
    if self:is_htmx_request() then
      self:set_header("HX-Redirect", "/talks?channel=" .. channel_key)
      return self:html("")
    end
    return self:redirect("/talks?channel=" .. channel_key)
  end

  return self:redirect("/talks")
end

-- LiveQuery token
function TalksController:livequery_token()
  local token = Sdb:LiveQueryToken()

  self:json({
    token = token,
    expires_in = 30
  })
end

-- Get user info (for call notifications)
function TalksController:get_user()
  local key = self.params.key
  local result = Sdb:Sdbql([[
    FOR u IN users
    FILTER u._key == @key
    LIMIT 1
    RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname, status: u.status }
  ]], { key = key })

  if result and result.result and result.result[1] then
    self:json(result.result[1])
  else
    self:json({ error = "User not found" }, 404)
  end
end

-- File proxy
function TalksController:file()
  local key = self.params.key
  local db_name = Sdb._db_config and Sdb._db_config.db_name or "_system"
  local db_url = Sdb._db_config and Sdb._db_config.url or "http://localhost:6745"

  -- Get token for blob access
  local token = Sdb:LiveQueryToken()

  -- Redirect to blob URL
  local blob_url = db_url .. "/_api/blob/" .. db_name .. "/files/" .. key
  return self:redirect(blob_url .. "?token=" .. token)
end

-- Call UI
function TalksController:call_ui()
  local current_user = get_current_user()
  local channel_key = self.params.channel_key
  local call_type = self.params.type or "audio"

  local channel = Channel.find_for_user(channel_key, current_user._key)
  if not channel then return self:html("") end

  -- For DM channels, get the other user's name for display
  local display_name = channel.name
  if channel.type == Channel.TYPES.DM then
    local dm_other = channel:dm_other_user(current_user._key)
    if dm_other then
      display_name = dm_other.firstname or "User"
      if dm_other.lastname then
        display_name = display_name .. " " .. dm_other.lastname
      end
    end
  end

  self.layout = false
  self:render("talks/_call_ui", {
    channel = channel,
    call_type = call_type,
    current_user = current_user,
    display_name = display_name
  })
end

-- Join call
function TalksController:call_join()
  local current_user = get_current_user()
  local channel = Channel:find(self.params.channel_key)
  local result = nil
  if channel then
    result = channel:join_call(current_user._key)
  end

  self:json({
    success = true,
    channel_found = channel ~= nil,
    result = result
  })
end

-- Leave call
function TalksController:call_leave()
  local current_user = get_current_user()
  local channel = Channel:find(self.params.channel_key)
  if channel then
    channel:leave_call(current_user._key)
  end

  self:json({ success = true })
end

-- Get active call participants
function TalksController:call_participants()
  local current_user = get_current_user()
  local channel = Channel:find(self.params.channel_key)
  if not channel then
    return self:json({ error = "Channel not found" }, 404)
  end

  local participants = channel.active_call_participants or {}

  -- Get user info for each participant
  local users = {}
  for _, user_key in ipairs(participants) do
    if user_key ~= current_user._key then
      local result = Sdb:Sdbql([[
        FOR u IN users
        FILTER u._key == @key
        LIMIT 1
        RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname }
      ]], { key = user_key })
      if result and result.result and result.result[1] then
        table.insert(users, result.result[1])
      end
    end
  end

  self:json({ participants = users })
end

-- Decline call (receiver declines incoming call)
function TalksController:call_decline()
  local current_user = get_current_user()
  local body = GetBody()
  local data = body and DecodeJson(body) or self.params

  local debug = {
    caller_key = data.caller_key,
    channel_key = data.channel_key,
    from_user = current_user._key
  }

  -- Send decline signal to the caller
  if data.caller_key then
    local signal_result = Signal.send(
      current_user._key,
      data.caller_key,
      "decline",
      { declined_by = current_user._key, declined_by_name = current_user.firstname },
      data.channel_key
    )
    debug.signal_result = signal_result
  end

  -- Also remove caller from active_call_participants since call is declined
  if data.channel_key then
    local channel = Channel:find(data.channel_key)
    if channel and data.caller_key then
      channel:leave_call(data.caller_key)
      debug.leave_call = true
    end
  end

  self:json({ success = true, debug = debug })
end

-- Send signal
function TalksController:call_signal()
  local current_user = get_current_user()

  -- Parse JSON body
  local body = GetBody()
  local data = body and DecodeJson(body) or self.params

  Signal.send(
    current_user._key,
    data.to_user,
    data.type,
    data.data,
    data.channel_id
  )

  self:json({ success = true })
end

-- Delete signal (after processing)
function TalksController:call_signal_delete()
  Signal.delete_by_key(self.params.key)

  self:json({ success = true })
end

-- Setup presence script
function TalksController:setup_presence()
  local script = [[
-- Presence tracking script
local user_id = params.user_id
if not user_id then return { error = "Missing user_id" } end

-- Increment connection count on connect
db:Sdbql("UPDATE " .. user_id .. " WITH { connection_count: (OLD.connection_count || 0) + 1, status: 'online' } IN users")

-- Return cleanup function
return {
  on_disconnect = function()
    db:Sdbql("UPDATE " .. user_id .. " WITH { connection_count: MAX(0, (OLD.connection_count || 1) - 1) } IN users")
    -- Set offline if no connections
    db:Sdbql("FOR u IN users FILTER u._key == '" .. user_id .. "' AND u.connection_count <= 0 UPDATE u WITH { status: 'offline' } IN users")
  end
}
]]

  Sdb:Sdbql("UPSERT { _key: 'presence' } INSERT { _key: 'presence', script: @script, type: 'websocket' } UPDATE { script: @script } IN _scripts", { script = script })

  self:json({ success = true })
end

return TalksController
