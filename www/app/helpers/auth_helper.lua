local function get_safe_db()
  return _G.Sdb
end

local AuthHelper = {}

-- Request-level cache to avoid multiple DB hits per request
local _request_cache = {}

-- Clear cache (can be called at start of request if needed, but Lua state usually per request in Redbean)
function AuthHelper.clear_cache()
  _request_cache = {}
end

function AuthHelper.get_current_user()
  -- Check cache first
  if _request_cache.current_user then
      return _request_cache.current_user
  end

  local session = GetSession()
  if not session.user_id then return nil end

  local db = get_safe_db()
  if not db then return nil end

  -- Optimization: Could cache in session too, but simple DB lookup by ID is fast
  local res = db:Sdbql("FOR u IN users FILTER u._id == @id RETURN u", { id = session.user_id })

  if res and res.result and #res.result > 0 then
    local user = res.result[1]

    -- Auto-promote first user to admin if not already set
    if user.is_admin == nil then
      local count_res = db:Sdbql("RETURN LENGTH(users)")
      local user_count = count_res and count_res.result and count_res.result[1] or 0
      if user_count == 1 then
        db:Sdbql("UPDATE @key WITH { is_admin: true } IN users", { key = user._key })
        user.is_admin = true
      end
    end

    _request_cache.current_user = user
    return user
  end

  return nil
end

function AuthHelper.require_login(controller, redirect_path)
  local user = AuthHelper.get_current_user()
  if not user then
    if controller.params._json then
      controller:json({ error = "Unauthorized" }, 401)
    else
      local current_path = redirect_path or GetPath() or "/talks"
      controller:redirect("/auth/login?redirect=" .. current_path)
    end
    return nil
  end
  return user
end

return AuthHelper
