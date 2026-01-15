-- Belote Game Engine
-- Core game logic for deck, dealing, bidding, play validation, and scoring

local BeloteEngine = {}
local BeloteGame = require("models.belote_game")

-- Create a standard 32-card Belote deck
function BeloteEngine.create_deck()
  local deck = {}
  for _, suit in ipairs(BeloteGame.SUITS) do
    for _, value in ipairs(BeloteGame.VALUES) do
      table.insert(deck, { suit = suit, value = value })
    end
  end
  return deck
end

-- Shuffle deck using Fisher-Yates algorithm
function BeloteEngine.shuffle_deck(deck)
  math.randomseed(os.time() + math.random(1000000))
  for i = #deck, 2, -1 do
    local j = math.random(i)
    deck[i], deck[j] = deck[j], deck[i]
  end
  return deck
end

-- Deal cards (3+2 pattern in Belote = 5 cards before bidding)
-- Returns hands table and remaining deck
function BeloteEngine.deal_cards(deck, dealer)
  local hands = { ["0"] = {}, ["1"] = {}, ["2"] = {}, ["3"] = {} }
  local current = BeloteGame.next_seat(dealer)

  -- First round: 3 cards each
  for _ = 1, 3 do
    for _ = 1, 4 do
      table.insert(hands[tostring(current)], table.remove(deck, 1))
      current = BeloteGame.next_seat(current)
    end
  end

  -- Turn up one card for trump proposal
  local turned_card = table.remove(deck, 1)

  -- Second round: 2 cards each (before bidding - players see 5 cards)
  for _ = 1, 2 do
    for _ = 1, 4 do
      table.insert(hands[tostring(current)], table.remove(deck, 1))
      current = BeloteGame.next_seat(current)
    end
  end

  return hands, deck, turned_card
end

-- Deal remaining cards after bidding (3 cards each, turned card goes to taker)
-- Players have 5 cards, need 3 more for total of 8
function BeloteEngine.deal_remaining(game)
  local deck = game.deck or {}
  local hands = game.hands or {}
  local trump_chooser = game.trump_chooser
  local turned_card = game.turned_card
  local dealer = game.dealer

  -- Give turned card to trump chooser
  if turned_card and trump_chooser ~= nil then
    table.insert(hands[tostring(trump_chooser)], turned_card)
  end

  -- Deal remaining cards: 3 to non-takers, 2 to taker (who got turned card)
  local current = BeloteGame.next_seat(dealer)
  for seat = 0, 3 do
    local cards_to_deal = 3
    if current == trump_chooser then
      cards_to_deal = 2  -- Taker already got turned card
    end
    for _ = 1, cards_to_deal do
      if #deck > 0 then
        table.insert(hands[tostring(current)], table.remove(deck, 1))
      end
    end
    current = BeloteGame.next_seat(current)
  end

  return hands, deck
end

-- Join a game
function BeloteEngine.join_game(game, user_key, seat, is_bot, name)
  local players = game.players or {}

  -- Check if seat is available
  for _, p in ipairs(players) do
    if p.seat == seat then
      return { error = "Seat already taken" }
    end
    if p.user_key == user_key and not is_bot then
      return { error = "You're already in this game" }
    end
  end

  -- Add player
  table.insert(players, {
    seat = seat,
    user_key = user_key,
    name = name or (is_bot and ("Bot " .. (seat + 1)) or "Player"),
    is_bot = is_bot,
    team = BeloteGame.TEAMS[seat]
  })

  -- Update game
  Sdb:Sdbql(
    "FOR g IN belote_games FILTER g._key == @key UPDATE g WITH { players: @players, updated_at: @now } IN belote_games",
    { key = game._key, players = players, now = os.time() }
  )

  game.players = players
  return { success = true, game = game }
end

