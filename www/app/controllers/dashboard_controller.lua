-- Dashboard Controller for SoliDB Admin UI
-- Main controller handling auth and index page
-- Note: Protected routes use "dashboard_auth" middleware

local Controller = require("controller")
local DashboardController = Controller:extend()
local AuthHelper = require("helpers.auth_helper")

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

-- Sidebar Widgets
function DashboardController:sidebar_tasks_progress()
  local user = AuthHelper.get_current_user()
  if not user then return self:html("") end

  local Task = require("models.task")
  local tasks = Task.in_progress_for_user(user._key, 5)

  self.layout = false
  self:render("shared/_widget_tasks_progress", { tasks = tasks })
end

function DashboardController:sidebar_tasks_priority()
  local user = AuthHelper.get_current_user()
  if not user then return self:html("") end

  local Task = require("models.task")
  -- Use model method to get high priority todos (strictly status == "todo")
  local tasks = Task.todo_for_user(user._key, 5)

  self.layout = false
  self:render("shared/_widget_tasks_priority", { tasks = tasks })
end

function DashboardController:sidebar_pending_mrs()
  local user = AuthHelper.get_current_user()
  if not user then return self:html("") end

  local MergeRequest = require("models.merge_request")
  -- Fetch open MRs (global for now, or filtered by user's projects if needed)
  local result = Sdb:Sdbql([[
    FOR mr IN merge_requests
      FILTER mr.status == "open"
      SORT mr.created_at DESC
      LIMIT 5
      RETURN mr
  ]], {})

  local mrs = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(mrs, MergeRequest:new(doc))
    end
  end

  self.layout = false
  self:render("shared/_widget_merge_requests", { mrs = mrs })
end

function DashboardController:sidebar_recent_messages()
  local user = AuthHelper.get_current_user()
  if not user then return self:html("") end

  local Message = require("models.message")
  -- Fetch recent messages from channels the user is in
  -- (Complex query: User -> Channels -> Messages)
  -- Simplified for MVP: Messages from any channel, assuming public or user has access
  -- Ideally: Join with subscriptions/memberships

  local result = Sdb:Sdbql([[
    FOR m IN messages
      SORT m.timestamp DESC
      LIMIT 5
      LET sender = (FOR u IN users FILTER u._key == m.user_key RETURN {firstname: u.firstname})[0]
      RETURN MERGE(m, {sender_name: sender.firstname})
  ]], {})

  local messages = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(messages, Message:new(doc))
    end
  end

  self.layout = false
  self:render("shared/_widget_recent_messages", { messages = messages })
end

return DashboardController
