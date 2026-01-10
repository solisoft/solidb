local Controller = require("controller")
local CalendarController = Controller:extend()
local AuthHelper = require("helpers.auth_helper")
local MailboxEvent = require("models.mailbox_event")
local MailboxMessage = require("models.mailbox_message")

-- Get current user
local function get_current_user()
  return AuthHelper.get_current_user()
end

-- Get all users for attendee selection
local function get_all_users()
  local result = Sdb:Sdbql([[
    FOR u IN users
    RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname, email: u.email }
  ]])
  return (result and result.result) or {}
end

-- Calendar index (month view)
function CalendarController:index()
  local current_user = get_current_user()

  -- Get current month/year from params or use today
  local now = os.date("*t")
  local year = tonumber(self.params.year) or now.year
  local month = tonumber(self.params.month) or now.month
  local view = self.params.view or "month"

  -- Get events for the month
  local events = MailboxEvent.for_month(current_user._key, year, month)

  -- Get folder counts for sidebar
  local folder_counts = MailboxMessage.folder_counts(current_user._key)

  -- Build calendar data
  local calendar = build_calendar_month(year, month, events)

  local view_data = {
    current_user = current_user,
    folder_counts = folder_counts,
    current_folder = "calendar",
    year = year,
    month = month,
    month_name = os.date("%B", os.time({ year = year, month = month, day = 1 })),
    view = view,
    calendar = calendar,
    events = events,
    today = now
  }

  if self:is_htmx_request() then
    self.layout = false
    if view == "week" then
      return self:render("mailbox/calendar/_week_view", view_data)
    else
      return self:render("mailbox/calendar/_month_view", view_data)
    end
  end

  self.layout = "application"
  self:render("mailbox/calendar/index", view_data)
end

-- Get events for a date range (HTMX)
function CalendarController:events()
  local current_user = get_current_user()

  local year = tonumber(self.params.year) or os.date("*t").year
  local month = tonumber(self.params.month) or os.date("*t").month

  local events = MailboxEvent.for_month(current_user._key, year, month)

  self.layout = false
  self:render("mailbox/calendar/_events_list", {
    events = events,
    current_user = current_user
  })
end

-- Show single event
function CalendarController:show()
  local current_user = get_current_user()
  local event_id = self.params.id

  local event = MailboxEvent:find(event_id)
  if not event then
    return self:json({ error = "Event not found" }, 404)
  end

  local attendees = event:attendees_with_info()
  local user_status = event:attendee_status(current_user._key)

  if self:is_htmx_request() then
    self.layout = false
    return self:render("mailbox/calendar/_event_details", {
      event = event,
      attendees = attendees,
      user_status = user_status,
      current_user = current_user
    })
  end

  return self:json({
    event = event.data,
    attendees = attendees,
    user_status = user_status
  })
end

-- Create event modal
function CalendarController:modal_create()
  local current_user = get_current_user()
  local users = get_all_users()

  -- Pre-fill date if provided
  local date = self.params.date
  local start_time = nil
  if date then
    local y, m, d = date:match("(%d+)-(%d+)-(%d+)")
    if y and m and d then
      start_time = os.time({ year = tonumber(y), month = tonumber(m), day = tonumber(d), hour = 9, min = 0, sec = 0 })
    end
  end

  self.layout = false
  self:render("mailbox/calendar/_event_modal", {
    current_user = current_user,
    users = users,
    event = nil,
    start_time = start_time,
    colors = MailboxEvent.COLORS
  })
end

-- Edit event modal
function CalendarController:modal_edit()
  local current_user = get_current_user()
  local event_id = self.params.id

  local event = MailboxEvent:find(event_id)
  if not event then
    self.layout = false
    return self:html('<div class="text-red-400">Event not found</div>')
  end

  -- Only organizer can edit
  local organizer_key = event.organizer_key or event.data.organizer_key
  if organizer_key ~= current_user._key then
    self.layout = false
    return self:html('<div class="text-red-400">Only the organizer can edit this event</div>')
  end

  local users = get_all_users()

  self.layout = false
  self:render("mailbox/calendar/_event_modal", {
    current_user = current_user,
    users = users,
    event = event,
    colors = MailboxEvent.COLORS
  })
