local Solidb = require("db.solidb")

local get_current_user = function()
  local db = SoliDB.primary
  local user_id = GetCookie("talks_session")
  if not user_id or user_id == "" then return nil end

  local res = db:Sdbql("FOR u IN users FILTER u._id == @id RETURN u", { id = user_id })
  if res and res.result and #res.result > 0 then
    return res.result[1]
  end
  return nil
end

local app = {
  index = function()
    -- Initialize database connection
    local db = SoliDB.primary

    -- Create collections if they don't exist
    pcall(function() db:CreateCollection("channels") end)
    pcall(function() db:CreateCollection("messages") end)
    pcall(function() db:CreateCollection("users") end)
    pcall(function() db:CreateCollection("files", { type = "blob" }) end)
    pcall(function() db:CreateCollection("signals") end)

    -- Bootstrap presence WebSocket script
    -- We force delete and recreate to ensure it has the latest code
    local presence_script_key = "talks_presence"
    db:Sdbql("REMOVE @key IN _scripts", { key = presence_script_key })

    Logger("Creating presence WebSocket script...")
    local presence_code = [=[
local user_id = context.query_params.user_id
if not user_id then return end

-- Increment connection count atomically
local res = db:query([[
    FOR u IN users
    FILTER u._id == @id OR u._key == @id
    UPDATE u WITH {
        connection_count: IF(u.connection_count != null, u.connection_count, 0) + 1,
        status: "online",
        last_seen: DATE_NOW()
    } IN users
    RETURN NEW
]], { id = user_id })

while true do
    local msg = solidb.ws.recv()
    if not msg then break end
end

-- Decrement connection count atomically and update status
db:query([[
    FOR u IN users
    FILTER u._id == @id OR u._key == @id
    LET old_count = IF(u.connection_count != null, u.connection_count, 1)
    LET new_count = old_count - 1
    LET status = IF(new_count > 0, "online", "offline")
    UPDATE u WITH {
        connection_count: new_count,
        status: status,
        last_seen: DATE_NOW()
    } IN users
]], { id = user_id })
]=]
    db:CreateDocument("_scripts", {
      _key = presence_script_key,
      name = "Presence Tracker",
      methods = { "WS" },
      path = "presence",
      database = db._db_config.db_name,
      code = presence_code,
      description = "Tracks user online/offline status via WebSocket connection counting",
      created_at = os.date("!%Y-%m-%dT%H:%M:%SZ"),
      updated_at = os.date("!%Y-%m-%dT%H:%M:%SZ")
    })
    Logger("Presence WebSocket script created.")

    -- Seed demo messages for #general if not already present
    local res = db:Sdbql("FOR c IN channels FILTER c.name == 'general' RETURN c")
    if res and res.result and #res.result == 0 then
      -- Create channels
      local general_channel = db:CreateDocument("channels", { name = "general", type = "standard" })
      db:CreateDocument("channels", { name = "development", type = "standard" })
      db:CreateDocument("channels", { name = "announcements", type = "standard" })

      -- Insert demo messages with reactions (usernames list) and code samples
      db:CreateDocument("messages", {
        channel_id = general_channel._id,
        sender = "rust-bot",
        text = "Welcome to #general! The SoliDB cluster is now connected and ready for action.",
        timestamp = os.time(),
        reactions = {
          { emoji = "🚀", users = { "olivier.bonnaure", "antigravity", "rust-bot" } },
          { emoji = "👍", users = { "antigravity", "olivier.bonnaure" } }
        }
      })
      db:CreateDocument("messages", {
        channel_id = general_channel._id,
        sender = "antigravity",
        text = [[Here's a sample query optimization I've been working on:
```rust
#[inline(always)]
pub fn optimize_query(query: &Query) -> Result<Plan, Error> {
    // Fast path for point lookups
    if let Some(key) = query.get_point_key() {
        return Ok(Plan::PointLookup(key.clone()));
    }

    // Cost-based optimization
    let mut plan = Plan::default();
    plan.analyze_filters(&query.filters)?;
    Ok(plan)
}
```]],
        timestamp = os.time() + 1,
        reactions = { { emoji = "🔥", users = { "olivier.bonnaure", "rust-bot", "alice", "bob", "charlie" } } }
      })
      db:CreateDocument("messages", {
        channel_id = general_channel._id,
        sender = "olivier.bonnaure",
        text = "The UI looks great! Let's start integrating real data. Love the dark theme 🌙",
        timestamp = os.time() + 2,
        reactions = {
          { emoji = "❤️", users = { "antigravity", "rust-bot", "alice", "bob" } },
          { emoji = "✨", users = { "antigravity", "charlie" } }
        }
      })
      db:CreateDocument("messages", {
        channel_id = general_channel._id,
        sender = "rust-bot",
        text = [[System performance metrics for today:
```json
{
  "queries_per_second": 15420,
  "avg_latency_ms": 0.8,
  "cache_hit_rate": 0.94,
  "active_connections": 127
}
```]],
        timestamp = os.time() + 3,
        reactions = { { emoji = "📊", users = { "olivier.bonnaure" } } }
      })
    end

    -- Fetch all channels
    local channelsRes = db:Sdbql("FOR c IN channels FILTER c.type != 'dm' SORT c.name ASC RETURN c")
    local channels = (channelsRes and channelsRes.result) or {}

    -- Get current user early for validation
    local current_user = get_current_user()
    if not current_user then
      RedirectTo("/talks/login")
      return
    end

    -- Get current channel (default to first channel from DB)
    local currentChannel = GetParam("channel") or "general"

    if not currentChannel or currentChannel == "" then
      if #channels > 0 then
        currentChannel = channels[1].name
      else
        currentChannel = "general"
      end
    end

    Params.channel = currentChannel

    local channelRes = db:Sdbql(
      [[
        FOR c IN channels FILTER c.name == @name RETURN c
      ]],
      { name = currentChannel }
    ).result[1]

    -- Lazy create DM channel if it doesn't exist
    if not channelRes and string.sub(currentChannel, 1, 3) == "dm_" then
      local remainder = string.sub(currentChannel, 4) -- Remove "dm_" prefix
      local underscorePos = string.find(remainder, "_")

      if underscorePos and current_user then
        local key1 = string.sub(remainder, 1, underscorePos - 1)
        local key2 = string.sub(remainder, underscorePos + 1)

        -- Validate keys exist in users collection
        local user1Res = db:Sdbql("FOR u IN users FILTER u._key == @k RETURN u._key", { k = key1 })
        local user2Res = db:Sdbql("FOR u IN users FILTER u._key == @k RETURN u._key", { k = key2 })

        local user1Exists = user1Res and user1Res.result and #user1Res.result > 0
        local user2Exists = user2Res and user2Res.result and #user2Res.result > 0

        -- For self-DM, both keys are the same
        local isSelfDM = (key1 == key2)
        local bothExist = isSelfDM and user1Exists or (user1Exists and user2Exists)

        if bothExist then
            -- Verify authorization: current user must be one of the keys
            local isAuthorized = (current_user._key == key1 or current_user._key == key2)

            if isAuthorized then
                -- Create the channel
                local createRes = db:CreateDocument("channels", {
                    name = currentChannel,
                    type = "dm",
                    members = { key1, key2 }
                })

                -- Refetch the channel to ensure we have the correct format
                channelRes = db:Sdbql("FOR c IN channels FILTER c.name == @name RETURN c", { name = currentChannel }).result[1]
            end
        end
      end
    end

    -- If still no channel found, default to general
    if not channelRes then
        Params.channel = "general"
        channelRes = db:Sdbql("FOR c IN channels FILTER c.name == 'general' RETURN c").result[1]
    end

    -- Fetch messages for current channel using the resolved channel ID
    local messagesRes = db:Sdbql(
      [[
        FOR m IN messages FILTER m.channel_id == @channelId
        SORT m.timestamp ASC RETURN m
      ]],
      { channelId = channelRes._id }
    )
    local messages = (messagesRes and messagesRes.result) or {}

    -- Fetch all users for the sidebar
    local usersRes = db:Sdbql("FOR u IN users RETURN { _key: u._key, _id: u._id, firstname: u.firstname, lastname: u.lastname, email: u.email, status: u.status, connection_count: u.connection_count }")
    local users = (usersRes and usersRes.result) or {}

    -- Fetch DM channels for the current user
    local dmRes = db:Sdbql("FOR c IN channels FILTER c.type == 'dm' AND POSITION(c.members, @me) >= 0 RETURN c", { me = current_user._key })
    local dmChannels = (dmRes and dmRes.result) or {}

    -- Pass data to view
    Params.channels = EncodeJson(channels)
    Params.channelId = channelRes._id
    Params.messages = EncodeJson(messages)
    Params.channels = EncodeJson(channels)
    Params.channelId = channelRes._id
    Params.messages = EncodeJson(messages)
    Params.users = EncodeJson(users)
    Params.dmChannels = EncodeJson(dmChannels)
    Params.currentChannel = currentChannel

    -- Robustly extract DB name
    local config = db._db_config
    Params.db_name = config.db_name or config.database or config.name

    -- DB host for WebSocket connections (from env var or fallback)
    Params.db_host = os.getenv("DB_HOST") or "localhost:6745"

    Logger("DEBUG: Final Params.db_name: " .. tostring(Params.db_name))

    -- Authentication check
    -- Authentication check (already done at top)

    -- Access Control for DM channels
    if channelRes.type == "dm" then
      local isMember = false
      if channelRes.members then
        for _, memberKey in ipairs(channelRes.members) do
          if memberKey == current_user._key then
            isMember = true
            break
          end
        end
      end

      if not isMember then
        -- Unauthorized access to DM, redirect to general
        RedirectTo("/talks?channel=general")
        return
      end
    end

    Params.currentUser = EncodeJson(current_user)
    Params.full_height = true
    Params.hide_header = true
    Page("talks/index", "app")
  end,

  login_form = function()
    Params.hide_header = true
    Params.no_padding = true
    Page("talks/login", "app")
  end,

  login = function()
    local db = SoliDB.primary
    local email = GetParam("email")
    local password = GetParam("password")

    if not email or not password then
      Params.error = "Email and password are required"
      Params.hide_header = true
      Params.no_padding = true
      return Page("talks/login", "app")
    end

    local res = db:Sdbql("FOR u IN users FILTER u.email == @email RETURN u", { email = email })

    if not res or not res.result or #res.result == 0 then
      Params.error = "Invalid email or password"
      Params.hide_header = true
      Params.no_padding = true
      return Page("talks/login", "app")
    end

    local user = res.result[1]
    local ok, err = argon2.verify(user.password_hash, password)

    if ok then
      RedirectTo("/talks")
      SetCookie("talks_session", user._id, { Path = "/", HttpOnly = true, MaxAge = 86400 * 30 })
    else
      Params.error = "Invalid email or password"
      Params.hide_header = true
      Params.no_padding = true
      Page("talks/login", "app")
    end
  end,

  signup_form = function()
    Params.hide_header = true
    Params.no_padding = true
    Page("talks/signup", "app")
  end,

  signup = function()
    local db = SoliDB.primary
    local firstname = GetParam("firstname")
    local lastname = GetParam("lastname")
    local email = GetParam("email")
    local password = GetParam("password")

    if not firstname or not lastname or not email or not password then
      Params.error = "All fields are required"
      Params.hide_header = true
      Params.no_padding = true
      return Page("talks/signup", "app")
    end

    -- Check if email exists
    local res = db:Sdbql("FOR u IN users FILTER u.email == @email RETURN u", { email = email })
    if res and res.result and #res.result > 0 then
      Params.error = "Email already registered"
      Params.hide_header = true
      Params.no_padding = true
      return Page("talks/signup", "app")
    end

    -- Hash password
    local salt = EncodeBase64(GetRandomBytes(16))
    local hash, err = argon2.hash_encoded(password, salt)

    if not hash then
      Params.error = "Error hashing password: " .. tostring(err)
      Params.hide_header = true
      Params.no_padding = true
      return Page("talks/signup", "app")
    end

    local user = {
      firstname = firstname,
      lastname = lastname,
      email = email,
      lastname = lastname,
      email = email,
      password_hash = hash,
      connection_count = 0,
      status = "offline"
    }

    local createRes = db:CreateDocument("users", user)

    -- Retrieve the created user (CreateDocument might return the doc or ID depending on driver)
    -- Assuming it returns the document or we use logic to get ID.
    -- Redbean/SoliDB driver typically returns the document or meta.
    -- If createRes has _id, use it.
    local userId = createRes._id
    Logger(createRes)
    if not userId then
       -- Fallback if driver returns something else, fetch by email
       local u = db:Sdbql("FOR u IN users FILTER u.email == @email RETURN u", { email = email }).result[1]
       userId = u._id
    end

    RedirectTo("/talks")
    SetCookie("talks_session", userId, { Path = "/", HttpOnly = true, MaxAge = 86400 * 30 })
  end,

  logout = function()
    RedirectTo("/talks/login")
    SetCookie("talks_session", "", { Path = "/", MaxAge = 0 })
  end,

  -- Proxy file upload to SoliDB blob API
  upload = function()
    local db = SoliDB.primary
    -- Use local get_current_user directly
    local current_user = get_current_user()
    if not current_user then
      SetStatus(401)
      WriteJSON({ error = "Unauthorized" })
      return
    end

    local body = GetBody()
    if not body or #body == 0 then
      SetStatus(400)
      WriteJSON({ error = "Empty body" })
      return
    end

    -- Forward to /_api/blob/{db}/files
    -- We must preserve the Content-Type header from the client request as it contains the multipart boundary
    local content_type = GetHeader("Content-Type")

    local path = "/_api/blob/" .. db._db_config.db_name .. "/files"
    local url = db:Api_url(path)
    local headers = {
       ["Content-Type"] = content_type
    }

    if db._token ~= "" then
      headers["Authorization"] = "Bearer " .. db._token
    end

    local status, res_headers, res_body = Fetch(url, {
      method = "POST",
      body = body,
      headers = headers
    })

    if status ~= 201 and status ~= 200 then
       SetStatus(status)
       -- Try to decode error to give better feedback
       local err_json = DecodeJson(res_body)
       if err_json and err_json.errorMessage then
          WriteJSON({ error = err_json.errorMessage })
       else
          Write(res_body)
       end
       return
    end

    -- res_body is JSON like { _key: "...", name: "...", ... }
    SetStatus(200)
    SetHeader("Content-Type", "application/json")
    Write(res_body)
  end,

  -- Proxy file download from SoliDB blob API
  file = function()
    local db = SoliDB.primary
    -- Allow downloading file if logged in
    local current_user = get_current_user()
    if not current_user then
       SetStatus(401)
       return
    end

    local key = GetParam("key")
    if not key or key == "" then
       SetStatus(400)
       Write("Key required")
       return
    end

    -- Get a short-lived token (2 seconds) instead of exposing the main JWT
    local short_token = db:LiveQueryToken()
    if not short_token then
       SetStatus(500)
       Write("Could not generate access token")
       return
    end

    -- Redirect directly to the blob API with short-lived token
    local path = "/_api/blob/" .. db._db_config.db_name .. "/files/" .. key
    local url = db:Api_url(path) .. "?token=" .. short_token
    RedirectTo(url)
  end,

  -- Generate a short-lived token for live query WebSocket connections
  livequery_token = function()
    local token = SoliDB.primary:LiveQueryToken()
    SetHeader("Content-Type", "application/json")
    if token then
      WriteJSON({ token = token, expires_in = 30 })
    else
      SetStatus(500)
      WriteJSON({ error = "Failed to generate token" })
    end
  end,

  -- API endpoint to create a new message
  create_message = function()
    local db = SoliDB.primary

    -- Parse JSON body
    local body = DecodeJson(GetBody() or "{}") or {}
    local channel = body.channel or "general"
    local text = body.text or ""
    local sender = body.sender or "anonymous"
    -- Attachments is array of { key, filename, type, size }
    local attachments = body.attachments or {}

    -- Require text OR attachments
    if text == "" and #attachments == 0 then
      SetStatus(400)
      WriteJSON({ error = "Message text or attachment is required" })
      return
    end

    -- Create the message
    local result = db:CreateDocument("messages", {
       channel_id = channel,
       sender = sender,
       text = text,
       timestamp = os.time(),
       attachments = attachments,
       reactions = {}
    })

    SetStatus(201)
    SetHeader("Content-Type", "application/json")
    WriteJSON({ success = true, message = result })
  end,

  -- API endpoint to toggle a reaction on a message
  toggle_reaction = function()
    local db = SoliDB.primary

    -- Parse JSON body
    local body = DecodeJson(GetBody() or "{}") or {}
    local message_key = body.message_key or ""
    local emoji = body.emoji or ""
    local username = body.username or "anonymous"

    if message_key == "" or emoji == "" then
      SetStatus(400)
      WriteJSON({ error = "message_key and emoji are required" })
      return
    end

    -- Get the message
    local msgRes = db:Sdbql(string.format(
      "FOR m IN messages FILTER m._key == '%s' RETURN m",
      message_key
    ))

    if not msgRes or not msgRes.result or #msgRes.result == 0 then
      SetStatus(404)
      WriteJSON({ error = "Message not found" })
      return
    end

    local message = msgRes.result[1]
    local reactions = message.reactions or {}
    local foundReaction = nil
    local foundIndex = nil

    -- Find if this emoji reaction already exists
    for i, reaction in ipairs(reactions) do
      if reaction.emoji == emoji then
        foundReaction = reaction
        foundIndex = i
        break
      end
    end

    local action = "added"

    if foundReaction then
      -- Check if user already reacted
      local userFound = false
      local userIndex = nil
      local users = foundReaction.users or {}

      for i, user in ipairs(users) do
        if user == username then
          userFound = true
          userIndex = i
          break
        end
      end

      if userFound then
        -- Remove user from reaction
        table.remove(users, userIndex)
        action = "removed"

        -- If no users left, remove the reaction entirely
        if #users == 0 then
          table.remove(reactions, foundIndex)
        else
          reactions[foundIndex].users = users
        end
      else
        -- Add user to existing reaction
        table.insert(users, username)
        reactions[foundIndex].users = users
      end
    else
      -- Create new reaction with this user
      table.insert(reactions, { emoji = emoji, users = { username } })
    end

    -- Update the message
    db:UpdateDocument("messages/" .. message_key, { reactions = reactions })

    SetStatus(200)
    SetHeader("Content-Type", "application/json")
    -- Ensure reactions is a proper array (use cjson array marker if available)
    if #reactions == 0 then
      Write('{"success":true,"action":"' .. action .. '","reactions":[]}')
    else
    WriteJSON({ success = true, action = action, reactions = reactions })
    end
  end,

  -- Create or get a Direct Message channel
  create_dm = function()
    local db = SoliDB.primary
    local current_user = get_current_user()
    if not current_user then
      SetStatus(401)
      WriteJSON({ error = "Unauthorized" })
      return
    end

    local body = DecodeJson(GetBody() or "{}") or {}
    local target_user_key = body.target_user_key

    if not target_user_key or target_user_key == "" then
      SetStatus(400)
      WriteJSON({ error = "target_user_key is required" })
      return
    end

    -- Sort Keys to create deterministic channel name
    local keys = { current_user._key, target_user_key }
    table.sort(keys)
    local channelName = "dm_" .. keys[1] .. "_" .. keys[2]

    -- Check if channel exists
    local res = db:Sdbql("FOR c IN channels FILTER c.name == @name RETURN c", { name = channelName })

    if not res or not res.result or #res.result == 0 then
      -- Create new DM channel
      db:CreateDocument("channels", {
        name = channelName,
        type = "dm",
        members = { current_user._key, target_user_key }
      })
    end

    SetStatus(200)
    SetHeader("Content-Type", "application/json")
    WriteJSON({ success = true, channel = channelName })
  end,

  -- Send a WebRTC signaling message
  send_signal = function()
    local db = SoliDB.primary
    local current_user = get_current_user()
    if not current_user then
      SetStatus(401)
      WriteJSON({ error = "Unauthorized" })
      return
    end

    local body = DecodeJson(GetBody() or "{}") or {}
    local target_user = body.target_user
    local type = body.type
    local data = body.data

    if not target_user or not type then
      SetStatus(400)
      WriteJSON({ error = "target_user and type are required" })
      return
    end

    -- Create signal document
    -- We include a timestamp so the receiver can ignore old signals
    local signal = {
        from_user = current_user._key,
        to_user = target_user,
        type = type,
        data = data,
        timestamp = os.time() * 1000 -- ms precision if possible, or just os.time()
    }

    db:CreateDocument("signals", signal)

    SetStatus(200)
    WriteJSON({ success = true })
  end,

  -- Fetch Open Graph metadata for URL previews
  og_metadata = function()
    local url = GetParam("url")
    if not url or url == "" then
      SetStatus(400)
      SetHeader("Content-Type", "application/json")
      Write('{"error":"URL parameter required"}')
      return
    end

    -- Fetch the URL content
    local status, headers, body = Fetch(url)

    if status ~= 200 or not body then
      SetStatus(200)
      SetHeader("Content-Type", "application/json")
      Write('{"error":"Failed to fetch URL"}')
      return
    end

    -- Parse Open Graph meta tags
    local og = {}

    -- DEBUG: Log parts of body to see what we're parsing
    if body then
       Logger("OG Fetch Body Preview: " .. body:sub(1, 1000))
    end

    -- og:title
    local title = body:match('<meta[^>]+property=["\']og:title["\'][^>]+content=["\']([^"\']+)["\']')
      or body:match('<meta[^>]+content=["\']([^"\']+)["\'][^>]+property=["\']og:title["\']')
      or body:match('<title>([^<]+)</title>')
    og.title = title

    -- og:description (with fallbacks)
    local desc = body:match('<meta[^>]+property=["\']og:description["\'][^>]+content=["\']([^"\']+)["\']')
      or body:match('<meta[^>]+content=["\']([^"\']+)["\'][^>]+property=["\']og:description["\']')
      or body:match('<meta[^>]+name=["\']twitter:description["\'][^>]+content=["\']([^"\']+)["\']')
      or body:match('<meta[^>]+content=["\']([^"\']+)["\'][^>]+name=["\']twitter:description["\']')
      or body:match('<meta[^>]+name=["\']description["\'][^>]+content=["\']([^"\']+)["\']')
      or body:match('<meta[^>]+content=["\']([^"\']+)["\'][^>]+name=["\']description["\']')
    og.description = desc

    -- og:image (with fallbacks)
    -- Allow spaces around =
    local image = body:match('<meta[^>]+property%s*=%s*["\']og:image["\'][^>]+content%s*=%s*["\']([^"\']+)["\']')
      or body:match('<meta[^>]+content%s*=%s*["\']([^"\']+)["\'][^>]+property%s*=%s*["\']og:image["\']')
      -- twitter:image fallback
      or body:match('<meta[^>]+name%s*=%s*["\']twitter:image["\'][^>]+content%s*=%s*["\']([^"\']+)["\']')
      or body:match('<meta[^>]+content%s*=%s*["\']([^"\']+)["\'][^>]+name%s*=%s*["\']twitter:image["\']')
      -- First large image fallback (skip icons/logos)
      or body:match('<img[^>]+src%s*=%s*["\']([^"\']+%.jpg)["\']')
      or body:match('<img[^>]+src%s*=%s*["\']([^"\']+%.png)["\']')
      or body:match('<img[^>]+src%s*=%s*["\']([^"\']+%.webp)["\']')

    Logger("OG extracted raw image: " .. tostring(image))

    -- Make relative image URL absolute
    if image and not image:match("^https?://") then
      if image:sub(1, 2) == "//" then
        image = "https:" .. image
      elseif image:sub(1, 1) == "/" then
        -- Root relative
        local domain = url:match("^(https?://[^/]+)")
        if domain then
          image = domain .. image
        end
      else
        -- Path relative
        local path_base = url:match("(.*/)")
        if not path_base then
           -- URL is just domain like https://example.com or https://example.com/file (no trailing slash implies file at root?)
           -- Actually if url is https://example.com/foo, base is https://example.com/
           -- robust way: if url ends with /, take it. else remove last segment.
           if url:sub(-1) == "/" then
             path_base = url
           else
             -- check if it has path
             local domain = url:match("^(https?://[^/]+)")
             if url == domain then
               path_base = url .. "/"
             else
               path_base = url:match("(.*/)") or (url .. "/")
             end
           end
        end
        image = path_base .. image
      end
    end
    og.image = image
    Logger("OG final image: " .. tostring(image))

    -- og:site_name
    local site = body:match('<meta[^>]+property=["\']og:site_name["\'][^>]+content=["\']([^"\']+)["\']')
      or body:match('<meta[^>]+content=["\']([^"\']+)["\'][^>]+property=["\']og:site_name["\']')
    og.site_name = site

    -- favicon
    local favicon = body:match('<link[^>]+rel=["\']icon["\'][^>]+href=["\']([^"\']+)["\']')
      or body:match('<link[^>]+href=["\']([^"\']+)["\'][^>]+rel=["\']icon["\']')
      or body:match('<link[^>]+rel=["\']shortcut icon["\'][^>]+href=["\']([^"\']+)["\']')
    if favicon and not favicon:match("^https?://") then
      -- Make relative favicon absolute
      local base = url:match("^(https?://[^/]+)")
      if base then
        if favicon:sub(1, 1) == "/" then
          favicon = base .. favicon
        else
          favicon = base .. "/" .. favicon
        end
      end
    end
    og.favicon = favicon

    og.url = url

    SetStatus(200)
    SetHeader("Content-Type", "application/json")
    SetHeader("Cache-Control", "public, max-age=3600")
    WriteJSON(og)
  end,
}

return BeansEnv == "development" and HandleController(app) or app
