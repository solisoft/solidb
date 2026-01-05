-- Dashboard Controller for SoliDB Admin UI
-- Main controller handling auth and index page
-- Note: Protected routes use "dashboard_auth" middleware

local Controller = require("controller")
local DashboardController = Controller:extend()

-- Login page
function DashboardController:login()
  self.layout = "auth" -- Use a separate auth layout or none
  self:render("dashboard/login", {
    title = "Sign In - SoliDB",
    error_message = GetFlashMessage("error")
  })
end

-- Handle login POST
function DashboardController:do_login()
  local username = self.params.username
  local password = self.params.password
  local server_url = self.params.server_url or "http://localhost:6745"

  -- Ensure server_url has http/https
  if not server_url:match("^https?://") then
    server_url = "http://" .. server_url
  end

  Log(kLogInfo, "Attempting login for user: " .. tostring(username) .. " at " .. server_url)

  -- Call the SoliDB API
  local status, headers, body = Fetch(server_url .. "/auth/login", {
    method = "POST",
    headers = { ["Content-Type"] = "application/json" },
    body = EncodeJson({ username = username, password = password })
  })

  Log(kLogInfo, "Login API response status: " .. tostring(status))
  Log(kLogInfo, "Login API response body: " .. tostring(body))

  if status == 200 then
     local ok, response = pcall(DecodeJson, body)
     Log(kLogInfo, "JSON Decode status: " .. tostring(ok) .. ", result type: " .. type(response))

     if ok and response and response.token then
       Log(kLogInfo, "Login successful, token found. Setting cookies...")

       -- Use client-side redirect to ensure cookies are set (bypassing potential 302 header issues)
       local target_url = "/database/_system"

       self:redirect(target_url)

       self:set_cookie("sdb_token", response.token, {
        path = "/",
        http_only = true,
        same_site = "Lax",
        max_age = 3600 * 24 * 30 -- 30 days
       })

       self:set_cookie("sdb_server", server_url, {
        path = "/",
        http_only = true,
        same_site = "Lax",
        max_age = 3600 * 24 * 30 -- 30 days
       })
     else
       Log(kLogWarn, "Login failed: Invalid response or missing token. Response: " .. EncodeJson(response or {}))
       SetFlash("error", "Invalid response from server")
       self:redirect("/dashboard/login")
     end
  else
     Log(kLogWarn, "Login failed with status " .. tostring(status))
     local error_msg = "Authentication failed"
     if body and body ~= "" then
       local ok, err_resp = pcall(DecodeJson, body)
       if ok and err_resp.error then
         error_msg = err_resp.error
       end
     end
     SetFlash("error", error_msg)
     self:redirect("/dashboard/login")
  end
end

-- Logout
function DashboardController:logout()
  SetCookie("sdb_token", "", { MaxAge = 0, Path = "/" })
  SetCookie("sdb_server", "", { MaxAge = 0, Path = "/" })
  DestroySession()
  self:redirect("/dashboard/login")
end

-- Helper to get database name from params
function DashboardController:get_db()
  return self.params.db or "_system"
end

-- Dashboard index (protected by middleware)
function DashboardController:index()
  self.layout = "dashboard"
  self:render("dashboard/index", {
    title = "Dashboard - " .. self:get_db(),
    db = self:get_db(),
    current_page = "index"
  })
end

return DashboardController
