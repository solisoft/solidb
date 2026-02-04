-- API Documentation Controller
-- Generates OpenAPI/Swagger documentation from Lua script annotations

local DashboardBaseController = require("dashboard/base_controller")
local ApiDocsController = DashboardBaseController:extend()

function ApiDocsController:index()
  self.layout = "dashboard"
  local db = self:get_db()
  local selected_service = self.params.service or GetParam("service")

  -- Fetch all services
  local services = {}
  local svc_status, _, svc_body = self:fetch_api("/_api/database/" .. db .. "/services")
  if svc_status == 200 then
    local ok, data = pcall(DecodeJson, svc_body)
    if ok and data and data.services then
      services = data.services
    end
  end

  -- Fetch all scripts list
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/scripts")
  local scripts = {}
  local documented_count = 0

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      -- The list endpoint doesn't include code, so fetch each script individually
      for _, script in ipairs(data.scripts or {}) do
        -- Filter by service if specified
        if not selected_service or script.service == selected_service then
          local script_id = script.id or script._key or script.key
          if script_id then
            local s_status, _, s_body = self:fetch_api("/_api/database/" .. db .. "/scripts/" .. script_id)
            if s_status == 200 then
              local s_ok, full_script = pcall(DecodeJson, s_body)
              if s_ok and full_script then
                table.insert(scripts, full_script)
                -- Count scripts with documentation
                if full_script.code and full_script.code:match("%-%-%-@") then
                  documented_count = documented_count + 1
                end
              else
                table.insert(scripts, script)
              end
            else
              table.insert(scripts, script)
            end
          else
            table.insert(scripts, script)
          end
        end
      end
    end
  end

  -- Get API server URL from cookie
  local api_server = GetCookie("sdb_server") or "http://localhost:6745"

  self:render("dashboard/apidocs", {
    title = "API Documentation - " .. db,
    db = db,
    current_page = "api_docs",
    scripts = scripts,
    services = services,
    selected_service = selected_service,
    documented_count = documented_count,
    api_server = api_server
  })
end

function ApiDocsController:openapi()
  local db = self:get_db()
  local selected_service = self.params.service or GetParam("service")

  -- Fetch scripts list
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/scripts")
  local scripts = {}
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      -- The list endpoint doesn't include code, so fetch each script individually
      for _, script in ipairs(data.scripts or {}) do
        -- Filter by service if specified
        if not selected_service or script.service == selected_service then
          local script_id = script.id or script._key or script.key
          if script_id then
            local s_status, _, s_body = self:fetch_api("/_api/database/" .. db .. "/scripts/" .. script_id)
            if s_status == 200 then
              local s_ok, full_script = pcall(DecodeJson, s_body)
              if s_ok and full_script then
                table.insert(scripts, full_script)
              else
                table.insert(scripts, script) -- Fallback to list data
              end
            else
              table.insert(scripts, script) -- Fallback to list data
            end
          else
            table.insert(scripts, script)
          end
        end
      end
    end
  end

  -- Return scripts as JSON (parsing done client-side)
  self:json({
    scripts = scripts,
    database = db,
    selected_service = selected_service
  })
end

return ApiDocsController
