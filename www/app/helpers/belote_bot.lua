-- Belote Bot AI
-- Simple but reasonable strategy for bidding and card play

local BeloteBot = {}
local BeloteGame = require("models.belote_game")
local BeloteEngine = require("helpers.belote_engine")

-- Decide whether to take or pass during bidding
function BeloteBot.decide_bid(hand, turned_card, passes, seat)
  local trump_suit = turned_card.suit
  local trump_count = BeloteBot.count_suit(hand, trump_suit)

  -- Calculate hand strength
  local has_trump_jack = BeloteBot.has_card(hand, trump_suit, "J")
  local has_trump_nine = BeloteBot.has_card(hand, trump_suit, "9")
  local ace_count = BeloteBot.count_aces(hand)

  local score = 0

  -- Trump strength
  if has_trump_jack then score = score + 35 end
  if has_trump_nine then score = score + 20 end
  score = score + trump_count * 8

  -- Non-trump strength
  score = score + ace_count * 12

  -- Long suits are good
  for _, suit in ipairs(BeloteGame.SUITS) do
    local count = BeloteBot.count_suit(hand, suit)
    if count >= 4 then
      score = score + 10
    end
  end

  -- Be more aggressive in later positions (more info)
  -- First bidder needs 50, fourth needs only 35
  local position = passes % 4
  local threshold = 50 - (position * 5)

  -- Second round (passes >= 4) - can choose any suit, be more careful
  if passes >= 4 then
    -- Find best suit to call
    local best_suit, best_score = BeloteBot.find_best_trump(hand)
    if best_score >= 45 then
      return { action = "take", suit = best_suit }
    else
      return { action = "pass" }
    end
  end

  if score >= threshold then
    return { action = "take", suit = trump_suit }
  else
    return { action = "pass" }
  end
end

-- Find the best suit to call as trump
function BeloteBot.find_best_trump(hand)
  local best_suit = nil
  local best_score = 0

  for _, suit in ipairs(BeloteGame.SUITS) do
    local score = 0
    local count = BeloteBot.count_suit(hand, suit)

    if BeloteBot.has_card(hand, suit, "J") then score = score + 35 end
    if BeloteBot.has_card(hand, suit, "9") then score = score + 20 end
    score = score + count * 10

    if score > best_score then
      best_score = score
      best_suit = suit
    end
  end

  return best_suit, best_score
end

-- Choose which card to play
function BeloteBot.choose_card(hand, game_state)
  local trick = game_state.current_trick or {}
  local trump = game_state.trump_suit
  local my_seat = game_state.my_seat
  local valid = BeloteEngine.get_valid_plays(hand, trick, trump, my_seat)

  if #valid == 0 then
    return nil
  end

  if #valid == 1 then
    return valid[1]
  end

  -- If leading, play strategically
  if #trick == 0 then
    return BeloteBot.choose_lead(valid, hand, trump, game_state)
  end

  -- If following, try to win or dump
  return BeloteBot.choose_follow(valid, trick, trump, game_state)
end

-- Choose card when leading
function BeloteBot.choose_lead(valid, hand, trump, game_state)
  -- Strategy: Lead with Aces first, then high non-trump, avoid leading trump early

  -- First try non-trump Aces
  for _, card in ipairs(valid) do
    if card.value == "A" and card.suit ~= trump then
      return card
    end
  end

  -- Try leading 10s in suits where we have the Ace
  for _, card in ipairs(valid) do
    if card.value == "10" and card.suit ~= trump then
      if BeloteBot.has_card(hand, card.suit, "A") then
        -- We have the Ace too, safe to lead 10
      else
        -- Try high cards in long suits
        local suit_cards = BeloteEngine.cards_of_suit(valid, card.suit)
        if #suit_cards >= 3 then
          return card
        end
      end
    end
  end

  -- Play non-trump cards, preferring high cards
  local non_trump = {}
  for _, card in ipairs(valid) do
    if card.suit ~= trump then
      table.insert(non_trump, card)
    end
  end

  if #non_trump > 0 then
    -- Sort by rank (high to low)
    table.sort(non_trump, function(a, b)
      return BeloteBot.card_strength(a, trump) > BeloteBot.card_strength(b, trump)
    end)
    return non_trump[1]
  end

  -- Only trump left - play lowest
  table.sort(valid, function(a, b)
    return BeloteBot.card_strength(a, trump) < BeloteBot.card_strength(b, trump)
  end)
  return valid[1]
end

