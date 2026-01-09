-- Migration: Create uploads collection for storing file metadata

local M = {}

function M.up(db, helpers)
  helpers.create_collection("_uploads")
  helpers.add_index("_uploads", { "created_at" })
  helpers.add_index("_uploads", { "collection" })
end

function M.down(db, helpers)
  helpers.drop_collection("_uploads")
end

return M
