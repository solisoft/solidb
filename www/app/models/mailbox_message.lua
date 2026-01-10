local Model = require("model")

local MailboxMessage = Model.create("mailbox_messages", {
  permitted_fields = {
    "sender_key", "recipients", "cc", "subject", "body",
    "folder", "read", "starred", "thread_id", "attachments"
  },
  validations = {
    subject = { presence = true },
    body = { presence = true }
  }
})

-- Get sender info
function MailboxMessage:sender_info()
  if self.data.sender and next(self.data.sender) then
    return self.data.sender
  end

  local sender_key = self.sender_key or self.data.sender_key
  if not sender_key then return {} end

  local result = Sdb:Sdbql(
    "FOR u IN users FILTER u._key == @key LIMIT 1 RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname, email: u.email }",
    { key = sender_key }
  )
  if result and result.result and result.result[1] then
    self.data.sender = result.result[1]
    return result.result[1]
  end
  return {}
end

-- Bulk fetch sender info for a list of messages
local function bulk_fetch_senders(messages)
  if #messages == 0 then return end

  local user_keys = {}
  local unique_keys = {}

  for _, msg in ipairs(messages) do
    local key = msg.sender_key or msg.data.sender_key
    if key and not unique_keys[key] then
      unique_keys[key] = true
      table.insert(user_keys, key)
    end
  end

  if #user_keys == 0 then return end

  local result = Sdb:Sdbql([[
    FOR u IN users
    FILTER u._key IN @keys
    RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname, email: u.email }
  ]], { keys = user_keys })

  local user_map = {}
  if result and result.result then
    for _, u in ipairs(result.result) do
      user_map[u._key] = u
    end
  end

  for _, msg in ipairs(messages) do
    local key = msg.sender_key or msg.data.sender_key
    msg.data.sender = user_map[key] or {}
  end
end

-- Get messages for a user in a specific folder
function MailboxMessage.for_folder(folder, user_key, limit, offset)
  limit = limit or 50
  offset = offset or 0

  local query
  local params = { user_key = user_key, limit = limit, offset = offset }

  if folder == "sent" then
    -- Sent: messages where user is sender
    query = [[
      FOR m IN mailbox_messages
      FILTER m.sender_key == @user_key AND m.folder == "sent"
      SORT m._created_at DESC
      LIMIT @offset, @limit
      RETURN m
    ]]
  elseif folder == "starred" then
    -- Starred: messages where user is recipient AND starred
    query = [[
      FOR m IN mailbox_messages
      FILTER @user_key IN m.recipients AND m.starred == true
      SORT m._created_at DESC
      LIMIT @offset, @limit
      RETURN m
    ]]
  elseif folder == "drafts" then
    -- Drafts: messages where user is sender AND folder is drafts
    query = [[
      FOR m IN mailbox_messages
      FILTER m.sender_key == @user_key AND m.folder == "drafts"
      SORT m._updated_at DESC
      LIMIT @offset, @limit
      RETURN m
    ]]
  else
    -- Inbox/Archive: messages where user is recipient
    params.folder = folder or "inbox"
    query = [[
      FOR m IN mailbox_messages
      FILTER @user_key IN m.recipients AND m.folder == @folder
      SORT m._created_at DESC
      LIMIT @offset, @limit
      RETURN m
    ]]
  end

  local result = Sdb:Sdbql(query, params)

  local messages = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(messages, MailboxMessage:new(doc))
    end
    bulk_fetch_senders(messages)
  end
  return messages
end

-- Count unread messages for a user
function MailboxMessage.unread_count(user_key)
  local result = Sdb:Sdbql([[
    FOR m IN mailbox_messages
    FILTER @user_key IN m.recipients AND m.folder == "inbox" AND m.read == false
    COLLECT WITH COUNT INTO count
    RETURN count
  ]], { user_key = user_key })

  if result and result.result and result.result[1] then
    return result.result[1]
  end
  return 0
end

