local M = {}

function M.up(db, helpers)
  helpers.create_collection("pages")
  helpers.add_index("pages", { "parent_id" })
  helpers.add_index("pages", { "position" })
  helpers.add_index("pages", { "created_by" })
end

function M.down(db, helpers)
  helpers.drop_collection("pages")
end

return M
