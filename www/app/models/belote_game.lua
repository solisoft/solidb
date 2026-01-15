local Model = require("model")

local BeloteGame = Model.create("belote_games", {
  permitted_fields = {
    "name",              -- Room name
    "host_key",          -- User who created the game
    "state",             -- Game state (waiting, dealing, bidding, playing, scoring, finished)
    "players",           -- Array of 4 player objects {seat, user_key, name, is_bot, team}
    "deck",              -- Remaining shuffled deck
    "hands",             -- Player hands: {["0"]: [...], ["1"]: [...], ...}
    "trump_suit",        -- Current trump suit
    "trump_chooser",     -- Seat of player who chose trump
    "turned_card",       -- Card turned up for bidding
    "bidding_seat",      -- Current bidder seat
    "bidding_passed",    -- Array of seats that passed
    "current_trick",     -- Array of {seat, card} for current trick
    "last_trick",        -- Previous completed trick (for display)
    "trick_lead",        -- Seat that led the current trick
    "tricks_won",        -- {team_a: count, team_b: count}
    "current_player",    -- Seat of player whose turn it is
    "dealer",            -- Seat of current dealer
    "scores",            -- {team_a: total, team_b: total}
    "round_points",      -- Points scored in current round
    "last_trick_winner", -- Seat that won last trick
    "created_at",
    "started_at",
    "finished_at",
    "updated_at"         -- Last activity timestamp
  },
  validations = {
    name = { presence = true, length = { between = {1, 50} } }
  }
})

-- Game states
BeloteGame.STATES = {
  WAITING = "waiting",
  DEALING = "dealing",
  BIDDING = "bidding",
  PLAYING = "playing",
  SCORING = "scoring",
  FINISHED = "finished"
}

-- Suits
BeloteGame.SUITS = { "hearts", "diamonds", "clubs", "spades" }

-- Suit symbols for display
BeloteGame.SUIT_SYMBOLS = {
  hearts = "♥",
  diamonds = "♦",
  clubs = "♣",
  spades = "♠"
}

-- Card values (7-A)
BeloteGame.VALUES = { "7", "8", "9", "10", "J", "Q", "K", "A" }

-- Point values for trump cards
BeloteGame.TRUMP_POINTS = {
  J = 20,
  ["9"] = 14,
  A = 11,
  ["10"] = 10,
  K = 4,
  Q = 3,
  ["8"] = 0,
  ["7"] = 0
}

-- Point values for non-trump cards
BeloteGame.NON_TRUMP_POINTS = {
  A = 11,
  ["10"] = 10,
  K = 4,
  Q = 3,
  J = 2,
  ["9"] = 0,
  ["8"] = 0,
  ["7"] = 0
}

-- Trump card ranking (index = strength, higher is better)
BeloteGame.TRUMP_RANK = {
  ["7"] = 1, ["8"] = 2, Q = 3, K = 4, ["10"] = 5, A = 6, ["9"] = 7, J = 8
}

-- Non-trump card ranking
BeloteGame.NON_TRUMP_RANK = {
  ["7"] = 1, ["8"] = 2, ["9"] = 3, J = 4, Q = 5, K = 6, ["10"] = 7, A = 8
}

