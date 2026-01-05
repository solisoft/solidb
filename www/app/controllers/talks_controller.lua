local Controller = require("controller")
local TalksController = Controller:extend()
local SoliDB = require("solidb")
local argon2 = require("argon2")

-- Helper to get current user from session
local function get_current_user(start_session)
  if not HasSession() then return nil end
  local session = GetSession()
  if not session.user_id then return nil end

  -- If we have Sdb global, use it, otherwise create new instance (inefficient but safe)
  local db = _G.Sdb or SoliDB.new({ driver = "redbean" }) -- Fallback, assuming config logic handled elsewhere

  -- Ideally we cache the user in request, but for now fetch it
  -- We use Sdbql to get the user
  local res = db:Sdbql("FOR u IN users FILTER u._id == @id RETURN u", { id = session.user_id })
  if res and res.result and #res.result > 0 then
    return res.result[1]
  end
  return nil
end

-- Helper to find or create a channel
local function get_or_create_channel(db, current_user, channel_identifier)
  -- Logic ported from www
  -- Try to find channel by key first, then by name
  local channelQuery = "FOR c IN channels FILTER c._key == @key OR c.name == @name RETURN c"
  local channelRes = db:Sdbql(channelQuery, { key = channel_identifier, name = channel_identifier })
  local channel = channelRes and channelRes.result[1]

  -- Access Control for Private Channels
  if channel and channel.type == "private" then
    local isMember = false
    if channel.members then
      for _, m in ipairs(channel.members) do
        if m == current_user._key then isMember = true break end
      end
    end
    if not isMember then return nil end
  end

  -- Lazy create DM channel
  if not channel and string.sub(channel_identifier, 1, 3) == "dm_" then
      -- DM creation logic omitted for brevity in first pass, but essential for DMs.
      -- Simplified: if it starts with dm_, we might need to create it.
      -- For now, let's assume valid DMs are created via create_dm action.
      return nil
  end

  -- Access Control for DM channels
  if channel and channel.type == "dm" then
    local isMember = false
    if channel.members then
       -- DM members stores user _keys
       for _, m in ipairs(channel.members) do
          if m == current_user._key then isMember = true break end
       end
    end
    if not isMember then return nil end
  end

  return channel
end

function TalksController:index()
  local current_user = get_current_user()
  if not current_user then
    self:redirect("/talks/login")
    return
  end

  local db = _G.Sdb or SoliDB.new({})

  -- Fetch all channels
  -- Logic from www: standard OR system OR (private AND member)
  local channelsRes = db:Sdbql("FOR c IN channels FILTER c.type == 'standard' OR c.type == 'system' OR (c.type == 'private' AND @me IN c.members) SORT c.type DESC, c.name ASC RETURN c", { me = current_user._key })
  local channels = (channelsRes and channelsRes.result) or {}

  -- Current channel
  local currentChannelName = self.params.channel or "general"

  -- Ensure "general" exists if empty
  if #channels == 0 and currentChannelName == "general" then
      -- Maybe seed it?
  end

  local channel = get_or_create_channel(db, current_user, currentChannelName)
  if not channel then
      -- Fallback or error
       if currentChannelName ~= "general" then
           self:redirect("/talks?channel=general")
           return
       end
  end

  -- If we have a channel, fetch messages
  local messages = {}
  if channel then
      local messagesRes = db:Sdbql("FOR m IN messages FILTER m.channel_id == @channelId SORT m.timestamp ASC RETURN m", { channelId = channel._id })
      messages = (messagesRes and messagesRes.result) or {}

      -- Update last seen
      local seen = current_user.channel_last_seen or {}
      seen[channel._id] = os.time()
      db:UpdateDocument("users/" .. current_user._key, { channel_last_seen = seen })
      current_user.channel_last_seen = seen
  end

  -- Users for sidebar
  local usersRes = db:Sdbql("FOR u IN users RETURN { _key: u._key, _id: u._id, firstname: u.firstname, lastname: u.lastname, email: u.email, status: u.status, connection_count: u.connection_count }")
  local users = (usersRes and usersRes.result) or {}

  -- DM Channels
  local dmRes = db:Sdbql("FOR c IN channels FILTER c.type == 'dm' AND POSITION(c.members, @me) >= 0 RETURN c", { me = current_user._key })
  local dmChannels = (dmRes and dmRes.result) or {}

  -- Skip layout for HTMX requests (partial updates)
  if not self:is_htmx_request() then
    self.layout = "talks"
  end

  self:render("talks/index", {
    current_user = current_user,
    channels = channels,
    current_channel = currentChannelName,
    currentChannelData = channel,
    messages = messages,
    users = users,
    dmChannels = dmChannels,
    db_name = db._db_config and db._db_config.db_name or "_system",
  })
end

-- Login Form (GET /talks/login)
function TalksController:login()
   self.layout = "auth"
   local flash_error = nil
   if type(GetFlashMessage) == "function" then
     flash_error = GetFlashMessage("error")
   end
   self:render("talks/login", { error = flash_error })
end

-- Login Action (POST /talks/login)
function TalksController:do_login()
  local email = self.params.email
  local password = self.params.password
  local db = _G.Sdb or SoliDB.new({})

  if not email or not password then
     SetFlash("error", "Email and password are required")
     self:redirect("/talks/login")
     return
  end

  -- 1. Find user by email
  -- Using Sdbql for query
  local usersRes = db:Sdbql("FOR u IN users FILTER u.email == @email LIMIT 1 RETURN u", { email = email })
  local user = (usersRes and usersRes.result and usersRes.result[1])

  if not user then
      SetFlash("error", "Invalid email or password")
      self:redirect("/talks/login")
      return
  end

  -- 2. Verify password (argon2)
  -- user.password is the hash
  local valid, err = argon2.verify(user.password_hash, password) -- Keep user.password_hash as per original code
  if not valid then
      SetFlash("error", "Invalid email or password")
      self:redirect("/talks/login")
      return
  end

  -- 3. Set Session
  local session = GetSession()
  session.user_id = user._id -- Store full ID users/123

  self:redirect("/talks")
