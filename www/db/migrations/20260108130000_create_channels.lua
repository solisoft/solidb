local M = {}

function M.up(db, helpers)
  helpers.create_collection("channels")
  helpers.add_index("channels", { "type" })
  helpers.add_index("channels", { "name" }, { unique = true })
  helpers.add_index("channels", { "created_by" })

  -- Seed default channel
  helpers.seed("channels", {
    { _key = "general", name = "general", type = "system", created_at = os.time() }
  })
end

function M.down(db, helpers)
  helpers.drop_collection("channels")
end

return M
