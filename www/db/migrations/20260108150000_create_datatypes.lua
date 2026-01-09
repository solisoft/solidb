local M = {}

function M.up(db, helpers)
  helpers.create_collection("datatypes")
  helpers.add_index("datatypes", { "slug" }, { unique = true })
end

function M.down(db, helpers)
  helpers.drop_collection("datatypes")
end

return M