end

-- Signup Form (GET /talks/signup)
function TalksController:signup()
  self.layout = "auth"
  -- signup.etlua is a complete HTML page, no layout needed
  local flash_error = nil
  if type(GetFlashMessage) == "function" then
    flash_error = GetFlashMessage("error")
  end
  self:render("talks/signup", { error = flash_error })
end

-- Signup Action (POST /talks/signup)
function TalksController:do_signup()
  local firstname = self.params.firstname
  local lastname = self.params.lastname
  local email = self.params.email
  local password = self.params.password
  local db = _G.Sdb or SoliDB.new({})

  if not firstname or not lastname or not email or not password then
     SetFlash("error", "All fields are required")
     self:redirect("/talks/signup")
     return
  end

  if #password < 8 then
     SetFlash("error", "Password must be at least 8 chars")
     self:redirect("/talks/signup")
     return
  end

  -- Check email
  local res = db:Sdbql("FOR u IN users FILTER u.email == @email RETURN u", { email = email })
  if res and res.result and #res.result > 0 then
     SetFlash("error", "Email already registered")
     self:redirect("/talks/signup")
     return
  end

  -- Create user
  local salt = EncodeBase64(GetRandomBytes(16))
  local hash, err = argon2.hash_encoded(password, salt)

  local user = {
    firstname = firstname,
    lastname = lastname,
    email = email,
    password_hash = hash,
    connection_count = 0,
    status = "offline"
  }

  local createRes = db:CreateDocument("users", user)
  -- Assuming createRes has _id or we fetch it
  local userId = createRes._id
  if not userId then
     local uRes = db:Sdbql("FOR u IN users FILTER u.email == @email RETURN u", { email = email })
     if uRes and uRes.result and #uRes.result > 0 then
         userId = uRes.result[1]._id
     end
  end

  SetSession({ user_id = userId })
  self:redirect("/talks")
end

-- Logout
function TalksController:logout()
  DestroySession()
  self:redirect("/talks/login")
end

-- Fetch Messages (HTMX)
function TalksController:messages()
  local current_user = get_current_user()
  if not current_user then return end
  local db = _G.Sdb or SoliDB.new({})

  local channelName = self.params.channel or "general"
  local channel = get_or_create_channel(db, current_user, channelName)

  local messages = {}
  if channel then
    local messagesRes = db:Sdbql("FOR m IN messages FILTER m.channel_id == @channelId SORT m.timestamp ASC RETURN m", { channelId = channel._id })
    messages = (messagesRes and messagesRes.result) or {}
  end

  -- Render partial _messages
  self:render_partial("talks/_messages", {
    messages = messages,
    channel = channelName
  })
end

-- Send Message (HTMX/API)
function TalksController:send_message()
  local current_user = get_current_user()
  if not current_user then
     if self.params._json then self:render_json({ error = "Unauthorized" }, 401) else self:redirect("/talks/login") end
     return
  end

  local db = _G.Sdb or SoliDB.new({})

  local channelName = self.params.channel or "general"
  local text = self.params.text or ""

  -- If coming from JSON
  if self.params._json then
     channelName = self.params.channel
     text = self.params.text
  end

  if not text or text == "" then
     -- If HTMX, maybe return empty or error?
     if self.params._json then self:render_json({ error = "Message empty" }, 400) end
     return
  end

  local channel = get_or_create_channel(db, current_user, channelName)
  if not channel then
      -- Error handling
      return
  end

  local doc = {
      channel_id = channel._id,
      sender = {
        firstname = current_user.firstname,
        lastname = current_user.lastname,
        email = current_user.email
      },
      user_key = current_user._key,
      text = text,
      timestamp = os.time(),
      reactions = {}
  }

  local result = db:CreateDocument("messages", doc)

  -- Result is just the metadata usually, so we construct the message object for display
  -- or fetch it back.
  doc._key = result._key
  doc._id = result._id

  if self.params._json then
      self:render_json({ success = true, message = doc }, 201)
  else
      -- HTMX: Render the single message partial
      self:render_partial("talks/_message", { message = doc })
  end
end

-- Channels List (HTMX)
function TalksController:channels_list()
  local current_user = get_current_user()
  if not current_user then return end
  local db = _G.Sdb or SoliDB.new({})

  local channelsRes = db:Sdbql("FOR c IN channels FILTER c.type == 'standard' OR c.type == 'system' OR (c.type == 'private' AND @me IN c.members) SORT c.type DESC, c.name ASC RETURN c", { me = current_user._key })
  local channels = (channelsRes and channelsRes.result) or {}

  self:render_partial("talks/_channels_list", { channels = channels, current_channel = self.params.channel })
end

-- DM List (HTMX)
function TalksController:dm_list()
  local current_user = get_current_user()
  if not current_user then return end
  local db = _G.Sdb or SoliDB.new({})

  local dmRes = db:Sdbql("FOR c IN channels FILTER c.type == 'dm' AND POSITION(c.members, @me) >= 0 RETURN c", { me = current_user._key })
  local dmChannels = (dmRes and dmRes.result) or {}

  self:render_partial("talks/_dm_list", { dmChannels = dmChannels, current_channel = self.params.channel })
end

return TalksController
