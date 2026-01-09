local M = {}

function M.up(db, helpers)
  helpers.create_collection("tasks")
  helpers.add_index("tasks", { "feature_id" })
  helpers.add_index("tasks", { "assignee_id" })
  helpers.add_index("tasks", { "status" })
  helpers.add_index("tasks", { "position" })
end

function M.down(db, helpers)
  helpers.drop_collection("tasks")
end

return M