end

-- Create event
function CalendarController:create()
  local current_user = get_current_user()

  local title = self.params.title
  local description = self.params.description or ""
  local location = self.params.location or ""
  local start_date = self.params.start_date
  local start_time_str = self.params.start_time or "09:00"
  local end_date = self.params.end_date
  local end_time_str = self.params.end_time or "10:00"
  local all_day = self.params.all_day == "true" or self.params.all_day == "1"
  local color = self.params.color or MailboxEvent.COLORS[1]
  local attendees_str = self.params.attendees or ""

  if not title or title == "" then
    return self:json({ error = "Title is required" }, 400)
  end

  if not start_date or start_date == "" then
    return self:json({ error = "Start date is required" }, 400)
  end

  -- Parse start time
  local sy, sm, sd = start_date:match("(%d+)-(%d+)-(%d+)")
  local sh, smin = start_time_str:match("(%d+):(%d+)")
  if not sy or not sm or not sd then
    return self:json({ error = "Invalid start date format" }, 400)
  end

  local start_time = os.time({
    year = tonumber(sy),
    month = tonumber(sm),
    day = tonumber(sd),
    hour = tonumber(sh) or 9,
    min = tonumber(smin) or 0,
    sec = 0
  })

  -- Parse end time
  local end_time = start_time + 3600 -- Default 1 hour
  if end_date and end_date ~= "" then
    local ey, em, ed = end_date:match("(%d+)-(%d+)-(%d+)")
    local eh, emin = end_time_str:match("(%d+):(%d+)")
    if ey and em and ed then
      end_time = os.time({
        year = tonumber(ey),
        month = tonumber(em),
        day = tonumber(ed),
        hour = tonumber(eh) or 10,
        min = tonumber(emin) or 0,
        sec = 0
      })
    end
  end

  -- Parse attendees
  local attendee_keys = {}
  for key in string.gmatch(attendees_str, "[^,]+") do
    local trimmed = key:match("^%s*(.-)%s*$")
    if trimmed and trimmed ~= "" then
      table.insert(attendee_keys, trimmed)
    end
  end

  local event = MailboxEvent.create_event(
    current_user,
    title,
    description,
    location,
    start_time,
    end_time,
    all_day,
    attendee_keys,
    color
  )

  if self:is_htmx_request() then
    SetHeader("HX-Trigger", "eventCreated")
    SetHeader("HX-Redirect", "/mailbox/calendar")
    return self:html("")
  end

  return self:json({ success = true, event_key = event._key or event.data._key })
end

-- Update event
function CalendarController:update()
  local current_user = get_current_user()
  local event_id = self.params.id

  local event = MailboxEvent:find(event_id)
  if not event then
    return self:json({ error = "Event not found" }, 404)
  end

  -- Only organizer can update
  local organizer_key = event.organizer_key or event.data.organizer_key
  if organizer_key ~= current_user._key then
    return self:json({ error = "Only the organizer can edit this event" }, 403)
  end

  local title = self.params.title
  local description = self.params.description
  local location = self.params.location
  local start_date = self.params.start_date
  local start_time_str = self.params.start_time or "09:00"
  local end_date = self.params.end_date
  local end_time_str = self.params.end_time or "10:00"
  local all_day = self.params.all_day == "true" or self.params.all_day == "1"
  local color = self.params.color

  local updates = {}

  if title then updates.title = title end
  if description then updates.description = description end
  if location then updates.location = location end
  if color then updates.color = color end
  updates.all_day = all_day

  -- Parse times if provided
  if start_date and start_date ~= "" then
    local sy, sm, sd = start_date:match("(%d+)-(%d+)-(%d+)")
    local sh, smin = start_time_str:match("(%d+):(%d+)")
    if sy and sm and sd then
      updates.start_time = os.time({
        year = tonumber(sy),
        month = tonumber(sm),
        day = tonumber(sd),
        hour = tonumber(sh) or 9,
        min = tonumber(smin) or 0,
        sec = 0
      })
    end
  end

  if end_date and end_date ~= "" then
    local ey, em, ed = end_date:match("(%d+)-(%d+)-(%d+)")
    local eh, emin = end_time_str:match("(%d+):(%d+)")
    if ey and em and ed then
      updates.end_time = os.time({
        year = tonumber(ey),
        month = tonumber(em),
        day = tonumber(ed),
        hour = tonumber(eh) or 10,
        min = tonumber(emin) or 0,
        sec = 0
      })
    end
  end

  event:update(updates)

  if self:is_htmx_request() then
    SetHeader("HX-Trigger", "eventUpdated")
    SetHeader("HX-Redirect", "/mailbox/calendar")
    return self:html("")
  end

  return self:json({ success = true })
