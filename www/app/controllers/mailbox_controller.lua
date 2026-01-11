local Controller = require("controller")
local MailboxController = Controller:extend()
local AuthHelper = require("helpers.auth_helper")
local TextHelper = require("helpers.text_helper")
local MailboxMessage = require("models.mailbox_message")
local MailboxEvent = require("models.mailbox_event")

-- Get current user (middleware ensures user is authenticated)
local function get_current_user()
  return AuthHelper.get_current_user()
end

-- Main dashboard
function MailboxController:index()
  local current_user = get_current_user()

  -- Get stats
  local folder_counts = MailboxMessage.folder_counts(current_user._key)
  local upcoming_events = MailboxEvent.upcoming(current_user._key, 5)

  self.layout = "application"
  self.full_height = true
  self:render("mailbox/index", {
    current_user = current_user,
    folder_counts = folder_counts,
    upcoming_events = upcoming_events,
    current_folder = "inbox",
    db_name = self.params.db or (Sdb and Sdb.database and Sdb.database()) -- Attempt to get db name
  })
end

-- Refresh sidebar stats
function MailboxController:update_sidebar()
  local current_user = get_current_user()
  local folder_counts = MailboxMessage.folder_counts(current_user._key)
  local current_folder = self.params.folder or "inbox"
  
  self.layout = false
  self:render("mailbox/_sidebar", {
    current_user = current_user,
    folder_counts = folder_counts,
    current_folder = current_folder
  })
end

-- Inbox (default folder)
function MailboxController:inbox()
  return self:folder_view("inbox")
end

-- Messages by folder
function MailboxController:folder()
  local folder = self.params.folder or "inbox"
  return self:folder_view(folder)
end

-- Internal folder view helper
function MailboxController:folder_view(folder)
  local current_user = get_current_user()
  local page = tonumber(self.params.page) or 1
  local limit = 25
  local offset = (page - 1) * limit
  local query = self.params.q

  local messages = MailboxMessage.for_folder(folder, current_user._key, limit, offset, query)
  local folder_counts = MailboxMessage.folder_counts(current_user._key)

  local view_data = {
    current_user = current_user,
    messages = messages,
    folder_counts = folder_counts,
    current_folder = folder,
    page = page,
    TextHelper = TextHelper
  }

  if self:is_htmx_request() then
    self.layout = false
    return self:render("mailbox/messages/_list", view_data)
  end

  self.layout = "application"
  self.full_height = true
  self:render("mailbox/messages/index", view_data)
end

-- Messages table (HTMX partial)
function MailboxController:messages_table()
  local current_user = get_current_user()
  local folder = self.params.folder or "inbox"
  local page = tonumber(self.params.page) or 1
  local limit = 25
  local offset = (page - 1) * limit
  local query = self.params.q

  local messages = MailboxMessage.for_folder(folder, current_user._key, limit, offset, query)

  self.layout = false
  self:render("mailbox/messages/_list", {
    messages = messages,
    current_folder = folder,
    page = page,
    TextHelper = TextHelper
  })
end

-- View single message
function MailboxController:view_message()
  local current_user = get_current_user()
  local message_id = self.params.id

  local message = MailboxMessage:find(message_id)
  if not message then
    return self:redirect("/mailbox/messages/inbox")
  end

  -- Mark as read if recipient
  local recipients = message.recipients or message.data.recipients or {}
  local is_recipient = false
  for _, r in ipairs(recipients) do
    if r == current_user._key then
      is_recipient = true
      break
    end
  end

  if is_recipient and not (message.read or message.data.read) then
    message:mark_read()
  end

  local folder_counts = MailboxMessage.folder_counts(current_user._key)

  self.layout = "application"
  self:render("mailbox/messages/view", {
    current_user = current_user,
    message = message,
    folder_counts = folder_counts,
    current_folder = message.folder or message.data.folder,
    TextHelper = TextHelper
  })
end

-- Get all users for client-side search
local function get_all_users()
  local result = Sdb:Sdbql([[
    FOR u IN users
    RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname, email: u.email }
  ]])
  return (result and result.result) or {}
