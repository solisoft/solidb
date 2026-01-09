local M = {}

function M.up(db, helpers)
  helpers.create_collection("users")
end

function M.down(db, helpers)
  helpers.drop_collection("users")
end

return M
