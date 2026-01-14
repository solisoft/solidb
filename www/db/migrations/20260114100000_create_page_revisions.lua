local M = {}

function M.up(db, helpers)
  helpers.create_collection("page_revisions")
  helpers.add_index("page_revisions", { "page_id" })
  helpers.add_index("page_revisions", { "changed_by" })
  -- TTL index: auto-delete revisions older than 6 months (180 days = 15552000 seconds)
  -- Uses the TTL endpoint: POST /_api/database/{db}/ttl/{collection}
  local db_name = db._db_config and db._db_config.db_name or "_system"
  local url = db._db_config.url .. "/_api/database/" .. db_name .. "/ttl/page_revisions"
  db:RefreshToken()
  Fetch(url, {
    method = "POST",
    headers = {
      ["Content-Type"] = "application/json",
      ["Authorization"] = "Bearer " .. (db._token or "")
    },
    body = EncodeJson({
      name = "ttl_changed_at",
      field = "changed_at",
      expire_after_seconds = 15552000
    })
  })
end

function M.down(db, helpers)
  helpers.drop_collection("page_revisions")
end

return M
