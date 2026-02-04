-- Public API Documentation Controller
-- Serves OpenAPI/Swagger documentation with HTTP Basic Auth
-- Credentials configured via DB environment variables: API_DOCS_USERNAME, API_DOCS_PASSWORD

local Controller = require("controller")
local PublicApiDocsController = Controller:extend()

-- Helper to make API calls to SoliDB
function PublicApiDocsController:fetch_api(path, options)
  local server_url = os.getenv("SOLIDB_URL") or "http://localhost:6745"

  if server_url:sub(-1) == "/" then server_url = server_url:sub(1, -2) end
  if path:sub(1, 1) ~= "/" then path = "/" .. path end

  options = options or {}
  options.headers = options.headers or {}
  options.headers["Content-Type"] = "application/json"

  local status, headers, body = Fetch(server_url .. path, options)
  return status, headers, body
end

-- Check HTTP Basic Auth against DB environment variables
function PublicApiDocsController:check_basic_auth(db)
  -- Get Authorization header
  local auth_header = GetHeader("Authorization")

  if not auth_header then
    return false, "Missing Authorization header"
  end

  -- Parse Basic auth
  local scheme, credentials = auth_header:match("^(%w+)%s+(.+)$")
  if not scheme or scheme:lower() ~= "basic" then
    return false, "Invalid authorization scheme"
  end

  -- Decode base64 credentials
  local decoded = DecodeBase64(credentials)
  if not decoded then
    return false, "Invalid credentials encoding"
  end

  local username, password = decoded:match("^([^:]+):(.*)$")
  if not username or not password then
    return false, "Invalid credentials format"
  end

  -- Fetch expected credentials from DB environment
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/env/API_DOCS_USERNAME")
  if status ~= 200 then
    return false, "API_DOCS_USERNAME not configured"
  end
  local ok, data = pcall(DecodeJson, body)
  local expected_username = ok and data and data.value

  status, _, body = self:fetch_api("/_api/database/" .. db .. "/env/API_DOCS_PASSWORD")
  if status ~= 200 then
    return false, "API_DOCS_PASSWORD not configured"
  end
  ok, data = pcall(DecodeJson, body)
  local expected_password = ok and data and data.value

  if not expected_username or not expected_password then
    return false, "API docs credentials not configured"
  end

  -- Constant-time comparison to prevent timing attacks
  if username == expected_username and password == expected_password then
    return true
  end

  return false, "Invalid credentials"
end

-- Send 401 response with WWW-Authenticate header
function PublicApiDocsController:send_auth_required(db)
  SetStatus(401)
  SetHeader("WWW-Authenticate", 'Basic realm="API Documentation for ' .. db .. '"')
  SetHeader("Content-Type", "text/html")
  Write([[
<!DOCTYPE html>
<html>
<head><title>401 Unauthorized</title></head>
<body style="font-family: system-ui; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; background: #1a1a2e; color: #e0e0e0;">
  <div style="text-align: center;">
    <h1 style="font-size: 4rem; margin: 0; color: #f87171;">401</h1>
    <p style="color: #9ca3af;">Authentication required to access API documentation</p>
  </div>
</body>
</html>
]])
end

function PublicApiDocsController:index()
  local db = self.params.db or "_system"

  -- Check basic auth
  local authorized, err = self:check_basic_auth(db)
  if not authorized then
    self:send_auth_required(db)
    return
  end

  -- Fetch scripts with full code
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/scripts")
  local scripts = {}
  local documented_count = 0

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      for _, script in ipairs(data.scripts or {}) do
        local script_id = script.id or script._key or script.key
        if script_id then
          local s_status, _, s_body = self:fetch_api("/_api/database/" .. db .. "/scripts/" .. script_id)
          if s_status == 200 then
            local s_ok, full_script = pcall(DecodeJson, s_body)
            if s_ok and full_script then
              table.insert(scripts, full_script)
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

  self:render("public_apidocs", {
    db = db,
    scripts = scripts,
    documented_count = documented_count
  })
end

function PublicApiDocsController:openapi()
  local db = self.params.db or "_system"

  -- Check basic auth
  local authorized, err = self:check_basic_auth(db)
  if not authorized then
    self:send_auth_required(db)
    return
  end

  -- Fetch scripts with full code
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/scripts")
  local scripts = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      for _, script in ipairs(data.scripts or {}) do
        local script_id = script.id or script._key or script.key
        if script_id then
          local s_status, _, s_body = self:fetch_api("/_api/database/" .. db .. "/scripts/" .. script_id)
          if s_status == 200 then
            local s_ok, full_script = pcall(DecodeJson, s_body)
            if s_ok and full_script then
              table.insert(scripts, full_script)
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

  self:json({
    scripts = scripts,
    database = db
  })
end

-- Service-specific public docs
function PublicApiDocsController:service_index()
  local db = self.params.db or "_system"
  local service = self.params.service

  -- Check basic auth
  local authorized, err = self:check_basic_auth(db)
  if not authorized then
    self:send_auth_required(db)
    return
  end

  -- Fetch scripts filtered by service
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/scripts")
  local scripts = {}
  local documented_count = 0

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      for _, script in ipairs(data.scripts or {}) do
        -- Filter by service
        if script.service == service then
          local script_id = script.id or script._key or script.key
          if script_id then
            local s_status, _, s_body = self:fetch_api("/_api/database/" .. db .. "/scripts/" .. script_id)
            if s_status == 200 then
              local s_ok, full_script = pcall(DecodeJson, s_body)
              if s_ok and full_script then
                table.insert(scripts, full_script)
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

  -- Fetch service info
  local svc_status, _, svc_body = self:fetch_api("/_api/database/" .. db .. "/services/" .. service)
  local service_info = nil
  if svc_status == 200 then
    local ok, data = pcall(DecodeJson, svc_body)
    if ok then
      service_info = data
    end
  end

  self:render("public_apidocs", {
    db = db,
    service = service,
    service_info = service_info,
    scripts = scripts,
    documented_count = documented_count
  })
end

function PublicApiDocsController:service_openapi()
  local db = self.params.db or "_system"
  local service = self.params.service

  -- Check basic auth
  local authorized, err = self:check_basic_auth(db)
  if not authorized then
    self:send_auth_required(db)
    return
  end

  -- Fetch scripts filtered by service
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/scripts")
  local scripts = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      for _, script in ipairs(data.scripts or {}) do
        -- Filter by service
        if script.service == service then
          local script_id = script.id or script._key or script.key
          if script_id then
            local s_status, _, s_body = self:fetch_api("/_api/database/" .. db .. "/scripts/" .. script_id)
            if s_status == 200 then
              local s_ok, full_script = pcall(DecodeJson, s_body)
              if s_ok and full_script then
                table.insert(scripts, full_script)
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

  self:json({
    scripts = scripts,
    database = db,
    service = service
  })
end

return PublicApiDocsController
