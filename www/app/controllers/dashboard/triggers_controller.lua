-- Dashboard Triggers Controller
-- Handles collection triggers management
local DashboardBaseController = require("dashboard.base_controller")
local TriggersController = DashboardBaseController:extend()

-- Main triggers page
function TriggersController:index()
  self.layout = "dashboard"
  self:render("dashboard/triggers", {
    title = "Triggers - " .. self:get_db(),
    db = self:get_db(),
    current_page = "triggers"
  })
end

-- Triggers table (HTMX partial)
function TriggersController:table()
  local db = self:get_db()
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/triggers")

  local triggers = {}
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      triggers = data.triggers or data or {}
    end
  end

  self:render_partial("dashboard/_triggers_table", {
    triggers = triggers,
    db = db
  })
end

-- Create trigger modal
function TriggersController:modal_create()
  local db = self:get_db()

  -- Fetch available collections
  local collections = {}
  local c_status, _, c_body = self:fetch_api("/_api/database/" .. db .. "/collection")
  if c_status == 200 then
    local ok, data = pcall(DecodeJson, c_body)
    if ok and data then
      local coll_list = data.collections or data or {}
      for _, c in ipairs(coll_list) do
        -- Exclude system collections
        local name = type(c) == "table" and c.name or c
        if type(name) == "string" and name:sub(1, 1) ~= "_" then
          table.insert(collections, name)
        end
      end
    end
  end

  -- Fetch available scripts
  local scripts = {}
  local s_status, _, s_body = self:fetch_api("/_api/database/" .. db .. "/scripts")
  if s_status == 200 then
    local ok, data = pcall(DecodeJson, s_body)
    if ok and data then
      scripts = data.scripts or data or {}
    end
  end

  self:render_partial("dashboard/_modal_create_trigger", {
    db = db,
    collections = collections,
    scripts = scripts
  })
end

-- Edit trigger modal
function TriggersController:modal_edit()
  local db = self:get_db()
  local trigger_id = self.params.id

  -- Fetch trigger details
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/triggers/" .. trigger_id)
  if status ~= 200 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Trigger not found", "type": "error"}}')
    return self:html("")
  end

  local trigger = {}
  local ok, data = pcall(DecodeJson, body)
  if ok and data then
    trigger = data
  end

  -- Fetch available collections
  local collections = {}
  local c_status, _, c_body = self:fetch_api("/_api/database/" .. db .. "/collection")
  if c_status == 200 then
    local c_ok, c_data = pcall(DecodeJson, c_body)
    if c_ok and c_data then
      local coll_list = c_data.collections or c_data or {}
      for _, c in ipairs(coll_list) do
        local name = type(c) == "table" and c.name or c
        if type(name) == "string" and name:sub(1, 1) ~= "_" then
          table.insert(collections, name)
        end
      end
    end
  end

  -- Fetch available scripts
  local scripts = {}
  local s_status, _, s_body = self:fetch_api("/_api/database/" .. db .. "/scripts")
  if s_status == 200 then
    local s_ok, s_data = pcall(DecodeJson, s_body)
    if s_ok and s_data then
      scripts = s_data.scripts or s_data or {}
    end
  end

  self:render_partial("dashboard/_modal_edit_trigger", {
    db = db,
    trigger = trigger,
    collections = collections,
    scripts = scripts
  })
end

-- Create trigger action
function TriggersController:create()
  local db = self:get_db()
  local name = self.params.name
  local collection = self.params.collection
  local events_str = self.params.events or ""
  local script_path = self.params.script_path
  local queue = self.params.queue or "default"
  local priority = tonumber(self.params.priority) or 0
  local max_retries = tonumber(self.params.max_retries) or 5
  local enabled = self.params.enabled == "on" or self.params.enabled == "true" or self.params.enabled == "1"

  if not name or name == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Trigger name is required", "type": "error"}}')
    return self:table()
  end

  if not collection or collection == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Collection is required", "type": "error"}}')
    return self:table()
  end

  if not script_path or script_path == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Script path is required", "type": "error"}}')
    return self:table()
  end

  -- Parse events from comma-separated string
  local events = {}
  for event in string.gmatch(events_str, "[^,]+") do
    local trimmed = event:match("^%s*(.-)%s*$")
    if trimmed and trimmed ~= "" then
      table.insert(events, trimmed)
    end
  end

  if #events == 0 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "At least one event is required", "type": "error"}}')
    return self:table()
  end

  local request_body = {
    name = name,
    collection = collection,
    events = events,
    script_path = script_path,
    queue = queue,
    priority = priority,
    max_retries = max_retries,
    enabled = enabled
  }

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/triggers", {
    method = "POST",
    body = EncodeJson(request_body)
  })

  if status == 200 or status == 201 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Trigger created successfully", "type": "success"}, "closeModal": true}')
  else
    local err_msg = "Failed to create trigger"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:table()
end

-- Update trigger action
function TriggersController:update()
  local db = self:get_db()
  local trigger_id = self.params.id
  local name = self.params.name
  local collection = self.params.collection
  local events_str = self.params.events or ""
  local script_path = self.params.script_path
  local queue = self.params.queue
  local priority = tonumber(self.params.priority)
  local max_retries = tonumber(self.params.max_retries)
  local enabled = self.params.enabled == "on" or self.params.enabled == "true" or self.params.enabled == "1"

  -- Parse events
  local events = {}
  for event in string.gmatch(events_str, "[^,]+") do
    local trimmed = event:match("^%s*(.-)%s*$")
    if trimmed and trimmed ~= "" then
      table.insert(events, trimmed)
    end
  end

  local request_body = {
    name = name,
    collection = collection,
    events = #events > 0 and events or nil,
    script_path = script_path,
    queue = queue,
    priority = priority,
    max_retries = max_retries,
    enabled = enabled
  }

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/triggers/" .. trigger_id, {
    method = "PUT",
    body = EncodeJson(request_body)
  })

  if status == 200 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Trigger updated successfully", "type": "success"}, "closeModal": true}')
  else
    local err_msg = "Failed to update trigger"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:table()
end

-- Delete trigger action
function TriggersController:destroy()
  local db = self:get_db()
  local trigger_id = self.params.id

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/triggers/" .. trigger_id, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Trigger deleted successfully", "type": "success"}}')
    self:html("")
  else
    local err_msg = "Failed to delete trigger"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
    self:html("")
  end
end

-- Toggle trigger enabled/disabled
function TriggersController:toggle()
  local db = self:get_db()
  local trigger_id = self.params.id

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/triggers/" .. trigger_id .. "/toggle", {
    method = "POST"
  })

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    local state = "toggled"
    if ok and data then
      state = data.enabled and "enabled" or "disabled"
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "Trigger ' .. state .. '", "type": "success"}}')
  else
    local err_msg = "Failed to toggle trigger"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:table()
end

return TriggersController
