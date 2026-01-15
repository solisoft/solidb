local Model = require("model")

local MailboxEvent = Model.create("mailbox_events", {
  permitted_fields = {
    "organizer_key", "attendees", "title", "description", "location",
    "start_time", "end_time", "all_day", "recurring", "color"
  },
  validations = {
    title = { presence = true },
    start_time = { presence = true }
  }
})

-- Attendee status constants
MailboxEvent.STATUS_PENDING = "pending"
MailboxEvent.STATUS_ACCEPTED = "accepted"
MailboxEvent.STATUS_DECLINED = "declined"
MailboxEvent.STATUS_TENTATIVE = "tentative"

-- Default colors for events
MailboxEvent.COLORS = {
  "#3b82f6", -- blue
  "#10b981", -- green
  "#f59e0b", -- amber
  "#ef4444", -- red
  "#8b5cf6", -- purple
  "#ec4899", -- pink
  "#06b6d4", -- cyan
  "#84cc16"  -- lime
}

-- Get organizer info
function MailboxEvent:organizer_info()
  if self.data.organizer and next(self.data.organizer) then
    return self.data.organizer
  end

  local organizer_key = self.organizer_key or self.data.organizer_key
  if not organizer_key then return {} end

  local result = Sdb:Sdbql(
    "FOR u IN users FILTER u._key == @key LIMIT 1 RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname, email: u.email }",
    { key = organizer_key }
  )
  if result and result.result and result.result[1] then
    self.data.organizer = result.result[1]
    return result.result[1]
  end
  return {}
end

-- Bulk fetch organizer info for events
local function bulk_fetch_organizers(events)
  if #events == 0 then return end

  local user_keys = {}
  local unique_keys = {}

  for _, event in ipairs(events) do
    local key = event.organizer_key or event.data.organizer_key
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

  for _, event in ipairs(events) do
    local key = event.organizer_key or event.data.organizer_key
    event.data.organizer = user_map[key] or {}
  end
end

