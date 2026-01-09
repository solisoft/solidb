local MailboxController = {}

function MailboxController:index()
  local db = require("db")
  local user = session.user
  
  -- Get message counts
  local inbox_count = db.query("SELECT COUNT(*) as count FROM messages WHERE folder = 'inbox' AND recipient = ? AND read = false", user._key)[1].count
  local sent_count = db.query("SELECT COUNT(*) as count FROM messages WHERE folder = 'sent' AND sender = ?", user._key)[1].count
  local draft_count = db.query("SELECT COUNT(*) as count FROM messages WHERE folder = 'drafts' AND sender = ?", user._key)[1].count
  
  -- Get recent messages
  local recent_messages = db.query([[
    SELECT m.*, u.name as sender_name, u.avatar as sender_avatar
    FROM messages m
    LEFT JOIN users u ON m.sender = u._key
    WHERE m.recipient = ? OR m.sender = ?
    ORDER BY m.created_at DESC
    LIMIT 10
  ]], user._key, user._key)
  
  -- Get upcoming calendar events
  local upcoming_events = db.query([[
    SELECT * FROM calendar_events 
    WHERE start_time >= datetime('now') 
    AND (attendee LIKE ? OR organizer = ?)
    ORDER BY start_time ASC
    LIMIT 5
  ]], '%' .. user._key .. '%', user._key)
  
  -- Get IMAP accounts
  local imap_accounts = db.query("SELECT * FROM imap_accounts WHERE user_key = ?", user._key)
  
  return render("mailbox/index", {
    inbox_count = inbox_count,
    sent_count = sent_count,
    draft_count = draft_count,
    recent_messages = recent_messages,
    upcoming_events = upcoming_events,
    imap_accounts = imap_accounts
  })
end

function Mailbox:messages()
  local db = require("db")
  local user = session.user
  local folder = params.folder or "inbox"
  
  -- Get messages for folder
  local messages = db.query([[
    SELECT m.*, u.name as sender_name, u.avatar as sender_avatar
    FROM messages m
    LEFT JOIN users u ON m.sender = u._key
    WHERE (m.recipient = ? OR m.sender = ?) AND m.folder = ?
    ORDER BY m.created_at DESC
  ]], user._key, user._key, folder)
  
  -- Get folders
  local folders = db.query([[
    SELECT folder, COUNT(*) as count, SUM(CASE WHEN read = false THEN 1 ELSE 0 END) as unread
    FROM messages 
    WHERE (recipient = ? OR sender = ?)
    GROUP BY folder
  ]], user._key, user._key)
  
  return render("mailbox/messages/index", {
    messages = messages,
    folders = folders,
    current_folder = folder
  })
end

function Mailbox:calendar()
  local db = require("db")
  local user = session.user
  local year = tonumber(params.year) or os.date("%Y")
  local month = tonumber(params.month) or os.date("%m")
  
  -- Get calendar events for month
  local events = db.query([[
    SELECT * FROM calendar_events 
    WHERE strftime('%Y', start_time) = ? 
    AND strftime('%m', start_time) = ?
    AND (attendee LIKE ? OR organizer = ?)
    ORDER BY start_time ASC
  ]], year, string.format("%02d", month), '%' .. user._key .. '%', user._key)
  
  return render("mailbox/calendar/index", {
    events = events,
    year = year,
    month = month,
    month_name = os.date("%B", os.time({year = year, month = month, day = 1}))
  })
end

function Mailbox:contacts()
  local db = require("db")
  local user = session.user
  
  -- Get contacts
  local contacts = db.query([[
    SELECT c.*, u.name, u.avatar, u.email
    FROM contacts c
    LEFT JOIN users u ON c.contact_user_key = u._key
    WHERE c.user_key = ?
    ORDER BY c.name ASC
  ]], user._key)
  
  return render("mailbox/contacts/index", {
    contacts = contacts
  })
end

function Mailbox:settings()
  local db = require("db")
  local user = session.user
  
  -- Get IMAP accounts
  local imap_accounts = db.query("SELECT * FROM imap_accounts WHERE user_key = ?", user._key)
  
  -- Get user preferences
  local preferences = db.query("SELECT * FROM mailbox_preferences WHERE user_key = ?", user._key)[1] or {}
  
  return render("mailbox/settings/index", {
    imap_accounts = imap_accounts,
    preferences = preferences
  })
end

return MailboxController