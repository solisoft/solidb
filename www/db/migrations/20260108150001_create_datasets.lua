local M = {}

function M.up(db, helpers)
  helpers.create_collection("datasets")
  helpers.add_index("datasets", { "_type" })
  helpers.add_index("datasets", { "created_at" })
end

function M.down(db, helpers)
  helpers.drop_collection("datasets")
end

return M