end

-- Delete event
function CalendarController:delete()
  local current_user = get_current_user()
  local event_id = self.params.id

  local event = MailboxEvent:find(event_id)
  if not event then
    return self:json({ error = "Event not found" }, 404)
  end

  -- Only organizer can delete
  local organizer_key = event.organizer_key or event.data.organizer_key
  if organizer_key ~= current_user._key then
    return self:json({ error = "Only the organizer can delete this event" }, 403)
  end

  event:destroy()

  if self:is_htmx_request() then
    SetHeader("HX-Trigger", "eventDeleted")
    return self:html("")
  end

  return self:json({ success = true })
end

-- Respond to event invitation
function CalendarController:respond()
  local current_user = get_current_user()
  local event_id = self.params.id
  local status = self.params.status

  if not status or not (status == "accepted" or status == "declined" or status == "tentative") then
    return self:json({ error = "Invalid status" }, 400)
  end

  local event = MailboxEvent:find(event_id)
  if not event then
    return self:json({ error = "Event not found" }, 404)
  end

  local updated = event:respond(current_user._key, status)

  if self:is_htmx_request() then
    self.layout = false
    return self:render("mailbox/calendar/_respond_buttons", {
      event = event,
      user_status = status
    })
  end

  return self:json({ success = updated, status = status })
end

-- Helper: Build calendar month grid
function build_calendar_month(year, month, events)
  local first_day = os.time({ year = year, month = month, day = 1, hour = 12 })
  local first_weekday = tonumber(os.date("%w", first_day)) -- 0 = Sunday

  -- Get number of days in month
  local next_month = month == 12 and 1 or month + 1
  local next_year = month == 12 and year + 1 or year
  local last_day = os.time({ year = next_year, month = next_month, day = 1, hour = 12 }) - 86400
  local days_in_month = tonumber(os.date("%d", last_day))

  -- Build event lookup by day
  local events_by_day = {}
  for _, event in ipairs(events) do
    local start_time = event.start_time or event.data.start_time
    if start_time then
      local day = tonumber(os.date("%d", start_time))
      if not events_by_day[day] then
        events_by_day[day] = {}
      end
      table.insert(events_by_day[day], event)
    end
  end

  -- Build weeks
  local weeks = {}
  local current_week = {}

  -- Pad first week with empty days
  for _ = 1, first_weekday do
    table.insert(current_week, { day = nil, events = {} })
  end

  -- Add days
  local today = os.date("*t")
  for day = 1, days_in_month do
    local is_today = (year == today.year and month == today.month and day == today.day)

    table.insert(current_week, {
      day = day,
      is_today = is_today,
      events = events_by_day[day] or {},
      date = string.format("%04d-%02d-%02d", year, month, day)
    })

    if #current_week == 7 then
      table.insert(weeks, current_week)
      current_week = {}
    end
  end

  -- Pad last week
  while #current_week > 0 and #current_week < 7 do
    table.insert(current_week, { day = nil, events = {} })
  end
  if #current_week > 0 then
    table.insert(weeks, current_week)
  end

  return {
    weeks = weeks,
    days_in_month = days_in_month
  }
end

return CalendarController
