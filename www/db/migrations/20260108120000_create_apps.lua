local M = {}

function M.up(db, helpers)
  helpers.create_collection("apps")
  helpers.add_index("apps", { "created_by" })
  helpers.add_index("apps", { "position" })
end

function M.down(db, helpers)
  helpers.drop_collection("apps")
end

return M
