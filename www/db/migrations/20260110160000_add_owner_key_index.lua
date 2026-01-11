local M = {}

function M.up(db, helpers)
  helpers.add_index("mailbox_messages", { "owner_key" })
end

function M.down(db, helpers)
  -- removing index is not typically supported via helpers in this framework explicitly named like this usually, 
  -- but usually safe to ignore in down or just drop collection if it was create_collection
end

return M