-- Count messages by folder for a user
function MailboxMessage.folder_counts(user_key)
  local counts = {
    inbox = 0,
    sent = 0,
    drafts = 0,
    archive = 0,
    starred = 0,
    unread = 0
  }

  -- Inbox count
  local result = Sdb:Sdbql([[
    FOR m IN mailbox_messages
    FILTER @user_key IN m.recipients AND m.folder == "inbox"
    COLLECT WITH COUNT INTO count
    RETURN count
  ]], { user_key = user_key })
  if result and result.result and result.result[1] then
    counts.inbox = result.result[1]
  end

  -- Unread count
  result = Sdb:Sdbql([[
    FOR m IN mailbox_messages
    FILTER @user_key IN m.recipients AND m.folder == "inbox" AND m.read == false
    COLLECT WITH COUNT INTO count
    RETURN count
  ]], { user_key = user_key })
  if result and result.result and result.result[1] then
    counts.unread = result.result[1]
  end

  -- Sent count
  result = Sdb:Sdbql([[
    FOR m IN mailbox_messages
    FILTER m.sender_key == @user_key AND m.folder == "sent"
    COLLECT WITH COUNT INTO count
    RETURN count
  ]], { user_key = user_key })
  if result and result.result and result.result[1] then
    counts.sent = result.result[1]
  end

  -- Drafts count
  result = Sdb:Sdbql([[
    FOR m IN mailbox_messages
    FILTER m.sender_key == @user_key AND m.folder == "drafts"
    COLLECT WITH COUNT INTO count
    RETURN count
  ]], { user_key = user_key })
  if result and result.result and result.result[1] then
    counts.drafts = result.result[1]
  end

  -- Archive count
  result = Sdb:Sdbql([[
    FOR m IN mailbox_messages
    FILTER @user_key IN m.recipients AND m.folder == "archive"
    COLLECT WITH COUNT INTO count
    RETURN count
  ]], { user_key = user_key })
  if result and result.result and result.result[1] then
    counts.archive = result.result[1]
  end

  -- Starred count
  result = Sdb:Sdbql([[
    FOR m IN mailbox_messages
    FILTER @user_key IN m.recipients AND m.starred == true
    COLLECT WITH COUNT INTO count
    RETURN count
  ]], { user_key = user_key })
  if result and result.result and result.result[1] then
    counts.starred = result.result[1]
  end

  return counts
end

-- Send a new message
function MailboxMessage.send(sender, recipients, cc, subject, body, attachments)
  -- Create message in recipients' inbox
  local msg = MailboxMessage:create({
    sender_key = sender._key,
    recipients = recipients,
    cc = cc or {},
    subject = subject,
    body = body,
    folder = "inbox",
    read = false,
    starred = false,
    thread_id = nil,
    attachments = attachments or {}
  })

  -- Also save to sender's sent folder
  MailboxMessage:create({
    sender_key = sender._key,
    recipients = recipients,
    cc = cc or {},
    subject = subject,
    body = body,
    folder = "sent",
    read = true,
    starred = false,
    thread_id = nil,
    attachments = attachments or {}
  })

  msg.data.sender = { _key = sender._key, firstname = sender.firstname, lastname = sender.lastname, email = sender.email }
  return msg
end

-- Save as draft
function MailboxMessage.save_draft(sender, recipients, cc, subject, body, existing_key)
  if existing_key then
    -- Update existing draft
    local draft = MailboxMessage:find(existing_key)
    if draft and draft.data.sender_key == sender._key then
      draft:update({
        recipients = recipients or {},
        cc = cc or {},
        subject = subject or "",
        body = body or ""
      })
      return draft
    end
  end

  -- Create new draft
  return MailboxMessage:create({
    sender_key = sender._key,
    recipients = recipients or {},
    cc = cc or {},
    subject = subject or "",
    body = body or "",
    folder = "drafts",
    read = true,
    starred = false,
    thread_id = nil,
    attachments = {}
  })
end

-- Mark as read
function MailboxMessage:mark_read()
  self:update({ read = true })
  self.data.read = true
end

-- Toggle star
function MailboxMessage:toggle_star()
  local new_starred = not (self.starred or self.data.starred)
  self:update({ starred = new_starred })
  self.data.starred = new_starred
  return new_starred
end

-- Archive message
function MailboxMessage:archive()
  self:update({ folder = "archive" })
  self.data.folder = "archive"
end

-- Move to folder
function MailboxMessage:move_to(folder)
  self:update({ folder = folder })
  self.data.folder = folder
end

-- Get thread (replies to this message)
function MailboxMessage:get_thread()
  local message_id = "mailbox_messages/" .. (self._key or self.data._key)

  local result = Sdb:Sdbql([[
    FOR m IN mailbox_messages
    FILTER m.thread_id == @thread_id
    SORT m._created_at ASC
    RETURN m
  ]], { thread_id = message_id })

  local replies = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(replies, MailboxMessage:new(doc))
    end
    bulk_fetch_senders(replies)
  end
  return replies
end

-- Reply to this message
function MailboxMessage:reply(sender, body)
  local now = os.time()
  local original_sender = self.sender_key or self.data.sender_key
  local subject = self.subject or self.data.subject
  if not subject:match("^Re:") then
    subject = "Re: " .. subject
  end

  return MailboxMessage.send(
    sender,
    { original_sender },
    {},
    subject,
    body,
    {}
  )
end

-- Format date for display
function MailboxMessage:formatted_date()
  local timestamp = self._created_at or self.data._created_at
  if not timestamp then return "" end
  return os.date("%b %d, %Y %H:%M", timestamp)
end

-- Check if message is from today
function MailboxMessage:is_today()
  local timestamp = self._created_at or self.data._created_at
  if not timestamp then return false end
  local today = os.date("%Y-%m-%d")
  local msg_date = os.date("%Y-%m-%d", timestamp)
  return today == msg_date
end

-- Short date for list view
function MailboxMessage:short_date()
  local timestamp = self._created_at or self.data._created_at
  if not timestamp then return "" end

  if self:is_today() then
    return os.date("%H:%M", timestamp)
  else
    return os.date("%b %d", timestamp)
  end
end

return MailboxMessage
