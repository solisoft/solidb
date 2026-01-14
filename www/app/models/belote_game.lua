local Model = require("model")

local BeloteGame = Model.create("belote_games", {
  permitted_fields = { 
    "status", "created_by", "players", "scores", "current_turn", 
    "trump_suit", "trump_taker", "trick", "hands", "deck", "dealer",
    "turned_card", "bidding_round"
  },
  validations = {},
  before_create = { "initialize_game" }
})

-- Card definitions
local SUITS = { "hearts", "diamonds", "clubs", "spades" }
local RANKS = { "7", "8", "9", "10", "J", "Q", "K", "A" }
local SUIT_ICONS = { hearts = "♥", diamonds = "♦", clubs = "♣", spades = "♠" }

-- Suit display order (for sorting: spades, hearts, diamonds, clubs)
local SUIT_ORDER = { spades = 1, hearts = 2, diamonds = 3, clubs = 4 }

-- Rank order values (higher = stronger)
-- Non-trump: A(11) > 10(10) > K(4) > Q(3) > J(2) > 9(0) > 8(0) > 7(0)
local RANK_ORDER_NORMAL = { ["A"] = 8, ["10"] = 7, ["K"] = 6, ["Q"] = 5, ["J"] = 4, ["9"] = 3, ["8"] = 2, ["7"] = 1 }
-- Trump: J(20) > 9(14) > A(11) > 10(10) > K(4) > Q(3) > 8(0) > 7(0)
local RANK_ORDER_TRUMP = { ["J"] = 8, ["9"] = 7, ["A"] = 6, ["10"] = 5, ["K"] = 4, ["Q"] = 3, ["8"] = 2, ["7"] = 1 }

-- Sort a hand by suit then by rank
function BeloteGame:sort_hand(hand, trump_suit)
  if not hand or #hand == 0 then return end
  
  table.sort(hand, function(a, b)
    -- Get suit order (trump first if set, then standard)
    local function get_suit_order(card)
      if trump_suit and card.suit == trump_suit then
        return 0 -- Trump always first
      end
      return SUIT_ORDER[card.suit] or 99
    end
    
    -- Get rank order
    local function get_rank_order(card)
      if trump_suit and card.suit == trump_suit then
        return RANK_ORDER_TRUMP[card.rank] or 0
      end
      return RANK_ORDER_NORMAL[card.rank] or 0
    end
    
    local a_suit = get_suit_order(a)
    local b_suit = get_suit_order(b)
    
    if a_suit ~= b_suit then
      return a_suit < b_suit -- Lower suit order = earlier
    end
    
    -- Same suit: higher rank order = earlier (descending)
    return get_rank_order(a) > get_rank_order(b)
  end)
end

