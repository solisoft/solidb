local M = {}

function M.up(db, helpers)
  helpers.create_collection("signals")
  helpers.add_index("signals", { "to_user" })
  helpers.add_index("signals", { "channel_id" })
  helpers.add_index("signals", { "timestamp" })
end

function M.down(db, helpers)
  helpers.drop_collection("signals")
end

return M
