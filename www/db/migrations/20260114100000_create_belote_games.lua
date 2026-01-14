local M = {}

function M.up(db, helpers)
  helpers.create_collection("belote_games")
  helpers.add_index("belote_games", { "status" })
  helpers.add_index("belote_games", { "created_by" })
end

function M.down(db, helpers)
  helpers.drop_collection("belote_games")
end

return M
