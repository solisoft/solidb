local M = {}

function M.up(db, helpers)
  helpers.add_index("users", { "email" }, { unique = true, name = "idx_email" })
end

function M.down(db, helpers)
  helpers.drop_index("users", "idx_email")
end

return M
