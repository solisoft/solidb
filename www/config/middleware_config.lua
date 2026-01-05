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

-- Dashboard authentication middleware
Middleware.register("dashboard_auth", function(ctx, next)
  local token = GetCookie("sdb_token")
  if not token or token == "" then
    return ctx:redirect("/dashboard/login")
  end
  next()
end)

-- Dashboard admin authentication middleware (for _system database routes)
Middleware.register("dashboard_admin_auth", function(ctx, next)
  local token = GetCookie("sdb_token")
  if not token or token == "" then
    return ctx:redirect("/dashboard/login")
  end

  -- Verify admin role by checking with the API
  local server_url = GetCookie("sdb_server") or "http://localhost:6745"
  local status, headers, body = Fetch(server_url .. "/_api/auth/me", {
    headers = {
      ["Authorization"] = "Bearer " .. token,
      ["Content-Type"] = "application/json"
    }
  })

  if status ~= 200 then
    return ctx:redirect("/dashboard/login")
  end

  local ok, user_data = pcall(DecodeJson, body)
  if not ok or not user_data then
    return ctx:redirect("/dashboard/login")
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
    return ctx:halt(403, EncodeJson(roles))
  end

  next()
end)

return Middleware
