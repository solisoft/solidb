local Controller = require("controller")
local BeloteController = Controller:extend()
local AuthHelper = require("helpers.auth_helper")
local BeloteGame = require("models.belote_game")
local BelotePresence = require("models.belote_presence")

-- Get current user (with redirect if not found)
local function get_current_user(controller)
  local user = AuthHelper.get_current_user()
  if not user then
    controller:redirect("/auth/login?redirect=" .. (GetPath() or "/belote"))
    return nil
  end
  return user
end

-- GET /belote - Lobby page (list of games)
function BeloteController:index()
  local current_user = get_current_user(self)
  if not current_user then return end

  -- Cleanup stale and finished games
  BeloteGame.cleanup_stale_games()

  local games = BeloteGame.waiting_games()
  local my_games = BeloteGame.for_user(current_user._key)

  self.layout = "belote"
  self:render("belote/index", {
    current_user = current_user,
    games = games,
    my_games = my_games,
    db_name = Sdb._db_config and Sdb._db_config.db_name or "_system"
  })
end

-- GET /belote/games - HTMX partial for games list
function BeloteController:games_list()
  local current_user = get_current_user(self)
  if not current_user then return end

  local games = BeloteGame.waiting_games()

  self.layout = false
  self:render("belote/_games_list", {
    games = games,
    current_user = current_user
  })
end

-- GET /belote/modal/create - Create game modal
function BeloteController:modal_create()
  local current_user = get_current_user(self)
  if not current_user then return end

  self.layout = false
  self:render("belote/_create_modal", {
    current_user = current_user
  })
end

-- POST /belote/games - Create a new game
function BeloteController:create()
  local current_user = get_current_user(self)
  if not current_user then return end

  local name = self.params.name
  if not name or name == "" then
    name = (current_user.firstname or "Player") .. "'s Game"
  end

  -- Create game
  local now = os.time()
  local game = BeloteGame:create({
    name = name,
    host_key = current_user._key,
    state = BeloteGame.STATES.WAITING,
    players = {},
    scores = { team_a = 0, team_b = 0 },
    deck = {},
    hands = {},
    current_trick = {},
    tricks_won = { team_a = 0, team_b = 0 },
    dealer = 0,
    created_at = now,
    updated_at = now
  })

  -- Host joins seat 0 (South)
  local BeloteEngine = require("helpers.belote_engine")
  BeloteEngine.join_game(game, current_user._key, 0, false, current_user.firstname or "Player")

  if self:is_htmx_request() then
    self:set_header("HX-Redirect", "/belote/game/" .. game._key)
    return self:html("")
  end

  return self:redirect("/belote/game/" .. game._key)
end

-- GET /belote/game/:key - Game room page
function BeloteController:show()
  local current_user = get_current_user(self)
  if not current_user then return end

  local game = BeloteGame:find(self.params.key)

  if not game then
    return self:redirect("/belote")
  end

  -- Find player's seat (if they're in this game)
  local my_seat = -1
  local player = game:player_by_key(current_user._key)
  if player then
    my_seat = player.seat
  end

  self.layout = "belote"
  self:render("belote/game", {
    current_user = current_user,
    game = game,
    my_seat = my_seat,
    is_host = game.host_key == current_user._key,
    db_name = Sdb._db_config and Sdb._db_config.db_name or "_system"
  })
end

