local M = {}

function M.up(db, helpers)
  helpers.create_collection("repositories")
  helpers.add_index("repositories", { "owner_id" })
  helpers.add_index("repositories", { "name" }, { unique = true })
end

function M.down(db, helpers)
  helpers.drop_collection("repositories")
end

return M
