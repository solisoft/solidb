local M = {}

function M.up(db, helpers)
  helpers.create_collection("counters")
  -- Initialize task counter if creating fresh
  -- Note: We rely on upsert logic in model, but ensuring collection exists is enough
end

function M.down(db, helpers)
  helpers.drop_collection("counters")
end

return M