-- Sort all hands
function BeloteGame:sort_all_hands()
  local trump = self.data.trump_suit
  Log(kLogInfo, "BeloteGame: Sorting hands. Trump: " .. tostring(trump))
  
  for player_key, hand in pairs(self.data.hands) do
    if hand and #hand > 0 then
      self:sort_hand(hand, trump)
      -- Ensure sorted hand is stored back (table.sort modifies in place but let's be explicit)
      self.data.hands[player_key] = hand
      Log(kLogInfo, "BeloteGame: Sorted hand for " .. player_key .. ": " .. (hand[1] and hand[1].rank .. " of " .. hand[1].suit or "empty"))
    end
  end
end

-- Bot player keys
local BOT_KEYS = { "bot_1", "bot_2", "bot_3" }

-- Initialize new game
function BeloteGame.initialize_game(data)
  data.status = data.status or "waiting"
  data.players = data.players or {}
  data.scores = data.scores or { team1 = 0, team2 = 0 }
  data.current_turn = data.current_turn or 1
  data.trump_suit = nil
  data.trump_taker = nil
  data.turned_card = nil
  data.bidding_round = 1
  data.trick = data.trick or {}
  data.hands = data.hands or {}
  data.deck = data.deck or {}
  data.dealer = data.dealer or 1
  return data
end

-- Check if player is a bot
function BeloteGame:is_bot(player_key)
  return player_key and player_key:match("^bot_")
end

-- Add bots to fill remaining slots
function BeloteGame:add_bots()
  local bot_idx = 1
  local players = self.data.players or {}
  while #players < 4 and bot_idx <= #BOT_KEYS do
    table.insert(players, BOT_KEYS[bot_idx])
    bot_idx = bot_idx + 1
  end
  self.data.players = players
  self:save()
end

-- Join game
function BeloteGame:join(user_key)
  if #self.data.players >= 4 then return false, "Game is full" end
  for _, p in ipairs(self.data.players) do
    if p == user_key then return false, "Already joined" end
  end
  
  table.insert(self.data.players, user_key)
  self:save()
  return true
end

-- Start round: shuffle and deal first 5 cards, turn up trump card
function BeloteGame:start_round()
  Log(kLogInfo, "BeloteGame: Starting round for game " .. (self.data._key or "nil"))
  
  -- Add bots if needed
  self:add_bots()
  
  self.data.status = "bidding"
  self.data.bidding_round = 1
  self.data.deck = self:generate_deck()
  self.data.trick = {}
  self.data.trump_suit = nil
  self.data.trump_taker = nil
  
  -- Deal 5 cards to each player (3 + 2)
  self.data.hands = {}
  for i, player_key in ipairs(self.data.players) do
    self.data.hands[player_key] = {}
    -- Deal 3 cards
    for j = 1, 3 do
      table.insert(self.data.hands[player_key], table.remove(self.data.deck))
    end
    -- Deal 2 more cards
    for j = 1, 2 do
      table.insert(self.data.hands[player_key], table.remove(self.data.deck))
    end
  end
  
  -- Turn up one card for trump proposal
  self.data.turned_card = table.remove(self.data.deck)
  Log(kLogInfo, "BeloteGame: Turned card: " .. self.data.turned_card.rank .. " of " .. self.data.turned_card.suit)
  
  -- Set dealer to 4 so human (player 1) is first bidder
  self.data.dealer = 4
  
  -- First to bid is left of dealer (player 1 = human)
  self.data.current_turn = 1
  
  -- Sort hands for display (no trump yet during bidding)
  self:sort_all_hands()
  
  self:save()
  
  -- If first bidder is a bot, make it bid
  self:bot_bid()
end

-- Bot makes bidding decision
function BeloteGame:bot_bid()
  if self.data.status ~= "bidding" then return end
  
  local current_player = self.data.players[self.data.current_turn]
  if not self:is_bot(current_player) then return end
  
  -- Simple bot logic: take if round 1 and has Jack/9 of proposed suit, else pass
  local hand = self.data.hands[current_player]
  local should_take = false
  
  if self.data.bidding_round == 1 then
    -- Check if bot has Jack or 9 of turned card's suit
    local proposed_suit = self.data.turned_card.suit
    for _, card in ipairs(hand) do
      if card.suit == proposed_suit and (card.rank == "J" or card.rank == "9") then
        should_take = true
        break
      end
    end
    
    if should_take then
      self:take_trump(current_player)
    else
      self:pass_trump(current_player)
    end
  else
    -- Round 2: Bot picks a suit if it has strong cards, otherwise passes
    -- Find suit with most cards
    local suit_counts = {}
    for _, card in ipairs(hand) do
      suit_counts[card.suit] = (suit_counts[card.suit] or 0) + 1
    end
    
    local best_suit = nil
    local best_count = 0
    for suit, count in pairs(suit_counts) do
      if suit ~= self.data.turned_card.suit and count > best_count then
        best_suit = suit
        best_count = count
      end
    end
    
    if best_count >= 3 then
      self:take_trump(current_player, best_suit)
    else
      self:pass_trump(current_player)
    end
  end
end

-- Player takes the trump (accepts turned card in round 1, or calls a suit in round 2)
function BeloteGame:take_trump(user_key, called_suit)
  if self.data.status ~= "bidding" then return false, "Not in bidding phase" end
  
  local player_idx = self:get_player_index(user_key)
  if player_idx ~= self.data.current_turn then return false, "Not your turn" end
  
  if self.data.bidding_round == 1 then
    -- Round 1: Accept turned card's suit
    self.data.trump_suit = self.data.turned_card.suit
    -- Give turned card to the taker
    table.insert(self.data.hands[user_key], self.data.turned_card)
    self.data.turned_card = nil
  else
    -- Round 2: Call any suit (except turned card's original suit)
    if not called_suit then return false, "Must specify a suit" end
    self.data.trump_suit = called_suit
  end
  
  self.data.trump_taker = user_key
  Log(kLogInfo, "BeloteGame: Trump taken by " .. user_key .. " - Suit: " .. self.data.trump_suit)
  
  -- Finish dealing and start playing
  self:finish_dealing()
  
  return true
end

-- Player passes on trump
function BeloteGame:pass_trump(user_key)
  if self.data.status ~= "bidding" then return false, "Not in bidding phase" end
  
  local player_idx = self:get_player_index(user_key)
  if player_idx ~= self.data.current_turn then return false, "Not your turn" end
  
  -- Move to next player
  local next_turn = (self.data.current_turn % 4) + 1
  
  -- Check if we've gone around the table
  local first_bidder = (self.data.dealer % 4) + 1
  if next_turn == first_bidder then
    if self.data.bidding_round == 1 then
      -- Start round 2
      self.data.bidding_round = 2
      Log(kLogInfo, "BeloteGame: Round 1 complete, starting round 2")
    else
      -- Everyone passed twice - dealer must take (forced take)
      local dealer_key = self.data.players[self.data.dealer]
      Log(kLogInfo, "BeloteGame: All passed, dealer forced to take")
      self.data.trump_suit = self.data.turned_card.suit
      self.data.trump_taker = dealer_key
      table.insert(self.data.hands[dealer_key], self.data.turned_card)
      self.data.turned_card = nil
      self:finish_dealing()
      return true
    end
  end
  
  self.data.current_turn = next_turn
  self:save()
  
  -- If next bidder is a bot, make it bid
  self:bot_bid()
  
  return true
end

-- Finish dealing (deal 3 more cards to each player, 2 to taker if they got turned card)
function BeloteGame:finish_dealing()
  Log(kLogInfo, "BeloteGame: Finishing dealing")
  
  for i, player_key in ipairs(self.data.players) do
    local cards_to_deal = 3
    -- Taker already got the turned card, so only needs 2 more
    if player_key == self.data.trump_taker and self.data.bidding_round == 1 then
      cards_to_deal = 2
    end
    
    for j = 1, cards_to_deal do
      if #self.data.deck > 0 then
        table.insert(self.data.hands[player_key], table.remove(self.data.deck))
      end
    end
  end
  
  -- Start playing
  self.data.status = "playing"
  -- First to play is left of dealer
  self.data.current_turn = (self.data.dealer % 4) + 1
  
  -- Sort all hands with trump ordering
  self:sort_all_hands()
  
  self:save()
  
  -- If first player is a bot, make it play
  self:play_bot_turn()
end

-- Generate and shuffle deck
function BeloteGame:generate_deck()
  local deck = {}
  for _, suit in ipairs(SUITS) do
    for _, rank in ipairs(RANKS) do
      table.insert(deck, { suit = suit, rank = rank })
    end
  end
  
  -- Shuffle
  math.randomseed(os.time())
  for i = #deck, 2, -1 do
    local j = math.random(i)
    deck[i], deck[j] = deck[j], deck[i]
  end
  
  return deck
end

-- Bot plays a random valid card
function BeloteGame:play_bot_turn()
  if self.data.status ~= "playing" then return end
  
  local current_player = self.data.players[self.data.current_turn]
  if not self:is_bot(current_player) then return end
  
  local hand = self.data.hands[current_player]
  if not hand or #hand == 0 then return end
  
  -- Bot plays first card (could be smarter, but this is demo)
  local card_idx = 1
  local card = table.remove(hand, card_idx)
  table.insert(self.data.trick, { card = card, player = current_player })
  
  if #self.data.trick == 4 then
    self:resolve_trick()
  else
    self.data.current_turn = (self.data.current_turn % 4) + 1
  end
  
  self:save()
  
  -- Chain bot turns
  self:play_bot_turn()
end

-- Play a card (human player)
function BeloteGame:play_card(user_key, card_idx)
  if self.data.status ~= "playing" then 
    return false, "Game not in playing phase" 
  end
  
  local player_idx = self:get_player_index(user_key)
  if player_idx ~= self.data.current_turn then 
    return false, "Not your turn" 
  end
  
  local hand = self.data.hands[user_key]
  if not hand then
    return false, "Invalid hand"
  end
  
  if not hand[card_idx] then 
    return false, "Invalid card" 
  end
  
  local card = table.remove(hand, card_idx)
  table.insert(self.data.trick, { card = card, player = user_key })
  
  if #self.data.trick == 4 then
    self:resolve_trick()
  else
    self.data.current_turn = (self.data.current_turn % 4) + 1
  end
  
  self:save()
  
  -- After human plays, bots play if it's their turn
  self:play_bot_turn()
  
  return true
end

-- Resolve trick
function BeloteGame:resolve_trick()
  -- Simplified: just clear trick and rotate
  self.data.trick = {}
  
  -- Check if round over
  local round_over = true
  for _, hand in pairs(self.data.hands) do
    if #hand > 0 then round_over = false break end
  end
  
  if round_over then
    self.data.status = "finished"
  end
end

-- Get player index
function BeloteGame:get_player_index(user_key)
  if not self.data.players then return nil end
  for i, p in ipairs(self.data.players) do
    if p == user_key then return i end
  end
  return nil
end

return BeloteGame
