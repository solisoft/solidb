local Model = require("model")

local Signal = Model.create("signals", {
  permitted_fields = { "from_user", "to_user", "type", "data", "channel_id", "timestamp" },
  validations = {
    from_user = { presence = true },
    to_user = { presence = true },
    type = { presence = true }
  }
})

-- Create a new signal
function Signal.send(from_user_key, to_user_key, signal_type, signal_data, channel_id)
  return Signal:create({
    from_user = from_user_key,
    to_user = to_user_key,
    type = signal_type,
    data = signal_data,
    channel_id = channel_id,
    timestamp = os.time()
  })
end

-- Get pending signals for a user
function Signal.for_user(user_key, limit)
  limit = limit or 50
  local result = Sdb:Sdbql([[
    FOR s IN signals
    FILTER s.to_user == @user_key
    SORT s.timestamp ASC
    LIMIT @limit
    RETURN s
  ]], { user_key = user_key, limit = limit })

  local signals = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(signals, Signal:new(doc))
    end
  end
  return signals
end

-- Delete processed signal
function Signal.delete_by_key(key)
  Sdb:Sdbql("FOR s IN signals FILTER s._key == @key REMOVE s IN signals", { key = key })
end

return Signal
