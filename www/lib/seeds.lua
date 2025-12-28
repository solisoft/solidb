local Seeds = {}

function Seeds.run(db)
    -- Helper to log if available
    local function log_message(msg)
        if Logger then Logger(msg) else print(msg) end
    end

    if not _G.APP_INITIALIZED then
        pcall(function() db:CreateCollection("channels") end)
        pcall(function() db:CreateCollection("messages") end)
        pcall(function() db:CreateCollection("users") end)
        pcall(function() db:CreateCollection("files", { type = "blob" }) end)
        pcall(function() db:CreateCollection("signals") end)

        -- Create Indexes
        pcall(function() db:CreateIndex("channels", { fields = { "name" }, unique = true }) end)
        pcall(function() db:CreateIndex("users", { fields = { "email" }, unique = true }) end)
        pcall(function() db:CreateIndex("messages", { fields = { "channel_id" }, unique = false }) end)
        pcall(function() db:CreateIndex("signals", { fields = { "timestamp" }, type = "ttl", expireAfter = 3600 }) end)

        -- Create fulltext index on messages.text for search (if it doesn't exist)
        local existing_indexes = db:GetAllIndexes("messages")
        local has_fulltext_index = false
        if existing_indexes and existing_indexes.identifiers then
            for _, idx in pairs(existing_indexes.identifiers) do
                if idx.type == "fulltext" and idx.fields and idx.fields[1] == "text" then
                    has_fulltext_index = true
                    break
                end
            end
        end
        if not has_fulltext_index then
            pcall(function() db:CreateIndex("messages", { fields = { "text" }, type = "fulltext" }) end)
        end

        _G.APP_INITIALIZED = true
    end

    -- Bootstrap presence WebSocket script
    -- We force delete and recreate to ensure it has the latest code
    local presence_script_key = "talks_presence"
    db:Sdbql("REMOVE @key IN _scripts", { key = presence_script_key })

    log_message("Creating presence WebSocket script...")
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
    log_message("Presence WebSocket script created.")

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

    -- Seed mentions channel if not exists
    local mentionsRes = db:Sdbql("FOR c IN channels FILTER c.name == 'mentions' RETURN c")
    if mentionsRes and mentionsRes.result and #mentionsRes.result == 0 then
        db:CreateDocument("channels", { name = "mentions", type = "system" })
    end
end


return Seeds
