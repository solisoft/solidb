local Model = require("model")

local BelotePresence = Model.create("belote_presence", {
  permitted_fields = {
    "game_key",    -- The game being watched
    "user_key",    -- The connected user
    "seat",        -- Player's seat (if playing)
    "updated_at"   -- Last heartbeat timestamp
  }
})

-- Heartbeat timeout in seconds (if no heartbeat in this time, considered disconnected)
BelotePresence.TIMEOUT = 30

-- Register or update presence for a user in a game
function BelotePresence.heartbeat(game_key, user_key, seat)
  local now = os.time()
  local presence_key = game_key .. "_" .. user_key

  -- Try to update existing presence
  local result = Sdb:Sdbql([[
    UPSERT { _key: @key }
    INSERT { _key: @key, game_key: @game_key, user_key: @user_key, seat: @seat, updated_at: @now }
    UPDATE { updated_at: @now, seat: @seat }
    IN belote_presence
    RETURN NEW
  ]], {
    key = presence_key,
    game_key = game_key,
    user_key = user_key,
    seat = seat,
    now = now
  })

  return result and result.result and result.result[1]
end

-- Remove presence for a user in a game
function BelotePresence.leave(game_key, user_key)
  local presence_key = game_key .. "_" .. user_key
  Sdb:Sdbql([[
    REMOVE { _key: @key } IN belote_presence
  ]], { key = presence_key })
end

-- Get all active presence records for a game
function BelotePresence.for_game(game_key)
  local cutoff = os.time() - BelotePresence.TIMEOUT

  local result = Sdb:Sdbql([[
    FOR p IN belote_presence
    FILTER p.game_key == @game_key AND p.updated_at >= @cutoff
    RETURN p
  ]], { game_key = game_key, cutoff = cutoff })

  local presences = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(presences, BelotePresence:new(doc))
    end
  end
  return presences
end

-- Check if a game has any active connections
function BelotePresence.has_active_connections(game_key)
  local cutoff = os.time() - BelotePresence.TIMEOUT

  local result = Sdb:Sdbql([[
    FOR p IN belote_presence
    FILTER p.game_key == @game_key AND p.updated_at >= @cutoff
    LIMIT 1
    RETURN 1
  ]], { game_key = game_key, cutoff = cutoff })

  return result and result.result and #result.result > 0
end

-- Get all game keys with active presence
function BelotePresence.games_with_presence()
  local cutoff = os.time() - BelotePresence.TIMEOUT

  local result = Sdb:Sdbql([[
    FOR p IN belote_presence
    FILTER p.updated_at >= @cutoff
    COLLECT game_key = p.game_key
    RETURN game_key
  ]], { cutoff = cutoff })

  local game_keys = {}
  if result and result.result then
    for _, key in ipairs(result.result) do
      game_keys[key] = true
    end
  end
  return game_keys
end

-- Clean up stale presence records
function BelotePresence.cleanup_stale()
  local cutoff = os.time() - BelotePresence.TIMEOUT * 2  -- Keep some buffer

  Sdb:Sdbql([[
    FOR p IN belote_presence
    FILTER p.updated_at < @cutoff
    REMOVE p IN belote_presence
  ]], { cutoff = cutoff })
end

return BelotePresence
