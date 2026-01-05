-- Dashboard Query Controller
-- Handles query editor, REPL, and scripts
local DashboardBaseController = require("dashboard.base_controller")
local QueryController = DashboardBaseController:extend()

-- Query editor page
function QueryController:index()
  self.layout = "dashboard"
  self:render("dashboard/query", {
    title = "Query - " .. self:get_db(),
    db = self:get_db(),
    current_page = "query"
  })
end

-- Execute query (HTMX)
function QueryController:execute()
  local query = self.params.query or ""
  local query_type = self.params.type or "sdbql"
  local db_name = self:get_db()

  if query == "" then
    self:html('<div class="p-4 text-error">Query cannot be empty</div>')
    return
  end

  -- Call the appropriate API endpoint
  local endpoint
  if query_type == "sql" then
    endpoint = "/_api/database/" .. db_name .. "/sql"
  else
    endpoint = "/_api/database/" .. db_name .. "/cursor"
  end

  local status, headers, body = self:fetch_api(endpoint, {
    method = "POST",
    body = EncodeJson({ query = query })
  })

  if status and status >= 200 and status < 300 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      local results = data.result or {}
      local count = type(results) == "table" and #results or 0
      self:render_partial("dashboard/_query_results", {
        db = db_name,
        query = query,
        results = results,
        stats = {
          count = count,
          has_more = data.hasMore or false,
          cursor_id = data.id,
          execution_time_ms = data.executionTimeMs,
          documents_inserted = data.documentsInserted or 0,
          documents_updated = data.documentsUpdated or 0,
          documents_removed = data.documentsRemoved or 0
        }
      })
    else
      self:html('<div class="p-4 text-error">Failed to parse response</div>')
    end
  else
    local ok, err = pcall(DecodeJson, body or "")
    local error_msg = "Query failed (status: " .. tostring(status) .. ")"
    if ok and err and type(err) == "table" then
      error_msg = err.error or err.message or error_msg
    end
    self:html('<div class="p-4 text-error">' .. error_msg .. '</div>')
  end
end

-- Explain query (HTMX)
function QueryController:explain()
  local query = self.params.query or ""
  local query_type = self.params.type or "sdbql"
  local db_name = self:get_db()

  if query == "" then
    self:html('<div class="p-4 text-error">Query cannot be empty</div>')
    return
  end

  -- For SQL, translate to SDBQL first
  local sdbql_query = query
  if query_type == "sql" then
    local t_status, _, t_body = self:fetch_api("/_api/sql/translate", {
      method = "POST",
      body = EncodeJson({ query = query })
    })
    if t_status and t_status >= 200 and t_status < 300 then
      local ok, translated = pcall(DecodeJson, t_body)
      if ok and translated then
        sdbql_query = translated.sdbql or query
      end
    else
      self:html('<div class="p-4 text-error">SQL translation failed (status: ' .. tostring(t_status) .. ')</div>')
      return
    end
  end

  -- Call explain endpoint
  local status, _, body = self:fetch_api("/_api/database/" .. db_name .. "/explain", {
    method = "POST",
    body = EncodeJson({ query = sdbql_query })
  })

  if status and status >= 200 and status < 300 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      self:render_partial("dashboard/_query_explain", {
        db = db_name,
        query = query,
        sdbql_query = query_type == "sql" and sdbql_query or nil,
        explain = data
      })
    else
      self:html('<div class="p-4 text-error">Failed to parse explain response</div>')
    end
  else
    local ok, err = pcall(DecodeJson, body or "")
    local error_msg = "Explain failed (status: " .. tostring(status) .. ")"
    if ok and err and type(err) == "table" then
      error_msg = err.error or err.message or error_msg
    end
    self:html('<div class="p-4 text-error">' .. error_msg .. '</div>')
  end
end

-- Lua REPL page
function QueryController:repl()
  self.layout = "dashboard"
  self:render("dashboard/repl", {
    title = "Lua REPL - " .. self:get_db(),
    db = self:get_db(),
    current_page = "repl"
  })
end

-- REPL Execute
function QueryController:repl_execute()
  local db_name = self:get_db()

  -- Get code from params (LuaOnBeans parses JSON body automatically)
  local code = self.params.code or ""
  local session_id = self.params.session_id

  if code == "" then
    self:json({ error = { message = "No code provided" } })
    return
  end

  -- Call the backend REPL API
  local request_body = { code = code }
  if session_id then
    request_body.session_id = session_id
  end

  local status, headers, response_body = self:fetch_api("/_api/database/" .. db_name .. "/repl", {
    method = "POST",
    body = EncodeJson(request_body)
  })

  if status and status >= 200 and status < 300 then
    local ok, data = pcall(DecodeJson, response_body)
    if ok and data then
      self:json(data)
    else
      self:json({ error = { message = "Failed to parse response" } })
    end
  else
    local ok, err_data = pcall(DecodeJson, response_body or "")
    local error_msg = "Execution failed (status: " .. tostring(status) .. ")"
    if ok and err_data and type(err_data) == "table" then
      error_msg = err_data.error or err_data.message or error_msg
    end
    self:json({ error = { message = error_msg } })
  end
end

