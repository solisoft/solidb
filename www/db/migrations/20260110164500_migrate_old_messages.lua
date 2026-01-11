local M = {}

function M.up(db, helpers)
  -- 1. Fix SENT and DRAFTS messages (Set owner_key = sender_key)
  db:query([[
    FOR m IN mailbox_messages
    FILTER (m.folder == 'sent' OR m.folder == 'drafts') AND m.owner_key == null
    UPDATE m WITH { owner_key: m.sender_key } IN mailbox_messages
  ]])

  -- 2. Fix INBOX messages (Fan-out)
  -- Get all old inbox messages
  local result = db:query([[
    FOR m IN mailbox_messages
    FILTER m.folder == 'inbox' AND m.owner_key == null
    RETURN m
  ]])

  if result and result.result then
    for _, msg in ipairs(result.result) do
      local recipients = msg.recipients or {}
      -- Prevent duplicates if array has same user multiple times
      local unique_recipients = {}
      for _, r in ipairs(recipients) do
        unique_recipients[r] = true
      end

      -- Create a copy for each recipient
      for r_key, _ in pairs(unique_recipients) do
        -- Clone message data
        local new_msg = {}
        for k, v in pairs(msg) do
          if k ~= "_key" and k ~= "_id" and k ~= "_rev" then
            new_msg[k] = v
          end
        end
        new_msg.owner_key = r_key
        -- Insert new individual message
        db:query("INSERT @doc INTO mailbox_messages", { doc = new_msg })
      end

      -- Remove the old shared message
      db:query("REMOVE @key IN mailbox_messages", { key = msg._key })
    end
  end
end

function M.down(db, helpers)
  -- Reversion is complex/lossy (merging individual read statuses back to shared), skipping for now.
end

return M