end

-- Compose new message
function MailboxController:compose()
  local current_user = get_current_user()
  local folder_counts = MailboxMessage.folder_counts(current_user._key)
  local all_users = get_all_users()

  self.layout = "application"
  self:render("mailbox/messages/compose", {
    current_user = current_user,
    folder_counts = folder_counts,
    current_folder = "compose",
    reply_to = nil,
    all_users = all_users
  })
end

-- Compose reply
function MailboxController:compose_reply()
  local current_user = get_current_user()
  local message_id = self.params.id

  local original = MailboxMessage:find(message_id)
  if not original then
    return self:redirect("/mailbox/messages/inbox")
  end

  local folder_counts = MailboxMessage.folder_counts(current_user._key)
  local all_users = get_all_users()

  self.layout = "application"
  self:render("mailbox/messages/compose", {
    current_user = current_user,
    folder_counts = folder_counts,
    current_folder = "compose",
    reply_to = original,
    all_users = all_users
  })
end

function MailboxController:compose_forward()
  local current_user = get_current_user()
  local message_id = self.params.id

  local original = MailboxMessage:find(message_id)
  if not original then
    return self:redirect("/mailbox/messages/inbox")
  end

  local folder_counts = MailboxMessage.folder_counts(current_user._key)
  local all_users = get_all_users()

  -- Prepare forwarded content
  local subject = original.subject or original.data.subject or ""
  if not subject:match("^Fwd:") then
    subject = "Fwd: " .. subject
  end
  
  local sender = original:sender_info()
  local sender_name = (sender.firstname and sender.lastname and (sender.firstname .. " " .. sender.lastname)) or sender.email or "Unknown"
  local date = original:formatted_date()
  
  local body = "\n\n---------- Forwarded message ---------\n" ..
               "From: " .. sender_name .. " <" .. (sender.email or "") .. ">\n" ..
               "Date: " .. date .. "\n" ..
               "Subject: " .. (original.subject or original.data.subject or "") .. "\n" ..
               "To: " .. (original.recipients and table.concat(original.recipients, ", ") or "") .. "\n\n" ..
               (original.body or original.data.body or "")

  self.layout = "application"
  self:render("mailbox/messages/compose", {
    current_user = current_user,
    folder_counts = folder_counts,
    current_folder = "compose",
    forward_data = { -- Pass explicit forward data instead of using reply logic completely
      subject = subject,
      body = body
    },
    all_users = all_users
  })
end

-- Send message
function MailboxController:send()
  local current_user = get_current_user()

  local recipients_str = self.params.recipients or ""
  local cc_str = self.params.cc or ""
  local bcc_str = self.params.bcc or ""
  local subject = self.params.subject or ""
  local body = self.params.body or ""

  -- Parse recipient keys (comma-separated)
  local recipients = {}
  for key in string.gmatch(recipients_str, "[^,]+") do
    local trimmed = key:match("^%s*(.-)%s*$")
    if trimmed and trimmed ~= "" then
      table.insert(recipients, trimmed)
    end
  end

  local cc = {}
  for key in string.gmatch(cc_str, "[^,]+") do
    local trimmed = key:match("^%s*(.-)%s*$")
    if trimmed and trimmed ~= "" then
      table.insert(cc, trimmed)
    end
  end

  local bcc = {}
  for key in string.gmatch(bcc_str, "[^,]+") do
    local trimmed = key:match("^%s*(.-)%s*$")
    if trimmed and trimmed ~= "" then
      table.insert(bcc, trimmed)
    end
  end

  if #recipients == 0 then
    return self:json({ error = "At least one recipient is required" }, 400)
  end

  if subject == "" then
    return self:json({ error = "Subject is required" }, 400)
  end

  -- MailboxMessage.send(user, to, cc, subject, body, options, bcc)
  local message = MailboxMessage.send(current_user, recipients, cc, subject, body, {}, bcc)

  if self:is_htmx_request() then
    self:set_header("HX-Redirect", "/mailbox/messages/sent")
    return self:html("")
  end

  return self:redirect("/mailbox/messages/sent")
