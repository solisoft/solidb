local M = {}

function M.up(db, helpers)
  helpers.create_collection("mailbox_messages")
  helpers.add_index("mailbox_messages", { "sender_key" })
  helpers.add_index("mailbox_messages", { "folder" })
  helpers.add_index("mailbox_messages", { "thread_id" })
  -- recipients is an array, indexing it might be useful for IN queries if supported
  helpers.add_index("mailbox_messages", { "recipients" }) 
end

function M.down(db, helpers)
  helpers.drop_collection("mailbox_messages")
end

return M
