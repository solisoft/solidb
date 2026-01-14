local Controller = require("controller")
local BeloteController = Controller:extend()
local BeloteGame = require("models.belote_game")
local User = require("models.user")
local AuthHelper = require("helpers.auth_helper")

local function get_current_user()
  return AuthHelper.get_current_user()
end

function BeloteController:index()
  -- List open games using Model pattern
  local games = BeloteGame:new():where({ status = "waiting" }):order("doc.created_at DESC"):all()
  
  self:render("belote/index", { games = games, current_user = get_current_user() })
end

function BeloteController:create()
  local current_user = get_current_user()
  if not current_user then return self:redirect("/auth/login") end
  
  local game = BeloteGame:new({ 
    created_by = current_user._key,
    players = { current_user._key }
  })
  game:save()
  
  return self:redirect("/belote/game/" .. game._key)
end

function BeloteController:join()
  local game = BeloteGame:find(self.params.id)
  if not game then return self:redirect("/belote") end
  
  local current_user = get_current_user()
  local success, err = game:join(current_user._key)
  
  if success then
    return self:redirect("/belote/game/" .. game._key)
  else
    -- Handle error (flash message?)
    return self:redirect("/belote/game/" .. game._key) -- Just redirect for now
  end
end

function BeloteController:show()
  local game = BeloteGame:find(self.params.id)
  if not game then return self:redirect("/belote") end
  
  local current_user = get_current_user()
  local player_idx = game:get_player_index(current_user._key)
  
  -- Get player names
  local players_info = {}
  for i, key in ipairs(game.players) do
    if key:match("^bot_") then
      players_info[i] = "🤖 Bot " .. key:match("bot_(%d+)")
    else
      local u = User.find(key)
      players_info[i] = u and (u.firstname or u.username) or "Unknown"
    end
  end
  
  self:render("belote/game", { 
    game = game, 
    current_user = current_user,
    player_idx = player_idx,
    players_info = players_info
  })
end

-- HTMX Partial for game state updates
function BeloteController:state()
  local game = BeloteGame:find(self.params.id)
  if not game then return "Game not found" end
  
  local current_user = get_current_user()
  local player_idx = game:get_player_index(current_user._key)
  
  -- Get player names (could be optimized)
  local players_info = {}
  for i, key in ipairs(game.players) do
    if key:match("^bot_") then
      players_info[i] = "🤖 Bot " .. key:match("bot_(%d+)")
    else
      local u = User.find(key)
      players_info[i] = u and (u.firstname or u.username) or "Unknown"
    end
  end
  
  self.layout = false
  self:render("belote/_board", { 
    game = game, 
    current_user = current_user,
    player_idx = player_idx,
    players_info = players_info
  })
end

function BeloteController:play_card()
  local game = BeloteGame:find(self.params.id)
  if not game then return self:json({ error = "Game not found" }, 404) end
  
  local current_user = get_current_user()
  local card_idx = tonumber(self.params.card_idx)
  
  P("play_card - user: " .. current_user._key .. " card_idx: " .. tostring(card_idx))
  P(game.hands)
  
  local success, err = game:play_card(current_user._key, card_idx)
  
  if success then
    return self:json({ success = true })
  else
    P("play_card ERROR: " .. tostring(err))
    return self:json({ error = err }, 400)
  end
end

function BeloteController:start_game()
  local game = BeloteGame:find(self.params.id)
  if not game then return self:redirect("/belote") end
  
  game:start_round()
  return self:redirect("/belote/game/" .. game._key)
end

function BeloteController:take_trump()
  local game = BeloteGame:find(self.params.id)
  if not game then return self:json({ error = "Game not found" }, 404) end
  
  local current_user = get_current_user()
  local called_suit = self.params.suit -- For round 2
  
  local success, err = game:take_trump(current_user._key, called_suit)
  
  if success then
    return self:json({ success = true })
  else
    return self:json({ error = err }, 400)
  end
end

function BeloteController:pass_trump()
  local game = BeloteGame:find(self.params.id)
  if not game then return self:json({ error = "Game not found" }, 404) end
  
  local current_user = get_current_user()
  
  local success, err = game:pass_trump(current_user._key)
  
  if success then
    return self:json({ success = true })
  else
    return self:json({ error = err }, 400)
  end
end

return BeloteController
