local M = {}

function M.up(db, helpers)
  helpers.create_collection("belote_games")
  helpers.add_index("belote_games", { "state" })
  helpers.add_index("belote_games", { "host_key" })
  helpers.add_index("belote_games", { "created_at" })
end

function M.down(db, helpers)
  helpers.drop_collection("belote_games")
end

return M
