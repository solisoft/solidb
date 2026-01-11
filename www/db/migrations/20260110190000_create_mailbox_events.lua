local M = {}

function M.up(db, helpers)
  helpers.create_collection("mailbox_events")
  helpers.add_index("mailbox_events", { "organizer_key" }, { name = "idx_events_organizer" })
  helpers.add_index("mailbox_events", { "start_time" }, { name = "idx_events_start_time" })
  helpers.add_index("mailbox_events", { "end_time" }, { name = "idx_events_end_time" })
end

function M.down(db, helpers)
  helpers.drop_collection("mailbox_events")
end

return M
