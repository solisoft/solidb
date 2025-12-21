---Create a signed session object
---@param data JSON serializable lua object
---@param ttl number? TTL in minutes. Default to 60minutes.
---@return nil
SetSession = function(data, ttl)
  ttl = ttl or 60
  local msg = EncodeBase64(EncodeJson(data) or "")
  local sig = EncodeBase64(GetCryptoHash("SHA256", msg, ENV["SECRET_KEY"]))

  SetCookie("luaonbeans_session", "%s.%s" % {msg, sig}, {
    HttpOnly = true,
    MaxAge = 60 * ttl,
    SameSite = "Strict",
    Secure = BeansEnv == "production",
    Path = "/"
  })
end

---Get Session object
---@return table object deserialized
GetSession = function()
  local session  = GetCookie("luaonbeans_session")
  local data = string.split(session or "", ".")

  if #data ~= 2 then return {} end

  assert(EncodeBase64(GetCryptoHash("SHA256", data[1], ENV["SECRET_KEY"])) == data[2], "Session : Invalid signature")

  local session_data = DecodeJson(DecodeBase64(data[1]))
  SetSession(session_data)

  return session_data
end

---Set Flash Session object
---Usually used for flash message (aka Rails flash methods)
---@param string type
---@param string str
---@return nil
SetFlash = function(type, str)
  Flash[type] = str
  SetCookie("luaonbeans_flash", EncodeBase64(EncodeJson(Flash)), {
    HttpOnly = true,
    MaxAge = 1,
    SameSite = "Strict",
    Secure = BeansEnv == "production",
    Path = "/"
  })
end

---Clear Flash Cookie
---@return nil
local DeleteFlash = function()
  SetCookie("luaonbeans_flash", "", { MaxAge = -1 })
end

---Get Flash Session object
---@return table data empty string if cookie was not set
GetFlash = function()
  local flash_data = GetCookie("luaonbeans_flash")
  if flash_data == nil then return {} end

  local flash_session = DecodeJson(DecodeBase64(flash_data))
  DeleteFlash()
  return flash_session
end