-- Leave a game
function BeloteEngine.leave_game(game, user_key)
  local players = game.players or {}
  local new_players = {}

  for _, p in ipairs(players) do
    if p.user_key ~= user_key then
      table.insert(new_players, p)
    end
  end

  -- Update game
  Sdb:Sdbql(
    "FOR g IN belote_games FILTER g._key == @key UPDATE g WITH { players: @players, updated_at: @now } IN belote_games",
    { key = game._key, players = new_players, now = os.time() }
  )

  game.players = new_players
  return { success = true }
end

-- Add a bot to a seat
function BeloteEngine.add_bot(game, seat)
  local bot_id = "bot_" .. game._key .. "_" .. seat
  local bot_name = "Bot " .. BeloteGame.SEAT_NAMES[seat]

  return BeloteEngine.join_game(game, bot_id, seat, true, bot_name)
end

-- Start the game (deal cards, begin bidding)
function BeloteEngine.start_game(game)
  if #(game.players or {}) < 4 then
    return { error = "Need 4 players" }
  end

  -- Create and shuffle deck
  local deck = BeloteEngine.create_deck()
  deck = BeloteEngine.shuffle_deck(deck)

  -- Deal initial cards (3 each)
  local hands, remaining_deck, turned_card = BeloteEngine.deal_cards(deck, game.dealer)

  -- First bidder is left of dealer
  local first_bidder = BeloteGame.next_seat(game.dealer)

  -- Update game state
  local now = os.time()
  Sdb:Sdbql([[
    FOR g IN belote_games FILTER g._key == @key
    UPDATE g WITH {
      state: @state,
      deck: @deck,
      hands: @hands,
      turned_card: @turned_card,
      bidding_seat: @bidding_seat,
      bidding_passed: [],
      current_trick: [],
      tricks_won: { team_a: 0, team_b: 0 },
      round_points: { team_a: 0, team_b: 0 },
      started_at: @started_at,
      updated_at: @updated_at
    } IN belote_games
  ]], {
    key = game._key,
    state = BeloteGame.STATES.BIDDING,
    deck = remaining_deck,
    hands = hands,
    turned_card = turned_card,
    bidding_seat = first_bidder,
    started_at = now,
    updated_at = now
  })

  return {
    success = true,
    state = BeloteGame.STATES.BIDDING,
    turned_card = turned_card,
    bidding_seat = first_bidder
  }
end

-- Process a bid action (take or pass)
function BeloteEngine.process_bid(game, seat, action, suit)
  local bidding_passed = game.bidding_passed or {}
  local turned_card = game.turned_card

  if action == "take" then
    -- Player takes - set trump and move to playing
    local trump_suit = suit or (turned_card and turned_card.suit)

    -- Deal remaining cards
    game.trump_chooser = seat
    game.trump_suit = trump_suit
    local hands, deck = BeloteEngine.deal_remaining(game)

    -- First player is left of dealer
    local first_player = BeloteGame.next_seat(game.dealer)

    Sdb:Sdbql([[
      FOR g IN belote_games FILTER g._key == @key
      UPDATE g WITH {
        state: @state,
        trump_suit: @trump_suit,
        trump_chooser: @trump_chooser,
        hands: @hands,
        deck: @deck,
        current_player: @current_player,
        trick_lead: @trick_lead,
        turned_card: null,
        updated_at: @updated_at
      } IN belote_games
    ]], {
      key = game._key,
      state = BeloteGame.STATES.PLAYING,
      trump_suit = trump_suit,
      trump_chooser = seat,
      hands = hands,
      deck = deck,
      current_player = first_player,
      trick_lead = first_player,
      updated_at = os.time()
    })

    return {
      success = true,
      action = "take",
      trump_suit = trump_suit,
      state = BeloteGame.STATES.PLAYING,
      current_player = first_player
    }
  else
    -- Player passes
    table.insert(bidding_passed, seat)

    -- Check if all 4 have passed - redeal with new cards
    if #bidding_passed >= 4 then
      -- Re-deal with fresh cards
      return BeloteEngine.redeal(game)
    end

    -- Move to next bidder
    local next_bidder = BeloteGame.next_seat(seat)

    Sdb:Sdbql([[
      FOR g IN belote_games FILTER g._key == @key
      UPDATE g WITH {
        bidding_passed: @bidding_passed,
        bidding_seat: @bidding_seat,
        updated_at: @updated_at
      } IN belote_games
    ]], {
      key = game._key,
      bidding_passed = bidding_passed,
      bidding_seat = next_bidder,
      updated_at = os.time()
    })

    return {
      success = true,
      action = "pass",
      bidding_seat = next_bidder,
      passes = #bidding_passed
    }
  end