-- Scripts manager page
function QueryController:scripts()
  self.layout = "dashboard"
  self:render("dashboard/scripts", {
    title = "Scripts - " .. self:get_db(),
    db = self:get_db(),
    current_page = "scripts"
  })
end

-- Scripts table (HTMX partial)
function QueryController:scripts_table()
  local db = self:get_db()
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/scripts")

  local scripts = {}
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      scripts = data.scripts or {}
    end
  end

  self:render_partial("dashboard/_scripts_table", {
    scripts = scripts,
    db = db
  })
end

-- Scripts stats (HTMX partial)
function QueryController:scripts_stats()
  local status, _, body = self:fetch_api("/_api/scripts/stats")

  local stats = {
    active_scripts = 0,
    active_ws = 0,
    total_scripts_executed = 0,
    total_ws_connections = 0
  }

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      stats = data
    end
  end

  local db = self:get_db()
  -- Also get scripts count
  local s_status, _, s_body = self:fetch_api("/_api/database/" .. db .. "/scripts")
  local scripts_count = 0
  if s_status == 200 then
    local ok, data = pcall(DecodeJson, s_body)
    if ok and data and data.scripts then
      scripts_count = #data.scripts
    end
  end

  self:render_partial("dashboard/_scripts_stats", {
    stats = stats,
    scripts_count = scripts_count
  })
end

-- Scripts create modal
function QueryController:scripts_modal_create()
  self:render_partial("dashboard/_modal_script", {
    db = self:get_db(),
    script = nil,
    is_edit = false
  })
end

-- Scripts edit modal
function QueryController:scripts_modal_edit()
  local db = self:get_db()
  local script_id = self.params.script_id

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/scripts/" .. script_id)

  if status == 200 then
    local ok, script = pcall(DecodeJson, body)
    if ok and script then
      self:render_partial("dashboard/_modal_script", {
        db = db,
        script = script,
        is_edit = true
      })
      return
    end
  end

  self:html('<div class="p-4 text-error">Failed to load script</div>')
end

-- Create script action
function QueryController:create_script()
  local db = self:get_db()

  local request_body = {
    name = self.params.name or "",
    path = self.params.path or "",
    methods = {},
    code = self.params.code or "",
    description = self.params.description
  }

  -- Parse methods from form checkboxes
  for _, method in ipairs({"GET", "POST", "PUT", "DELETE", "WS"}) do
    if self.params["method_" .. method] then
      table.insert(request_body.methods, method)
    end
  end

  if #request_body.methods == 0 then
    request_body.methods = {"GET"}
  end

  if request_body.name == "" or request_body.path == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Name and Path are required", "type": "error"}}')
    return self:scripts_table()
  end

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/scripts", {
    method = "POST",
    body = EncodeJson(request_body)
  })

  if status == 200 or status == 201 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Script created successfully", "type": "success"}, "closeModal": true, "refreshStats": true}')
  else
    local err_msg = "Failed to create script"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:scripts_table()
end

-- Update script action
function QueryController:update_script()
  local db = self:get_db()
  local script_id = self.params.script_id

  local request_body = {
    name = self.params.name or "",
    path = self.params.path or "",
    methods = {},
    code = self.params.code or "",
    description = self.params.description
  }

  -- Parse methods from form checkboxes
  for _, method in ipairs({"GET", "POST", "PUT", "DELETE", "WS"}) do
    if self.params["method_" .. method] then
      table.insert(request_body.methods, method)
    end
  end

  if #request_body.methods == 0 then
    request_body.methods = {"GET"}
  end

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/scripts/" .. script_id, {
    method = "PUT",
    body = EncodeJson(request_body)
  })

  if status == 200 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Script updated successfully", "type": "success"}, "closeModal": true, "refreshStats": true}')
  else
    local err_msg = "Failed to update script"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:scripts_table()
end

-- Delete script action
function QueryController:delete_script()
  local db = self:get_db()
  local script_id = self.params.script_id

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/scripts/" .. script_id, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Script deleted successfully", "type": "success"}, "refreshStats": true}')
    self:html("")
  else
    local err_msg = "Failed to delete script"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
    self:html("")
  end
end

-- Live Query page
-- Live Query page
function QueryController:live_query()
  self.layout = "dashboard"
  local db = self:get_db()
  
  -- Fetch Live Query Token
  local t_status, _, t_body = self:fetch_api("/_api/livequery/token")
  local token = ""
  if t_status == 200 then
    local ok, data = pcall(DecodeJson, t_body)
    if ok and data and data.token then
      token = data.token
    end
  end
  
  -- Determine API URL for WS
  local api_url = GetCookie("sdb_server") or "http://localhost:6745"
  -- Remove protocol and trailing slash
  local ws_host = api_url:gsub("https?://", ""):gsub("/$", "")
  local ws_protocol = api_url:match("^https") and "wss" or "ws"
  local ws_url = ws_protocol .. "://" .. ws_host .. "/_api/ws/changefeed"

  self:render("dashboard/live_query", {
    title = "Live Query - " .. db,
    db = db,
    current_page = "live_query",
    livequery_token = token,
    ws_url = ws_url
  })
end

return QueryController
