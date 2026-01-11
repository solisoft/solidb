local M = {}

function M.up(db, helpers)
  helpers.add_index("mailbox_messages", { "subject" }, { type = "fulltext", name = "idx_mailbox_subject_ft" })
  helpers.add_index("mailbox_messages", { "body" }, { type = "fulltext", name = "idx_mailbox_body_ft" })
end

function M.down(db, helpers)
  helpers.drop_index("mailbox_messages", "idx_mailbox_subject_ft")
  helpers.drop_index("mailbox_messages", "idx_mailbox_body_ft")
end

return M