end

-- Re-deal after all pass
function BeloteEngine.redeal(game)
  -- Move dealer to next player
  local new_dealer = BeloteGame.next_seat(game.dealer)

  -- Create and shuffle new deck
  local deck = BeloteEngine.create_deck()
  deck = BeloteEngine.shuffle_deck(deck)

  -- Deal new cards
  local hands, remaining_deck, turned_card = BeloteEngine.deal_cards(deck, new_dealer)
  local first_bidder = BeloteGame.next_seat(new_dealer)

  Sdb:Sdbql([[
    FOR g IN belote_games FILTER g._key == @key
    UPDATE g WITH {
      dealer: @dealer,
      deck: @deck,
      hands: @hands,
      turned_card: @turned_card,
      bidding_seat: @bidding_seat,
      bidding_passed: [],
      trump_suit: null,
      trump_chooser: null,
      updated_at: @updated_at
    } IN belote_games
  ]], {
    key = game._key,
    dealer = new_dealer,
    deck = remaining_deck,
    hands = hands,
    turned_card = turned_card,
    bidding_seat = first_bidder,
    updated_at = os.time()
  })

  -- Check if first bidder is a bot
  local needs_bot_turn = false
  for _, p in ipairs(game.players or {}) do
    if p.seat == first_bidder and p.is_bot then
      needs_bot_turn = true
      break
    end
  end

  return {
    success = true,
    action = "redeal",
    dealer = new_dealer,
    bidding_seat = first_bidder,
    turned_card = turned_card,
    needs_bot_turn = needs_bot_turn
  }
end

-- Check if a card is in a hand
function BeloteEngine.has_card(hand, card)
  for _, c in ipairs(hand) do
    if c.suit == card.suit and c.value == card.value then
      return true
    end
  end
  return false
end

-- Remove a card from hand
function BeloteEngine.remove_card(hand, card)
  for i, c in ipairs(hand) do
    if c.suit == card.suit and c.value == card.value then
      table.remove(hand, i)
      return true
    end
  end
  return false
end

-- Get cards of a specific suit in hand
function BeloteEngine.cards_of_suit(hand, suit)
  local cards = {}
  for _, c in ipairs(hand) do
    if c.suit == suit then
      table.insert(cards, c)
    end
  end
  return cards
end