end

-- Save draft
function MailboxController:save_draft()
  local current_user = get_current_user()

  local recipients_str = self.params.recipients or ""
  local cc_str = self.params.cc or ""
  local bcc_str = self.params.bcc or ""
  local subject = self.params.subject or ""
  local body = self.params.body or ""
  local draft_key = self.params.draft_key

  -- Parse recipient keys
  local recipients = {}
  for key in string.gmatch(recipients_str, "[^,]+") do
    local trimmed = key:match("^%s*(.-)%s*$")
    if trimmed and trimmed ~= "" then
      table.insert(recipients, trimmed)
    end
  end

  local cc = {}
  for key in string.gmatch(cc_str, "[^,]+") do
    local trimmed = key:match("^%s*(.-)%s*$")
    if trimmed and trimmed ~= "" then
      table.insert(cc, trimmed)
    end
  end

  local bcc = {}
  for key in string.gmatch(bcc_str, "[^,]+") do
    local trimmed = key:match("^%s*(.-)%s*$")
    if trimmed and trimmed ~= "" then
      table.insert(bcc, trimmed)
    end
  end

  local draft = MailboxMessage.save_draft(current_user, recipients, cc, subject, body, draft_key, bcc)

  return self:json({ success = true, draft_key = draft._key or draft.data._key })
end

-- Search recipients (HTMX autocomplete)
function MailboxController:search_recipients()
  -- Get the search query from the input value
  local query = self.params["recipients-input"] or self.params["cc-input"] or self.params["bcc-input"] or self.params.q or ""

  -- Determine dropdown type based on htmx target
  local hx_target = GetHeader("HX-Target") or ""
  local dropdown_type = "recipients"
  if hx_target:match("cc") then
    dropdown_type = "cc"
  end
  if hx_target:match("bcc") then
    dropdown_type = "bcc"
  end

  if #query < 1 then
    self.layout = false
    return self:html("")
  end

  -- Search all users (including current user)
  local result = Sdb:Sdbql([[
    FOR u IN users
    FILTER CONTAINS(LOWER(u.firstname || ''), LOWER(@query))
        OR CONTAINS(LOWER(u.lastname || ''), LOWER(@query))
        OR CONTAINS(LOWER(u.email), LOWER(@query))
    LIMIT 10
    RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname, email: u.email }
  ]], { query = query })

  local users = (result and result.result) or {}

  self.layout = false
  self:render("mailbox/messages/_recipients_dropdown", { users = users, dropdown_type = dropdown_type })
end

-- Toggle star
function MailboxController:toggle_star()
  local current_user = get_current_user()
  local message_id = self.params.id

  local message = MailboxMessage:find(message_id)
  if not message then
    return self:json({ error = "Message not found" }, 404)
  end

  local new_starred = message:toggle_star()

  if self:is_htmx_request() then
    self.layout = false
    return self:render("mailbox/messages/_star_button", {
      message = message,
      starred = new_starred
    })
  end

  return self:json({ success = true, starred = new_starred })
end

-- Mark as read
function MailboxController:mark_read()
  local message_id = self.params.id

  local message = MailboxMessage:find(message_id)
  if not message then
    return self:json({ error = "Message not found" }, 404)
  end

  message:mark_read()

  return self:json({ success = true })
end

-- Archive message
function MailboxController:archive()
  local message_id = self.params.id

  local message = MailboxMessage:find(message_id)
  if not message then
    return self:json({ error = "Message not found" }, 404)
  end

  message:archive()

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "messageArchived")
    return self:html("")
  end

  return self:json({ success = true })
end

-- Delete message
function MailboxController:delete_message()
  local message_id = self.params.id

  local message = MailboxMessage:find(message_id)
  if not message then
    return self:json({ error = "Message not found" }, 404)
  end

  message:destroy()

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "messageDeleted")
    return self:html("")
  end

  return self:json({ success = true })
end

return MailboxController