-- Get events for a user within a date range
function MailboxEvent.for_user(user_key, start_date, end_date)
  -- Simplified query - just match organizer_key for now
  local result = Sdb:Sdbql([[
    FOR e IN mailbox_events
    SORT e.start_time ASC
    RETURN e
  ]], {})

  P("DEBUG: MailboxEvent.for_user: key=" .. tostring(user_key))
  
  local events = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      local matches = false
      if doc.organizer_key == user_key then
        matches = true
      elseif doc.attendees then
        for _, attendee in ipairs(doc.attendees) do
          if attendee.user_key == user_key then
            matches = true
            break
          end
        end
      end
      
      if matches then
        table.insert(events, MailboxEvent:new(doc))
      end
    end
    bulk_fetch_organizers(events)
  end
  P("DEBUG: final events count=" .. #events)
  return events
end

-- Get events for a specific day
function MailboxEvent.for_day(user_key, year, month, day)
  local start_of_day = os.time({ year = year, month = month, day = day, hour = 0, min = 0, sec = 0 })
  local end_of_day = start_of_day + 86400 - 1

  return MailboxEvent.for_user(user_key, start_of_day, end_of_day)
end

-- Get events for a month
function MailboxEvent.for_month(user_key, year, month)
  local start_of_month = os.time({ year = year, month = month, day = 1, hour = 0, min = 0, sec = 0 })
  -- Get last day of month
  local next_month = month == 12 and 1 or month + 1
  local next_year = month == 12 and year + 1 or year
  local end_of_month = os.time({ year = next_year, month = next_month, day = 1, hour = 0, min = 0, sec = 0 }) - 1

  return MailboxEvent.for_user(user_key, start_of_month, end_of_month)
end

-- Get events for a year (returns a lookup table by date key "YYYY-MM-DD")
function MailboxEvent.for_year(user_key, year)
  local start_of_year = os.time({ year = year, month = 1, day = 1, hour = 0, min = 0, sec = 0 })
  local end_of_year = os.time({ year = year + 1, month = 1, day = 1, hour = 0, min = 0, sec = 0 }) - 1

  local events = MailboxEvent.for_user(user_key, start_of_year, end_of_year)

  -- Build lookup table by date
  local events_by_date = {}
  for _, event in ipairs(events) do
    local start_time = event.start_time or event.data.start_time
    if start_time then
      local e_year = tonumber(os.date("%Y", start_time))
      if e_year == year then
        local date_key = os.date("%Y-%m-%d", start_time)
        if not events_by_date[date_key] then
          events_by_date[date_key] = {}
        end
        table.insert(events_by_date[date_key], event)
      end
    end
  end

  return events_by_date
end


-- Get events for a user with pending status
function MailboxEvent.pending_for_user(user_key)
  local result = Sdb:Sdbql([[
    FOR e IN mailbox_events
    SORT e.start_time ASC
    RETURN e
  ]], {})

  local events = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      local is_pending = false
      if doc.attendees then
        for _, a in ipairs(doc.attendees) do
          if a.user_key == user_key and a.status == "pending" then
            is_pending = true
            break
          end
        end
      end
      
      if is_pending then
        table.insert(events, MailboxEvent:new(doc))
      end
    end
    bulk_fetch_organizers(events)
  end
  return events
end

-- Get upcoming events for a user
function MailboxEvent.upcoming(user_key, limit)
  limit = limit or 10
  local now = os.time()

  local result = Sdb:Sdbql([[
    FOR e IN mailbox_events
    FILTER e.start_time >= @now
    SORT e.start_time ASC
    RETURN e
  ]], { now = now })

  local events = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      local matches = false
      if doc.organizer_key == user_key then
        matches = true
      elseif doc.attendees then
        for _, attendee in ipairs(doc.attendees) do
          if attendee.user_key == user_key then
            matches = true
            break
          end
        end
      end
      
      if matches then
        table.insert(events, MailboxEvent:new(doc))
        if #events >= limit then break end
      end
    end
    bulk_fetch_organizers(events)
  end
  return events
end

-- Create a new event
function MailboxEvent.create_event(organizer, title, description, location, start_time, end_time, all_day, attendee_keys, color)
  -- Build attendees list with pending status
  local attendees = {}
  for _, key in ipairs(attendee_keys or {}) do
    table.insert(attendees, {
      user_key = key,
      status = MailboxEvent.STATUS_PENDING
    })
  end

  local event = MailboxEvent:create({
    organizer_key = organizer._key,
    attendees = attendees,
    title = title,
    description = description or "",
    location = location or "",
    start_time = start_time,
    end_time = end_time or (start_time + 3600), -- Default 1 hour duration
    all_day = all_day or false,
    recurring = nil,
    color = color or MailboxEvent.COLORS[1]
  })

  event.data.organizer = {
    _key = organizer._key,
    firstname = organizer.firstname,
    lastname = organizer.lastname,
    email = organizer.email
  }

  return event
end

-- Respond to event invitation
function MailboxEvent:respond(user_key, status)
  local attendees = self.attendees or self.data.attendees or {}
  local updated = false

  for i, attendee in ipairs(attendees) do
    if attendee.user_key == user_key then
      attendees[i].status = status
      updated = true
      break
    end
  end

  if updated then
    self:update({ attendees = attendees })
    self.data.attendees = attendees
  end

  return updated
end

-- Get attendee status for a user
function MailboxEvent:attendee_status(user_key)
  local attendees = self.attendees or self.data.attendees or {}

  for _, attendee in ipairs(attendees) do
    if attendee.user_key == user_key then
      return attendee.status
    end
  end

  -- Check if user is organizer
  local organizer_key = self.organizer_key or self.data.organizer_key
  if organizer_key == user_key then
    return "organizer"
  end

  return nil
end

-- Get all attendees with user info
function MailboxEvent:attendees_with_info()
  local attendees = self.attendees or self.data.attendees or {}
  if #attendees == 0 then return {} end

  local user_keys = {}
  for _, a in ipairs(attendees) do
    table.insert(user_keys, a.user_key)
  end

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

  local attendees_info = {}
  for _, a in ipairs(attendees) do
    local user = user_map[a.user_key] or {}
    table.insert(attendees_info, {
      user_key = a.user_key,
      status = a.status,
      firstname = user.firstname,
      lastname = user.lastname,
      email = user.email
    })
  end

  return attendees_info
end

-- Format time for display
function MailboxEvent:formatted_time()
  local start_time = self.start_time or self.data.start_time
  local end_time = self.end_time or self.data.end_time
  local all_day = self.all_day or self.data.all_day

  if not start_time then return "" end

  if all_day then
    return "All day"
  end

  local start_str = os.date("%H:%M", start_time)
  local end_str = end_time and os.date("%H:%M", end_time) or ""

  if end_str ~= "" then
    return start_str .. " - " .. end_str
  end
  return start_str
end

-- Format date for display
function MailboxEvent:formatted_date()
  local start_time = self.start_time or self.data.start_time
  if not start_time then return "" end
  return os.date("%b %d, %Y", start_time)
end

-- Check if event is today
function MailboxEvent:is_today()
  local start_time = self.start_time or self.data.start_time
  if not start_time then return false end
  local today = os.date("%Y-%m-%d")
  local event_date = os.date("%Y-%m-%d", start_time)
  return today == event_date
end

-- Duration in minutes
function MailboxEvent:duration_minutes()
  local start_time = self.start_time or self.data.start_time
  local end_time = self.end_time or self.data.end_time
  if not start_time or not end_time then return 0 end
  return math.floor((end_time - start_time) / 60)
end

return MailboxEvent