-- Seat positions (0=South/player, 1=West, 2=North, 3=East from player's POV)
BeloteGame.SEATS = {
  SOUTH = 0,
  WEST = 1,
  NORTH = 2,
  EAST = 3
}

-- Seat names
BeloteGame.SEAT_NAMES = { [0] = "South", [1] = "West", [2] = "North", [3] = "East" }

-- Team assignments (North/South = A, East/West = B)
BeloteGame.TEAMS = {
  [0] = "a",  -- South = Team A
  [1] = "b",  -- West = Team B
  [2] = "a",  -- North = Team A
  [3] = "b"   -- East = Team B
}

-- Find all active games (exclude finished and stale games)
function BeloteGame.waiting_games()
  local five_minutes_ago = os.time() - 300
  local result = Sdb:Sdbql([[
    FOR g IN belote_games
    FILTER g.state != 'finished'
    FILTER g.updated_at >= @cutoff OR g.created_at >= @cutoff
    SORT g.state == 'waiting' ? 0 : 1, g.created_at DESC
    RETURN g
  ]], { cutoff = five_minutes_ago })

  local games = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(games, BeloteGame:new(doc))
    end
  end
  return games
end

-- Clean up old/finished games (only if no active presence)
function BeloteGame.cleanup_stale_games()
  local BelotePresence = require("models.belote_presence")

  -- First clean up stale presence records
  BelotePresence.cleanup_stale()

  -- Get games with active presence (connected players)
  local games_with_presence = BelotePresence.games_with_presence()

  local one_minute_ago = os.time() - 60

  -- Get candidate games for deletion
  local result = Sdb:Sdbql([[
    FOR g IN belote_games
    FILTER g.state == 'finished' OR (g.updated_at < @cutoff AND g.created_at < @cutoff)
    RETURN g._key
  ]], { cutoff = one_minute_ago })

  -- Only delete games without active presence
  if result and result.result then
    for _, game_key in ipairs(result.result) do
      if not games_with_presence[game_key] then
        Sdb:Sdbql([[
          REMOVE { _key: @key } IN belote_games
        ]], { key = game_key })
      end
    end
  end
end

-- Find games for a user (where they are a player)
function BeloteGame.for_user(user_key)
  local result = Sdb:Sdbql([[
    FOR g IN belote_games
    FILTER g.state != 'finished'
    FILTER @user_key IN g.players[*].user_key
    SORT g.created_at DESC
    RETURN g
  ]], { user_key = user_key })

  local games = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(games, BeloteGame:new(doc))
    end
  end
  return games
end

-- Get player at seat
function BeloteGame:player_at_seat(seat)
  local players = self.players or self.data.players or {}
  for _, p in ipairs(players) do
    if p.seat == seat then
      return p
    end
  end
  return nil
end

-- Get player by user_key
function BeloteGame:player_by_key(user_key)
  local players = self.players or self.data.players or {}
  for _, p in ipairs(players) do
    if p.user_key == user_key then
      return p
    end
  end
  return nil
end

-- Get available seats
function BeloteGame:available_seats()
  local players = self.players or self.data.players or {}
  local taken = {}
  for _, p in ipairs(players) do
    taken[p.seat] = true
  end

  local available = {}
  for seat = 0, 3 do
    if not taken[seat] then
      table.insert(available, seat)
    end
  end
  return available
end

-- Check if all seats are filled
function BeloteGame:is_full()
  local players = self.players or self.data.players or {}
  return #players >= 4
end

-- Get player count
function BeloteGame:player_count()
  local players = self.players or self.data.players or {}
  return #players
end

-- Get team score
function BeloteGame:team_score(team)
  local scores = self.scores or self.data.scores or { team_a = 0, team_b = 0 }
  if team == "a" then
    return scores.team_a or 0
  else
    return scores.team_b or 0
  end
end

-- Check if game is over (team reached 501)
function BeloteGame:is_game_over()
  local scores = self.scores or self.data.scores or { team_a = 0, team_b = 0 }
  return (scores.team_a or 0) >= 501 or (scores.team_b or 0) >= 501
end

-- Get winner team
function BeloteGame:winner_team()
  local scores = self.scores or self.data.scores or { team_a = 0, team_b = 0 }
  if (scores.team_a or 0) >= 501 then
    return "a"
  elseif (scores.team_b or 0) >= 501 then
    return "b"
  end
  return nil
end

-- Get hand for seat
function BeloteGame:hand_for_seat(seat)
  local hands = self.hands or self.data.hands or {}
  return hands[tostring(seat)] or {}
end

-- Next player (clockwise)
function BeloteGame.next_seat(seat)
  return (seat + 1) % 4
end

-- Partner seat
function BeloteGame.partner_seat(seat)
  return (seat + 2) % 4
end

-- Check if two seats are partners
function BeloteGame.are_partners(seat1, seat2)
  return BeloteGame.TEAMS[seat1] == BeloteGame.TEAMS[seat2]
end

return BeloteGame