-- GET /belote/game/:key/state - Get current game state (JSON)
function BeloteController:state()
  local current_user = get_current_user(self)
  if not current_user then return self:json({ error = "Unauthorized" }, 401) end

  local game = BeloteGame:find(self.params.key)

  if not game then
    return self:json({ error = "Game not found" }, 404)
  end

  -- Find player's seat
  local my_seat = -1
  local player = game:player_by_key(current_user._key)
  if player then
    my_seat = player.seat
  end

  -- Build card counts for each seat (don't reveal other players' cards)
  local card_counts = {}
  local hands = game.hands or {}
  for seat = 0, 3 do
    local hand = hands[tostring(seat)] or {}
    card_counts[tostring(seat)] = #hand
  end

  -- Build response (only show player's own hand)
  local response = {
    _key = game._key,
    state = game.state,
    players = game.players,
    trump_suit = game.trump_suit,
    trump_chooser = game.trump_chooser,
    turned_card = game.turned_card,
    bidding_seat = game.bidding_seat,
    bidding_passed = game.bidding_passed or {},
    current_player = game.current_player,
    current_trick = game.current_trick,
    last_trick = game.last_trick,
    last_trick_winner = game.last_trick_winner,
    trick_lead = game.trick_lead,
    tricks_won = game.tricks_won,
    scores = game.scores,
    dealer = game.dealer,
    my_seat = my_seat,
    my_hand = (my_seat >= 0) and game:hand_for_seat(my_seat) or {},
    card_counts = card_counts,
    is_host = game.host_key == current_user._key
  }

  return self:json(response)
end

-- POST /belote/game/:key/join - Join a game
function BeloteController:join()
  local current_user = get_current_user(self)
  if not current_user then
    return self:json({ error = "Unauthorized" }, 401)
  end

  local game = BeloteGame:find(self.params.key)
  local seat = tonumber(self.params.seat)

  if not game then
    return self:json({ error = "Game not found" }, 404)
  end

  if game.state ~= BeloteGame.STATES.WAITING then
    return self:json({ error = "Game already started" }, 400)
  end

  local BeloteEngine = require("helpers.belote_engine")
  local result = BeloteEngine.join_game(
    game,
    current_user._key,
    seat,
    false,
    current_user.firstname or "Player"
  )

  if result.error then
    return self:json(result, 400)
  end

  return self:json({ success = true, game = result.game })
end

-- POST /belote/game/:key/leave - Leave a game
function BeloteController:leave()
  local current_user = get_current_user(self)
  if not current_user then
    return self:json({ error = "Unauthorized" }, 401)
  end

  local game = BeloteGame:find(self.params.key)

  if not game then
    return self:json({ error = "Game not found" }, 404)
  end

  if game.state ~= BeloteGame.STATES.WAITING then
    return self:json({ error = "Cannot leave a started game" }, 400)
  end

  local BeloteEngine = require("helpers.belote_engine")
  local result = BeloteEngine.leave_game(game, current_user._key)

  return self:json(result)
end

-- POST /belote/game/:key/add_bot - Add bot to seat
function BeloteController:add_bot()
  local current_user = get_current_user(self)
  if not current_user then
    return self:json({ error = "Unauthorized" }, 401)
  end

  local game = BeloteGame:find(self.params.key)
  local seat = tonumber(self.params.seat)

  if not game then
    return self:json({ error = "Game not found" }, 404)
  end

  -- Only host can add bots
  if game.host_key ~= current_user._key then
    return self:json({ error = "Only host can add bots" }, 403)
  end

  if game.state ~= BeloteGame.STATES.WAITING then
    return self:json({ error = "Game already started" }, 400)
  end

  local BeloteEngine = require("helpers.belote_engine")
  local result = BeloteEngine.add_bot(game, seat)

  return self:json(result)
end

-- POST /belote/game/:key/start - Start the game
function BeloteController:start()
  local current_user = get_current_user(self)
  local game = BeloteGame:find(self.params.key)

  if not game then
    return self:json({ error = "Game not found" }, 404)
  end

  if game.host_key ~= current_user._key then
    return self:json({ error = "Only host can start" }, 403)
  end

  if not game:is_full() then
    return self:json({ error = "Need 4 players to start" }, 400)
  end

  local BeloteEngine = require("helpers.belote_engine")
  local result = BeloteEngine.start_game(game)

  -- Check if next player is a bot (client will trigger with delay)
  if result.success then
    local updated_game = BeloteGame:find(game._key)
    if updated_game then
      local next_seat = updated_game.bidding_seat
      for _, p in ipairs(updated_game.players or {}) do
        if p.seat == next_seat and p.is_bot then
          result.needs_bot_turn = true
          break
        end
      end
    end
  end

  return self:json(result)
end

-- POST /belote/game/:key/bid - Place a bid
function BeloteController:bid()
  local current_user = get_current_user(self)
  local game = BeloteGame:find(self.params.key)
  local action = self.params.action  -- "take" or "pass"
  local suit = self.params.suit       -- Only if "take"

  if not game then
    return self:json({ error = "Game not found" }, 404)
  end

  if game.state ~= BeloteGame.STATES.BIDDING then
    return self:json({ error = "Not in bidding phase" }, 400)
  end

  -- Find player's seat
  local player = game:player_by_key(current_user._key)
  if not player then
    return self:json({ error = "You're not in this game" }, 403)
  end

  if game.bidding_seat ~= player.seat then
    return self:json({ error = "Not your turn to bid" }, 400)
  end

  local BeloteEngine = require("helpers.belote_engine")
  local result = BeloteEngine.process_bid(game, player.seat, action, suit)

  -- Check if next player is a bot (client will trigger with delay)
  if result.success then
    local updated_game = BeloteGame:find(game._key)
    if updated_game then
      local next_seat = updated_game.bidding_seat or updated_game.current_player
      for _, p in ipairs(updated_game.players or {}) do
        if p.seat == next_seat and p.is_bot then
          result.needs_bot_turn = true
          break
        end
      end
    end
  end

  return self:json(result)
end

-- POST /belote/game/:key/play - Play a card
function BeloteController:play()
  local current_user = get_current_user(self)
  local game = BeloteGame:find(self.params.key)

  if not game then
    return self:json({ error = "Game not found" }, 404)
  end

  if game.state ~= BeloteGame.STATES.PLAYING then
    return self:json({ error = "Not in playing phase" }, 400)
  end

  -- Find player's seat
  local player = game:player_by_key(current_user._key)
  if not player then
    return self:json({ error = "You're not in this game" }, 403)
  end

  if game.current_player ~= player.seat then
    return self:json({ error = "Not your turn" }, 400)
  end

  -- Parse card from params
  local card = {
    suit = self.params.suit,
    value = self.params.value
  }

  if not card.suit or not card.value then
    return self:json({ error = "Invalid card" }, 400)
  end

  local BeloteEngine = require("helpers.belote_engine")
  local result = BeloteEngine.play_card(game, player.seat, card)

  -- Check if next player is a bot (client will trigger with delay)
  if result.success then
    local updated_game = BeloteGame:find(game._key)
    if updated_game then
      local next_seat = updated_game.current_player
      for _, p in ipairs(updated_game.players or {}) do
        if p.seat == next_seat and p.is_bot then
          result.needs_bot_turn = true
          break
        end
      end
    end
  end

  return self:json(result)
end

-- POST /belote/game/:key/bot_turn - Trigger a single bot turn (for visual delay)
function BeloteController:bot_turn()
  local game = BeloteGame:find(self.params.key)

  if not game then
    return self:json({ error = "Game not found" }, 404)
  end

  local BeloteEngine = require("helpers.belote_engine")
  local result = BeloteEngine.process_single_bot_turn(game._key)

  return self:json(result)
end

-- GET /belote/livequery_token - Get LiveQuery token for WebSocket
function BeloteController:livequery_token()
  local token = Sdb:LiveQueryToken()
  return self:json({ token = token, expires_in = 30 })
end

-- POST /belote/game/:key/heartbeat - Register presence heartbeat
function BeloteController:heartbeat()
  local current_user = get_current_user(self)
  if not current_user then
    return self:json({ error = "Unauthorized" }, 401)
  end

  local game = BeloteGame:find(self.params.key)
  if not game then
    return self:json({ error = "Game not found" }, 404)
  end

  -- Find player's seat if they're in the game
  local seat = nil
  local player = game:player_by_key(current_user._key)
  if player then
    seat = player.seat
  end

  -- Update presence
  BelotePresence.heartbeat(game._key, current_user._key, seat)

  return self:json({ success = true })
end

-- POST /belote/game/:key/disconnect - Remove presence on disconnect
function BeloteController:disconnect()
  local current_user = get_current_user(self)
  if not current_user then
    return self:json({ error = "Unauthorized" }, 401)
  end

  BelotePresence.leave(self.params.key, current_user._key)

  return self:json({ success = true })
end

return BeloteController