-- Get current trick winner (seat that's currently winning)
function BeloteEngine.get_trick_winner(trick, trump_suit)
  if #trick == 0 then
    return nil
  end

  local lead_suit = trick[1].card.suit
  local winner_seat = trick[1].seat
  local winner_card = trick[1].card
  local is_winner_trump = winner_card.suit == trump_suit

  for i = 2, #trick do
    local play = trick[i]
    local card = play.card
    local is_trump = card.suit == trump_suit

    local beats_winner = false

    if is_trump and not is_winner_trump then
      beats_winner = true
    elseif is_trump and is_winner_trump then
      if BeloteEngine.trump_rank(card) > BeloteEngine.trump_rank(winner_card) then
        beats_winner = true
      end
    elseif not is_trump and not is_winner_trump and card.suit == lead_suit then
      if BeloteEngine.non_trump_rank(card) > BeloteEngine.non_trump_rank(winner_card) then
        beats_winner = true
      end
    end

    if beats_winner then
      winner_seat = play.seat
      winner_card = card
      is_winner_trump = is_trump
    end
  end

  return winner_seat
end

-- Check if two seats are partners (same team)
function BeloteEngine.are_partners(seat1, seat2)
  return BeloteGame.TEAMS[seat1] == BeloteGame.TEAMS[seat2]
end

-- Get valid cards that can be played
function BeloteEngine.get_valid_plays(hand, trick, trump_suit, current_seat)
  -- If leading, all cards are valid
  if #trick == 0 then
    return hand
  end

  local lead_suit = trick[1].card.suit
  local same_suit = BeloteEngine.cards_of_suit(hand, lead_suit)

  -- Must follow suit if possible
  if #same_suit > 0 then
    return same_suit
  end

  -- Can't follow suit - check if partner is winning
  local trick_winner = BeloteEngine.get_trick_winner(trick, trump_suit)
  local partner_winning = current_seat and trick_winner and BeloteEngine.are_partners(current_seat, trick_winner)

  -- If partner is winning, can play any card (no trump obligation)
  if partner_winning then
    return hand
  end

  -- Opponent is winning - must trump if possible
  local trumps = BeloteEngine.cards_of_suit(hand, trump_suit)
  if #trumps > 0 then
    -- Must play higher trump if possible (Belote rule)
    local highest_trump = BeloteEngine.highest_trump_in_trick(trick, trump_suit)
    if highest_trump then
      local higher_trumps = {}
      for _, card in ipairs(trumps) do
        if BeloteEngine.trump_rank(card) > BeloteEngine.trump_rank(highest_trump) then
          table.insert(higher_trumps, card)
        end
      end
      if #higher_trumps > 0 then
        return higher_trumps
      end
    end
    return trumps
  end

  -- Can't follow suit or trump - play anything
  return hand
end

-- Get highest trump played in trick
function BeloteEngine.highest_trump_in_trick(trick, trump_suit)
  local highest = nil
  local highest_rank = 0
  for _, play in ipairs(trick) do
    if play.card.suit == trump_suit then
      local rank = BeloteEngine.trump_rank(play.card)
      if rank > highest_rank then
        highest_rank = rank
        highest = play.card
      end
    end
  end
  return highest
end

-- Get trump rank value
function BeloteEngine.trump_rank(card)
  return BeloteGame.TRUMP_RANK[card.value] or 0
end

-- Get non-trump rank value
function BeloteEngine.non_trump_rank(card)
  return BeloteGame.NON_TRUMP_RANK[card.value] or 0
end

-- Validate if a card play is legal
function BeloteEngine.is_valid_play(hand, card, trick, trump_suit, seat)
  local valid = BeloteEngine.get_valid_plays(hand, trick, trump_suit, seat)
  for _, v in ipairs(valid) do
    if v.suit == card.suit and v.value == card.value then
      return true
    end
  end
  return false
end

-- Play a card
function BeloteEngine.play_card(game, seat, card)
  local hands = game.hands or {}
  local hand = hands[tostring(seat)] or {}
  local trick = game.current_trick or {}
  local trump_suit = game.trump_suit

  -- Validate card is in hand
  if not BeloteEngine.has_card(hand, card) then
    return { error = "Card not in hand" }
  end

  -- Validate play is legal
  if not BeloteEngine.is_valid_play(hand, card, trick, trump_suit, seat) then
    return { error = "Illegal play" }
  end

  -- Remove card from hand and add to trick
  BeloteEngine.remove_card(hand, card)
  table.insert(trick, { seat = seat, card = card })
  hands[tostring(seat)] = hand

  -- Check if trick is complete (4 cards)
  if #trick >= 4 then
    return BeloteEngine.complete_trick(game, hands, trick)
  end

  -- Next player
  local next_player = BeloteGame.next_seat(seat)

  Sdb:Sdbql([[
    FOR g IN belote_games FILTER g._key == @key
    UPDATE g WITH {
      hands: @hands,
      current_trick: @trick,
      current_player: @next_player,
      updated_at: @updated_at
    } IN belote_games
  ]], {
    key = game._key,
    hands = hands,
    trick = trick,
    next_player = next_player,
    updated_at = os.time()
  })

  return {
    success = true,
    card = card,
    seat = seat,
    current_player = next_player,
    trick_complete = false
  }
end

-- Complete a trick (determine winner, award points)
function BeloteEngine.complete_trick(game, hands, trick)
  local trump_suit = game.trump_suit
  local lead_suit = trick[1].card.suit

  -- Determine winner
  local winner_seat = trick[1].seat
  local winner_card = trick[1].card
  local is_winner_trump = winner_card.suit == trump_suit

  for i = 2, #trick do
    local play = trick[i]
    local card = play.card
    local is_trump = card.suit == trump_suit

    local beats_winner = false

    if is_trump and not is_winner_trump then
      -- Trump beats non-trump
      beats_winner = true
    elseif is_trump and is_winner_trump then
      -- Both trump - higher rank wins
      if BeloteEngine.trump_rank(card) > BeloteEngine.trump_rank(winner_card) then
        beats_winner = true
      end
    elseif not is_trump and not is_winner_trump and card.suit == lead_suit then
      -- Same suit (non-trump) - higher rank wins
      if BeloteEngine.non_trump_rank(card) > BeloteEngine.non_trump_rank(winner_card) then
        beats_winner = true
      end
    end
    -- Cards not following suit (and not trump) never win

    if beats_winner then
      winner_seat = play.seat
      winner_card = card
      is_winner_trump = is_trump
    end
  end

  -- Calculate points for this trick
  local points = 0
  for _, play in ipairs(trick) do
    points = points + BeloteEngine.card_points(play.card, trump_suit)
  end

  -- Update tricks won
  local tricks_won = game.tricks_won or { team_a = 0, team_b = 0 }
  local winner_team = BeloteGame.TEAMS[winner_seat]
  if winner_team == "a" then
    tricks_won.team_a = tricks_won.team_a + 1
  else
    tricks_won.team_b = tricks_won.team_b + 1
  end

  -- Update round points
  local round_points = game.round_points or { team_a = 0, team_b = 0 }
  if winner_team == "a" then
    round_points.team_a = round_points.team_a + points
  else
    round_points.team_b = round_points.team_b + points
  end

  -- Check if round is over (8 tricks played)
  local total_tricks = tricks_won.team_a + tricks_won.team_b
  if total_tricks >= 8 then
    -- Last trick bonus (10 points)
    if winner_team == "a" then
      round_points.team_a = round_points.team_a + 10
    else
      round_points.team_b = round_points.team_b + 10
    end
    return BeloteEngine.complete_round(game, hands, tricks_won, round_points, winner_seat)
  end

  -- Start new trick with winner leading
  Sdb:Sdbql([[
    FOR g IN belote_games FILTER g._key == @key
    UPDATE g WITH {
      hands: @hands,
      current_trick: [],
      last_trick: @last_trick,
      current_player: @winner,
      trick_lead: @winner,
      tricks_won: @tricks_won,
      round_points: @round_points,
      last_trick_winner: @winner,
      updated_at: @updated_at
    } IN belote_games
  ]], {
    key = game._key,
    hands = hands,
    last_trick = trick,
    winner = winner_seat,
    tricks_won = tricks_won,
    round_points = round_points,
    updated_at = os.time()
  })

  return {
    success = true,
    trick_complete = true,
    winner_seat = winner_seat,
    points = points,
    current_player = winner_seat
  }
end

-- Complete a round (score and check for game end)
function BeloteEngine.complete_round(game, hands, tricks_won, round_points, last_winner)
  local trump_chooser = game.trump_chooser
  local chooser_team = BeloteGame.TEAMS[trump_chooser]

  -- Check for "capot" (one team took all tricks)
  local capot_team = nil
  if tricks_won.team_a == 8 then
    capot_team = "a"
    round_points = { team_a = 252, team_b = 0 }  -- All 162 points + 90 capot bonus
  elseif tricks_won.team_b == 8 then
    capot_team = "b"
    round_points = { team_a = 0, team_b = 252 }
  end

  -- Check if trump chooser's team won (scored more)
  local scores = game.scores or { team_a = 0, team_b = 0 }

  if not capot_team then
    -- Normal scoring
    if chooser_team == "a" then
      if round_points.team_a > round_points.team_b then
        -- Team A (choosers) won
        scores.team_a = scores.team_a + round_points.team_a
        scores.team_b = scores.team_b + round_points.team_b
      else
        -- Team A (choosers) lost - "dedans" - opponents get all points
        scores.team_b = scores.team_b + round_points.team_a + round_points.team_b
      end
    else
      if round_points.team_b > round_points.team_a then
        -- Team B (choosers) won
        scores.team_a = scores.team_a + round_points.team_a
        scores.team_b = scores.team_b + round_points.team_b
      else
        -- Team B (choosers) lost - opponents get all points
        scores.team_a = scores.team_a + round_points.team_a + round_points.team_b
      end
    end
  else
    -- Capot scoring
    scores.team_a = scores.team_a + round_points.team_a
    scores.team_b = scores.team_b + round_points.team_b
  end

  -- Check for game over (501 points)
  local game_over = scores.team_a >= 501 or scores.team_b >= 501
  local new_state = game_over and BeloteGame.STATES.FINISHED or BeloteGame.STATES.SCORING

  local now = os.time()
  local update_data = {
    state = new_state,
    scores = scores,
    hands = {},
    current_trick = {},
    tricks_won = { team_a = 0, team_b = 0 },
    round_points = { team_a = 0, team_b = 0 },
    trump_suit = nil,
    trump_chooser = nil,
    last_trick_winner = last_winner,
    updated_at = now
  }

  if game_over then
    update_data.finished_at = now
  else
    -- Prepare for next round
    update_data.dealer = BeloteGame.next_seat(game.dealer)
  end

  Sdb:Sdbql([[
    FOR g IN belote_games FILTER g._key == @key
    UPDATE g WITH @data IN belote_games
  ]], {
    key = game._key,
    data = update_data
  })

  if game_over then
    local winner = scores.team_a >= 501 and "a" or "b"
    return {
      success = true,
      round_complete = true,
      game_over = true,
      scores = scores,
      winner_team = winner
    }
  end

  return {
    success = true,
    round_complete = true,
    scores = scores,
    next_dealer = update_data.dealer
  }
end

-- Get card point value
function BeloteEngine.card_points(card, trump_suit)
  if card.suit == trump_suit then
    return BeloteGame.TRUMP_POINTS[card.value] or 0
  else
    return BeloteGame.NON_TRUMP_POINTS[card.value] or 0
  end
end

-- Start a new round (for continuing games)
function BeloteEngine.start_new_round(game)
  return BeloteEngine.start_game(game)
end

-- Process a single bot turn (for client-driven delays)
-- Returns info about what happened and whether another bot turn is pending
function BeloteEngine.process_single_bot_turn(game_key)
  local BeloteBot = require("helpers.belote_bot")

  -- Reload game state
  local result = Sdb:Sdbql(
    "FOR g IN belote_games FILTER g._key == @key RETURN g",
    { key = game_key }
  )
  if not result or not result.result or not result.result[1] then
    return { error = "Game not found" }
  end
  local game = result.result[1]

  -- Handle scoring state - start next round
  if game.state == "scoring" then
    game._key = game_key
    local round_result = BeloteEngine.start_new_round(game)
    if round_result.error then
      return round_result
    end
    return { success = true, action = "new_round", needs_bot_turn = true }
  end

  -- Check if it's a bot's turn
  local current_seat = nil
  if game.state == "bidding" then
    current_seat = game.bidding_seat
  elseif game.state == "playing" then
    current_seat = game.current_player
  else
    return { success = true, state = game.state, needs_bot_turn = false }
  end

  -- Find player at current seat
  local is_bot = false
  local bot_name = nil
  for _, p in ipairs(game.players or {}) do
    if p.seat == current_seat and p.is_bot then
      is_bot = true
      bot_name = p.name
      break
    end
  end

  if not is_bot then
    return { success = true, state = game.state, waiting_for_human = true, needs_bot_turn = false }
  end

  -- Bot's turn - make a decision
  local hand = game.hands and game.hands[tostring(current_seat)] or {}
  local action_result = nil

  if game.state == "bidding" then
    local passes = game.bidding_passed and #game.bidding_passed or 0
    local decision = BeloteBot.decide_bid(hand, game.turned_card, passes, current_seat)

    local bid_result = BeloteEngine.process_bid(
      { _key = game_key, bidding_passed = game.bidding_passed or {}, turned_card = game.turned_card,
        trump_chooser = game.trump_chooser, trump_suit = game.trump_suit, dealer = game.dealer,
        hands = game.hands, deck = game.deck, players = game.players },
      current_seat,
      decision.action,
      decision.suit
    )

    if bid_result.error then
      return bid_result
    end

    -- Check if this was a redeal (all passed)
    if bid_result.action == "redeal" then
      return {
        success = true,
        action = "redeal",
        seat = current_seat,
        bot_name = bot_name,
        needs_bot_turn = bid_result.needs_bot_turn
      }
    end

    action_result = {
      success = true,
      action = "bid",
      seat = current_seat,
      bot_name = bot_name,
      bid_action = decision.action,
      bid_suit = decision.suit
    }

  elseif game.state == "playing" then
    local game_state = {
      current_trick = game.current_trick or {},
      trump_suit = game.trump_suit,
      my_seat = current_seat,
      players = game.players
    }

    local card = BeloteBot.choose_card(hand, game_state)
    if not card then
      return { error = "Bot has no valid card to play" }
    end

    local play_result = BeloteEngine.play_card(
      { _key = game_key, hands = game.hands, current_trick = game.current_trick or {},
        trump_suit = game.trump_suit, tricks_won = game.tricks_won,
        round_points = game.round_points, trump_chooser = game.trump_chooser,
        scores = game.scores, dealer = game.dealer },
      current_seat,
      card
    )

    if play_result.error then
      return play_result
    end

    action_result = {
      success = true,
      action = "play",
      seat = current_seat,
      bot_name = bot_name,
      card = card,
      trick_complete = play_result.trick_complete,
      winner_seat = play_result.winner_seat
    }
  end

  -- Check if next turn is also a bot
  local next_result = Sdb:Sdbql(
    "FOR g IN belote_games FILTER g._key == @key RETURN g",
    { key = game_key }
  )
  local next_game = next_result and next_result.result and next_result.result[1]
  local needs_bot_turn = false

  if next_game then
    local next_seat = nil
    if next_game.state == "bidding" then
      next_seat = next_game.bidding_seat
    elseif next_game.state == "playing" then
      next_seat = next_game.current_player
    elseif next_game.state == "scoring" then
      needs_bot_turn = true  -- Need to start new round
    end

    if next_seat ~= nil then
      for _, p in ipairs(next_game.players or {}) do
        if p.seat == next_seat and p.is_bot then
          needs_bot_turn = true
          break
        end
      end
    end
  end

  action_result.needs_bot_turn = needs_bot_turn
  return action_result
end

-- Process all bot turns immediately (for backwards compatibility)
function BeloteEngine.process_bot_turns(game_key, max_turns)
  max_turns = max_turns or 10
  for _ = 1, max_turns do
    local result = BeloteEngine.process_single_bot_turn(game_key)
    if result.error or not result.needs_bot_turn then
      return result
    end
  end
  return { success = true, max_turns_reached = true }
end

return BeloteEngine
