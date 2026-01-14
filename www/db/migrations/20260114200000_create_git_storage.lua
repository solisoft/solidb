-- Migration: create_git_storage
-- Creates blob collection for git repository backup/sync

local M = {}

function M.up(db, helpers)
  -- Create blob collection for git storage
  -- Blob collections support automatic chunking for large files
  helpers.create_collection("_git_storage", { type = "blob" })
end

function M.down(db, helpers)
  helpers.drop_collection("_git_storage")
end

return M
