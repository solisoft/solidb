-- Dashboard Base Controller
-- Shared functionality for all dashboard controllers
-- Note: Authentication is handled by "dashboard_auth" middleware in routes

local Controller = require("controller")
local DashboardBaseController = Controller:extend()

-- Helper to get database name from params
function DashboardBaseController:get_db()
  return self.params.db or "_system"
end

-- Read the sdb_server cookie. The login controller base64-encodes the URL
-- because redbean's SetCookie silently drops values containing "://". Fall
-- back to the raw value for legacy unencoded cookies still in browsers, and
-- to localhost only as a last resort.
local function read_sdb_server_cookie()
  local raw = GetCookie("sdb_server")
  if not raw or raw == "" then
    return nil
  end
  local ok, decoded = pcall(DecodeBase64, raw)
  if ok and decoded and decoded:match("^https?://") then
    return decoded
  end
  return raw
end

-- Public accessor for callers that build URLs themselves (WebSocket targets,
-- API docs server field, raw Fetch calls outside fetch_api).
function DashboardBaseController:get_server_url()
  return read_sdb_server_cookie() or "http://localhost:6745"
end

-- Helper to make authenticated API calls to SoliDB backend
function DashboardBaseController:fetch_api(path, options)
  local server_url = self:get_server_url()
  local token = GetCookie("sdb_token")

  -- Ensure no double slashes
  if server_url:sub(-1) == "/" then server_url = server_url:sub(1, -2) end
  if path:sub(1, 1) ~= "/" then path = "/" .. path end

  options = options or {}
  options.headers = options.headers or {}
  options.headers["Authorization"] = "Bearer " .. (token or "")
  options.headers["Content-Type"] = "application/json"

  local start_time = GetTime()

  local status, headers, body = Fetch(server_url .. path, options)

  local elapsed_ms = (GetTime() - start_time) / 1000
  P(string.format("[TIMING] Fetch %s took %.2fms (status: %d)", path, elapsed_ms, status or 0))

  return status, headers, body
end

-- Helper for GET requests
function DashboardBaseController:api_get(path)
  local status, headers, body = self:fetch_api(path, { method = "GET" })
  return body, status
end

-- Helper for POST requests
function DashboardBaseController:api_post(path, body)
  local status, headers, response_body = self:fetch_api(path, {
    method = "POST",
    body = body
  })
  return response_body, status
end

-- Helper for DELETE requests
function DashboardBaseController:api_delete(path)
  local status, headers, body = self:fetch_api(path, { method = "DELETE" })
  return body, status
end

-- Helper to check if collection is columnar
function DashboardBaseController:is_columnar_collection(db, collection)
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/columnar/" .. collection)
  return status == 200
end

return DashboardBaseController
