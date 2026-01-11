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
  if P then
    P("DEBUG PARAMS DUMP:")
    P("Path: " .. tostring(GetPath()))
    -- Dump params
    for k, v in pairs(self.params) do
       P("Param [" .. tostring(k) .. "] = " .. tostring(v))
    end
    -- Try direct retrieval
    P("Direct view check: " .. tostring(GetParam("view")))
  end

  local now = os.date("*t")
  local year = tonumber(self.params.year) or tonumber(GetParam("year")) or now.year
  local month = tonumber(self.params.month) or tonumber(GetParam("month")) or now.month
  local view = self.params.view or GetParam("view") or "month"

  -- Get events for the month
  local events = MailboxEvent.for_month(current_user._key, year, month)

  -- Get folder counts for sidebar
  local folder_counts = MailboxMessage.folder_counts(current_user._key)

  -- Sanitize view parameter
  local valid_views = { month = true, week = true, day = true, year = true }
  if not valid_views[view] then
    view = "month"
  end
  
  -- Build calendar data
  -- Determine effective day for current_date object
  local view_day = tonumber(self.params.day) or tonumber(GetParam("day")) or now.day
  if view == "month" or view == "year" then
    -- For month/year view nav, default to 1 if not specified, or clamp to valid day?
    -- Actually, keeping today's day number is fine unless it exceeds max days in target month.
    local last_day = os.time({ year = year, month = month + 1, day = 0 }) -- Month rollover trick in Lua? No, standard Date trick.
    local d = os.date("*t", last_day) -- Wait, let's use standard max day calc.
    local next_m_ts = os.time({ year = year, month = month + 1, day = 1 })
    local last_day_ts = next_m_ts - 86400
    local max_day = tonumber(os.date("%d", last_day_ts))
    if view_day > max_day then view_day = max_day end
  end

  local current_date = { year = year, month = month, day = view_day, hour = 12, min = 0, sec = 0 }
  
  if view == "month" then
    calendar = build_calendar_month(year, month, events)
  elseif view == "week" then
    local day = tonumber(self.params.day) or tonumber(GetParam("day")) or now.day
    -- Removed restriction on month match
    
    calendar = build_calendar_week(year, month, day, events)
    -- current_date already set
  elseif view == "day" then
    local day = tonumber(self.params.day) or tonumber(GetParam("day")) or now.day
    calendar = { year = year, month = month, day = day }
    -- current_date already set
  elseif view == "year" then
    calendar = { year = year }
  end
  
  if not calendar then
    P("ERROR: Calendar is nil for view: " .. tostring(view))
    calendar = {} -- Fallback to empty table to prevent crash
  end

  local view_data = {
    current_user = current_user,
    folder_counts = folder_counts,
    current_folder = "calendar",
    year = year,
    month = month,
    month_name = os.date("%B", os.time({ year = year, month = month, day = 1 })),
    view = view,
    calendar = calendar,
    current_date = current_date,
    events = events,
    today = now
  }

  -- Print debug info
  if P then
    P("DEBUG: Calendar view=" .. tostring(view) .. " year=" .. tostring(year) .. " calendar exists=" .. tostring(calendar ~= nil))
  end

  if self:is_htmx_request() then
    self.layout = false
    return self:render("mailbox/calendar/_main_content", view_data)
  end

  self.layout = "application"
  self.full_height = true
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
    self:set_header("HX-Trigger", "eventCreated")
    self:set_header("HX-Redirect", "/mailbox/calendar")
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
    self:set_header("HX-Trigger", "eventUpdated")
    self:set_header("HX-Redirect", "/mailbox/calendar")
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
    self:set_header("HX-Trigger", "eventDeleted")
    return self:html("")
  end

  return self:json({ success = true })
end

function CalendarController:respond()
  local event_key = self.params.id
  local status = self.params.status
  local current_user = AuthHelper.get_current_user()

  if not current_user then
    return self:json({ error = "Unauthorized" }, 401)
  end

  if status ~= "accepted" and status ~= "declined" and status ~= "tentative" then
    return self:json({ error = "Invalid status" }, 400)
  end

  local event = MailboxEvent:find(event_key)
  if not event then
    return self:json({ error = "Event not found" }, 404)
  end

  local attendees = event.attendees or event.data.attendees or {}
  local found = false
  for i, attendee in ipairs(attendees) do
    if attendee.user_key == current_user._key then
      attendee.status = status
      found = true
      break
    end
  end

  if found then
    event:update({ attendees = attendees })
    
    if self:is_htmx_request() then
      self:set_header("HX-Trigger", '{"sidebar:invites": "true", "eventUpdated": "true"}')
      return self:html("")
    end
    
    return self:json({ success = true })
  else
    return self:json({ error = "User is not an attendee" }, 403)
  end
end



-- Helper: Build calendar week grid
function build_calendar_week(year, month, day, events)
  local reference_date = os.time({ year = year, month = month, day = day, hour = 12 })
  local weekday = tonumber(os.date("%w", reference_date)) -- 0 = Sunday
  
  -- Calculate start of week (Sunday)
  -- Lua os.time handles negative offsets correctly to go back to prev month
  local start_of_week = reference_date - (weekday * 86400)
  
  local week_days = {}
  local events_by_date = {}
  
  -- Pre-process events for quicker lookup (key: "YYYY-MM-DD")
  for _, event in ipairs(events) do
    local start_time = event.start_time or event.data.start_time
    if start_time then
      local date_key = os.date("%Y-%m-%d", start_time)
      if not events_by_date[date_key] then events_by_date[date_key] = {} end
      table.insert(events_by_date[date_key], event)
    end
  end

  for i = 0, 6 do
    local current_day_time = start_of_week + (i * 86400)
    local d_year = tonumber(os.date("%Y", current_day_time))
    local d_month = tonumber(os.date("%m", current_day_time))
    local d_day = tonumber(os.date("%d", current_day_time))
    local date_key = string.format("%04d-%02d-%02d", d_year, d_month, d_day)
    
    local today = os.date("*t")
    local is_today = (d_year == today.year and d_month == today.month and d_day == today.day)
    
    table.insert(week_days, {
      year = d_year,
      month = d_month,
      day = d_day,
      weekday = i, -- 0-6
      is_today = is_today,
      date_key = date_key,
      events = events_by_date[date_key] or {}
    })
  end
  
  return week_days
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
      -- Verify event is in this month/year (events list passed might be filtered but double check)
      local e_year = tonumber(os.date("%Y", start_time))
      local e_month = tonumber(os.date("%m", start_time))
      
      if e_year == year and e_month == month then
        local day = tonumber(os.date("%d", start_time))
        if not events_by_day[day] then
          events_by_day[day] = {}
        end
        table.insert(events_by_day[day], event)
      end
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