-- Choose card when following
function BeloteBot.choose_follow(valid, trick, trump, game_state)
  local my_seat = game_state.my_seat
  local lead_card = trick[1].card

  -- Find current winning card and seat
  local winner_seat, winner_card = BeloteBot.find_trick_winner(trick, trump)
  local partner_winning = BeloteBot.is_partner(winner_seat, my_seat)

  -- Calculate trick points
  local trick_points = 0
  for _, play in ipairs(trick) do
    trick_points = trick_points + BeloteEngine.card_points(play.card, trump)
  end

  -- If partner is winning with high value trick, play low
  if partner_winning and trick_points >= 15 then
    return BeloteBot.play_lowest(valid, trump)
  end

  -- Try to win the trick if it has value
  if trick_points >= 10 or not partner_winning then
    local winners = {}
    for _, card in ipairs(valid) do
      if BeloteBot.would_win(card, winner_card, trump, lead_card.suit) then
        table.insert(winners, card)
      end
    end

    if #winners > 0 then
      -- Play the cheapest winning card
      table.sort(winners, function(a, b)
        return BeloteEngine.card_points(a, trump) < BeloteEngine.card_points(b, trump)
      end)
      return winners[1]
    end
  end

  -- Can't win or partner winning - play lowest value card
  return BeloteBot.play_lowest(valid, trump)
end

-- Find current trick winner
function BeloteBot.find_trick_winner(trick, trump)
  local lead_suit = trick[1].card.suit
  local winner_seat = trick[1].seat
  local winner_card = trick[1].card
  local is_winner_trump = winner_card.suit == trump

  for i = 2, #trick do
    local play = trick[i]
    local card = play.card
    local is_trump_card = card.suit == trump

    local beats = false
    if is_trump_card and not is_winner_trump then
      beats = true
    elseif is_trump_card and is_winner_trump then
      if BeloteEngine.trump_rank(card) > BeloteEngine.trump_rank(winner_card) then
        beats = true
      end
    elseif not is_trump_card and not is_winner_trump and card.suit == lead_suit then
      if BeloteEngine.non_trump_rank(card) > BeloteEngine.non_trump_rank(winner_card) then
        beats = true
      end
    end

    if beats then
      winner_seat = play.seat
      winner_card = card
      is_winner_trump = is_trump_card
    end
  end

  return winner_seat, winner_card
end

-- Check if a card would win against current winner
function BeloteBot.would_win(card, winner_card, trump, lead_suit)
  local card_is_trump = card.suit == trump
  local winner_is_trump = winner_card.suit == trump

  if card_is_trump and not winner_is_trump then
    return true
  elseif card_is_trump and winner_is_trump then
    return BeloteEngine.trump_rank(card) > BeloteEngine.trump_rank(winner_card)
  elseif not card_is_trump and not winner_is_trump and card.suit == lead_suit then
    return BeloteEngine.non_trump_rank(card) > BeloteEngine.non_trump_rank(winner_card)
  end
  return false
end

-- Play lowest value card
function BeloteBot.play_lowest(cards, trump)
  table.sort(cards, function(a, b)
    -- Sort by point value first, then by rank
    local a_pts = BeloteEngine.card_points(a, trump)
    local b_pts = BeloteEngine.card_points(b, trump)
    if a_pts ~= b_pts then
      return a_pts < b_pts
    end
    return BeloteBot.card_strength(a, trump) < BeloteBot.card_strength(b, trump)
  end)
  return cards[1]
end

-- Helper: Count cards of a suit
function BeloteBot.count_suit(hand, suit)
  local count = 0
  for _, card in ipairs(hand) do
    if card.suit == suit then count = count + 1 end
  end
  return count
end

-- Helper: Check if hand has specific card
function BeloteBot.has_card(hand, suit, value)
  for _, card in ipairs(hand) do
    if card.suit == suit and card.value == value then
      return true
    end
  end
  return false
end

-- Helper: Count aces in hand
function BeloteBot.count_aces(hand)
  local count = 0
  for _, card in ipairs(hand) do
    if card.value == "A" then count = count + 1 end
  end
  return count
end

-- Helper: Check if two seats are partners
function BeloteBot.is_partner(seat1, seat2)
  return BeloteGame.TEAMS[seat1] == BeloteGame.TEAMS[seat2]
end

-- Helper: Get card strength (for sorting)
function BeloteBot.card_strength(card, trump)
  if card.suit == trump then
    return 100 + (BeloteGame.TRUMP_RANK[card.value] or 0)
  else
    return BeloteGame.NON_TRUMP_RANK[card.value] or 0
  end
end

return BeloteBot
