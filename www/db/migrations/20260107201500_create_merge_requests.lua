local M = {}

function M.up(db, helpers)
  helpers.create_collection("merge_requests")
  helpers.add_index("merge_requests", { "repo_id" })
  helpers.add_index("merge_requests", { "repo_id", "status" })
end

function M.down(db, helpers)
  helpers.drop_collection("merge_requests")
end

return M
