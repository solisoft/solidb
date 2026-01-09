local M = {}

function M.up(db, helpers)
  helpers.create_collection("features")
  helpers.add_index("features", { "app_id" })
  helpers.add_index("features", { "position" })
end

function M.down(db, helpers)
  helpers.drop_collection("features")
end

return M
