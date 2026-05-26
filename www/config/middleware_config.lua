-- Middleware configuration
-- Define global middleware that runs on all requests

local Middleware = require("middleware")

-- ============================================================================
-- Global Before Middleware (runs on all requests, in order)
-- ============================================================================

-- Uncomment to enable:
-- Middleware.use("request_logger")
-- Middleware.use("cors")
-- Middleware.use("security_headers")

-- ============================================================================
-- Global After Middleware (runs after controller, in reverse order)
-- ============================================================================

-- Uncomment to enable:
-- Middleware.after("response_logger")

-- ============================================================================
-- Custom Inline Middleware
-- ============================================================================

-- You can also define middleware inline:
-- Middleware.use(function(ctx, next)
--   -- Add custom header to all responses
--   ctx:set_header("X-Powered-By", "Luaonbeans")
--   next()
-- end)

-- ============================================================================
-- Named Middleware Registration (for route-scoped use)
-- ============================================================================

-- Session authentication middleware (for apps using Luaonbeans sessions)
Middleware.register("session_auth", function(ctx, next)
  local session = GetSession()
  if not session or not session.user_id then
    local current_path = GetPath() or "/"
    return ctx:redirect("/auth/login?redirect=" .. current_path)
  end

  -- Verify user still exists in database (safe require at runtime)
  -- Only destroy session if DB is reachable but user genuinely doesn't exist
  local ok, AuthHelper = pcall(require, "helpers.auth_helper")
  if ok and AuthHelper then
    local db = _G.Sdb
    if db then
      local res = db:Sdbql("FOR u IN users FILTER u._id == @id LIMIT 1 RETURN u._key", { id = session.user_id })
      if res and res.result then
        -- DB responded successfully
        if #res.result == 0 then
          -- User genuinely doesn't exist anymore - destroy session
          DestroySession()
          local current_path = GetPath() or "/"
          return ctx:redirect("/auth/login?redirect=" .. current_path)
        end
      end
      -- If res is nil or res.result is nil, DB had an error - keep session alive
    end
    -- If no DB, skip verification - keep session alive
  end

  -- Refresh session on each interaction to extend expiry (sliding expiration)
  SetSession(session)
  next()
end)

-- Read the sdb_server cookie, decoding the base64 wrapper set by the login
-- controller. Falls back to the raw cookie value for legacy unencoded
-- cookies still sitting in users' browsers, then to localhost as a last
-- resort.
local function read_sdb_server_cookie()
  local raw = GetCookie("sdb_server")
  if not raw or raw == "" then
    return nil, raw
  end
  local ok, decoded = pcall(DecodeBase64, raw)
  if ok and decoded and decoded:match("^https?://") then
    return decoded, raw
  end
  return raw, raw
end

-- Dashboard authentication middleware
Middleware.register("dashboard_auth", function(ctx, next)
  local token = GetCookie("sdb_token")
  Log(kLogInfo, "[dashboard_auth] sdb_token: " .. tostring(token and #token > 0 and "present (" .. #token .. " chars)" or "MISSING"))

  -- Check for token in query params (fallback for login redirect without cookies)
  if (not token or token == "") and ctx.params then
    local query_token = ctx.params._token
    local query_server = ctx.params._server
    if query_token and query_server then
      Log(kLogInfo, "[dashboard_auth] Found token in query params, setting cookies")
      SetCookie("sdb_token", query_token, { path = "/", http_only = true, same_site = "Lax", secure = true, max_age = 3600 * 24 * 30 })
      SetCookie("sdb_server", query_server, { path = "/", http_only = true, same_site = "Lax", secure = true, max_age = 3600 * 24 * 30 })
      token = query_token
    end
  end

  if not token or token == "" then
    local current_path = GetPath() or "/"
    Log(kLogWarn, "[dashboard_auth] No token, redirecting to login")
    return ctx:redirect("/dashboard/login?redirect=" .. current_path)
  end

  -- Validate token by calling the API
  local server_url, raw_server = read_sdb_server_cookie()
  Log(kLogInfo, "[dashboard_auth] sdb_server raw=" .. tostring(raw_server) .. " decoded=" .. tostring(server_url))
  server_url = server_url or "http://localhost:6745"
  Log(kLogInfo, "[dashboard_auth] Validating token against: " .. tostring(server_url))

  local ok, status, headers, body = pcall(Fetch, server_url .. "/_api/auth/me", {
    method = "GET",
    headers = {
      ["Authorization"] = "Bearer " .. token,
      ["Content-Type"] = "application/json"
    }
  })

  Log(kLogInfo, "[dashboard_auth] pcall ok=" .. tostring(ok) .. ", status=" .. tostring(status))
  if not ok then
    Log(kLogError, "[dashboard_auth] Fetch error: " .. tostring(status))
    local current_path = GetPath() or "/"
    return ctx:redirect("/dashboard/login?redirect=" .. current_path)
  end

  if status ~= 200 then
    Log(kLogWarn, "[dashboard_auth] Token validation failed, status=" .. tostring(status) .. ", body=" .. tostring(body))
    local current_path = GetPath() or "/"
    return ctx:redirect("/dashboard/login?redirect=" .. current_path)
  end

  Log(kLogInfo, "[dashboard_auth] Token valid")
  next()
end)

-- Dashboard admin authentication middleware (for _system database routes and admin apps)
Middleware.register("dashboard_admin_auth", function(ctx, next)
  local current_path = GetPath() or "/"
  local token = GetCookie("sdb_token")
  Log(kLogInfo, "[dashboard_admin_auth] sdb_token: " .. tostring(token and #token > 0 and "present (" .. #token .. " chars)" or "MISSING"))
  if not token or token == "" then
    return ctx:redirect("/dashboard/login?redirect=" .. current_path)
  end

  -- Verify admin role by checking with the API
  local server_url, raw_server = read_sdb_server_cookie()
  Log(kLogInfo, "[dashboard_admin_auth] sdb_server raw=" .. tostring(raw_server) .. " decoded=" .. tostring(server_url))
  server_url = server_url or "http://localhost:6745"
  Log(kLogInfo, "[dashboard_admin_auth] Validating against: " .. tostring(server_url))
  local status, headers, body = Fetch(server_url .. "/_api/auth/me", {
    headers = {
      ["Authorization"] = "Bearer " .. token,
      ["Content-Type"] = "application/json"
    }
  })

  Log(kLogInfo, "[dashboard_admin_auth] Validation status=" .. tostring(status))
  if status ~= 200 then
    return ctx:redirect("/dashboard/login?redirect=" .. current_path)
  end

  local ok, user_data = pcall(DecodeJson, body)
  if not ok or not user_data then
    return ctx:redirect("/dashboard/login?redirect=" .. current_path)
  end

  -- Check if user has admin role
  local roles = user_data.roles or {}
  local is_admin = false
  if #roles == 0 then roles = { "admin" } end

  for _, role in ipairs(roles) do
    if role == "admin" or role == "root" then
      is_admin = true
      break
    end
  end

  if not is_admin then
    return ctx:halt(403, "Access denied. Admin privileges required.")
  end

  next()
end)

return Middleware
