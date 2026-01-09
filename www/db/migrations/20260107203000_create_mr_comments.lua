local M = {}

function M.up(db, helpers)
  helpers.create_collection("mr_comments")
  helpers.add_index("mr_comments", { "mr_id" })
  helpers.add_index("mr_comments", { "author_id" })
end

function M.down(db, helpers)
  helpers.drop_collection("mr_comments")
end

return M
