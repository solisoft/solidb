-- Session management for Luaonbeans
-- Provides signed cookie-based sessions and flash messages

local Session = {}

-- Configuration
Session.COOKIE_NAME = "luaonbeans_session"
Session.FLASH_COOKIE_NAME = "luaonbeans_flash"
Session.DEFAULT_TTL = 60 * 24 * 7 -- 1 week in minutes

-- Get secret key with validation
local function get_secret_key()
  local key = _G.ENV["SECRET_KEY"]
  if not key or key == "" then
    error("SECRET_KEY environment variable is required for sessions")
  end
  return key
end

-- Timing-safe string comparison to prevent timing attacks
local function secure_compare(a, b)
  if type(a) ~= "string" or type(b) ~= "string" then
    return false
  end
  if #a ~= #b then
    return false
  end
  local result = 0
  for i = 1, #a do
    result = result + (string.byte(a, i) ~ string.byte(b, i))
  end
  return result == 0
end

-- Check if running in production
local function is_production()
  local env = os.getenv("BEANS_ENV") or "development"
  return env == "production"
end

---Create a signed session object
---@param data table JSON serializable lua object
---@param ttl number? TTL in minutes. Default to 60 minutes.
---@return nil
function SetSession(data, ttl)
  ttl = ttl or Session.DEFAULT_TTL
  if(type(ttl) ~= "number") then ttl = Session.DEFAULT_TTL end

  local secret = get_secret_key()

  local json_data = EncodeJson(data) or "{}"
  local msg = EncodeBase64(json_data)
  local sig = EncodeBase64(GetCryptoHash("SHA256", msg, secret))

  SetCookie(Session.COOKIE_NAME, msg .. "." .. sig, {
    HttpOnly = true,
    MaxAge = 60 * ttl,
    SameSite = "Strict",
    Secure = is_production(),
    Path = "/"
  })
end

---Get Session object
---@return table object deserialized session data, empty table if invalid/missing
function GetSession()
  local session = GetCookie(Session.COOKIE_NAME)
  if not session or session == "" then
    return {}
  end

  -- Split into message and signature
  local dot_pos = session:find(".", 1, true)
  if not dot_pos then
    return {}
  end

  local msg = session:sub(1, dot_pos - 1)
  local sig = session:sub(dot_pos + 1)

  if msg == "" or sig == "" then
    return {}
  end

  -- Verify signature
  local secret = get_secret_key()
  local expected_sig = EncodeBase64(GetCryptoHash("SHA256", msg, secret))

  if not secure_compare(expected_sig, sig) then
    -- Invalid signature - clear the cookie and return empty
    DestroySession()
    return {}
  end

  -- Decode session data
  local ok, session_data = pcall(function()
    return DecodeJson(DecodeBase64(msg))
  end)

  if not ok or type(session_data) ~= "table" then
    return {}
  end

  return session_data
end

---Destroy the current session
---@return nil
function DestroySession()
  SetCookie(Session.COOKIE_NAME, "", {
    HttpOnly = true,
    MaxAge = -1,
    SameSite = "Strict",
    Secure = is_production(),
    Path = "/"
  })
end

---Check if a session exists and is valid
---@return boolean
function HasSession()
  local session = GetSession()
  return next(session) ~= nil
end

-- Flash message storage (per-request)
local _flash_data = {}

---Set Flash Session object
---Usually used for flash messages (success, error, notice, etc.)
---@param flash_type string The type of flash (e.g., "success", "error", "notice")
---@param message string The flash message
---@return nil
function SetFlash(flash_type, message)
  _flash_data[flash_type] = message

  SetCookie(Session.FLASH_COOKIE_NAME, EncodeBase64(EncodeJson(_flash_data)), {
    HttpOnly = true,
    MaxAge = 60, -- 1 minute should be enough for redirect
    SameSite = "Strict",
    Secure = is_production(),
    Path = "/"
  })
end

---Clear Flash Cookie
---@return nil
local function DeleteFlash()
  SetCookie(Session.FLASH_COOKIE_NAME, "", {
    HttpOnly = true,
    MaxAge = -1,
    Path = "/"
  })
  _flash_data = {}
end

---Get Flash Session object and clear it
---@return table flash data, empty table if no flash
function GetFlash()
  local flash_cookie = GetCookie(Session.FLASH_COOKIE_NAME)
  if not flash_cookie or flash_cookie == "" then
    return {}
  end

  local ok, flash_session = pcall(function()
    return DecodeJson(DecodeBase64(flash_cookie))
  end)

  DeleteFlash()

  if not ok or type(flash_session) ~= "table" then
    return {}
  end

  return flash_session
end

---Get a specific flash message
---@param flash_type string The type of flash to get
---@return string|nil The flash message or nil
function GetFlashMessage(flash_type)
  local flash = GetFlash()
  return flash[flash_type]
end

return Session
