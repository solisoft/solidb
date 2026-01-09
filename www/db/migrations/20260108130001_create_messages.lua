local M = {}

function M.up(db, helpers)
  helpers.create_collection("messages")
  helpers.add_index("messages", { "channel_id" })
  helpers.add_index("messages", { "parent_id" })
  helpers.add_index("messages", { "user_key" })
  helpers.add_index("messages", { "timestamp" })
end

function M.down(db, helpers)
  helpers.drop_collection("messages")
end

return M
