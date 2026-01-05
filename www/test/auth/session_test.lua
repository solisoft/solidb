-- Tests for session module
-- test/auth/session_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local describe, it, expect = Test.describe, Test.it, Test.expect

-- Mock state - use _G so it persists when tests run
_G._test_mock_cookies = _G._test_mock_cookies or {}
_G._test_mock_env = _G._test_mock_env or {
  SECRET_KEY = "test-secret-key-12345",
  BEANS_ENV = "development"
}
_G._test_set_cookie_calls = _G._test_set_cookie_calls or {}

-- Mock os.getenv BEFORE loading session module
os.getenv = function(key)
  return _G._test_mock_env[key]
end

-- Mock redbean functions BEFORE loading session module
_G.GetCookie = function(name)
  return _G._test_mock_cookies[name]
end

_G.SetCookie = function(name, value, options)
  _G._test_mock_cookies[name] = value
  table.insert(_G._test_set_cookie_calls, { name = name, value = value, options = options })
end

_G.EncodeJson = _G.EncodeJson or function(data)
  if type(data) == "nil" then return "null" end
  if type(data) == "boolean" then return tostring(data) end
  if type(data) == "number" then return tostring(data) end
  if type(data) == "string" then return '"' .. data:gsub('"', '\\"') .. '"' end
  if type(data) == "table" then
    local parts = {}
    local is_array = #data > 0
    for k, v in pairs(data) do
      if is_array then
        table.insert(parts, _G.EncodeJson(v))
      else
        table.insert(parts, '"' .. tostring(k) .. '":' .. _G.EncodeJson(v))
      end
    end
    return is_array and "[" .. table.concat(parts, ",") .. "]" or "{" .. table.concat(parts, ",") .. "}"
  end
  return "null"
end

_G.DecodeJson = _G.DecodeJson or function(str)
  if str == "null" then return nil end
  if str == "true" then return true end
  if str == "false" then return false end
  if str:match("^%d+$") then return tonumber(str) end
  if str:match("^\"(.*)\"$") then return str:match("^\"(.*)\"$") end
  if str:match("^{") then
    local result = {}
    for key, value in str:gmatch('"([^"]+)":"?([^",}]+)"?') do
      if value == "true" then value = true
      elseif value == "false" then value = false
      elseif tonumber(value) then value = tonumber(value)
      end
      result[key] = value
    end
    return result
  end
  return {}
end

_G.EncodeBase64 = _G.EncodeBase64 or function(str)
  local b64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
  return ((str:gsub('.', function(x)
    local r, b = '', x:byte()
    for i = 8, 1, -1 do r = r .. (b % 2 ^ i - b % 2 ^ (i - 1) > 0 and '1' or '0') end
    return r
  end) .. '0000'):gsub('%d%d%d?%d?%d?%d?', function(x)
    if #x < 6 then return '' end
    local c = 0
    for i = 1, 6 do c = c + (x:sub(i, i) == '1' and 2 ^ (6 - i) or 0) end
    return b64:sub(c + 1, c + 1)
  end) .. ({ '', '==', '=' })[#str % 3 + 1])
end

_G.DecodeBase64 = _G.DecodeBase64 or function(str)
  local b64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
  str = str:gsub('[^' .. b64 .. '=]', '')
  return (str:gsub('.', function(x)
    if x == '=' then return '' end
    local r, f = '', (b64:find(x) - 1)
    for i = 6, 1, -1 do r = r .. (f % 2 ^ i - f % 2 ^ (i - 1) > 0 and '1' or '0') end
    return r
  end):gsub('%d%d%d?%d?%d?%d?%d?%d?', function(x)
    if #x ~= 8 then return '' end
    local c = 0
    for i = 1, 8 do c = c + (x:sub(i, i) == '1' and 2 ^ (8 - i) or 0) end
    return string.char(c)
  end))
end

_G.GetCryptoHash = _G.GetCryptoHash or function(algorithm, message, key)
  local hash = {}
  local size = algorithm == "SHA256" and 32 or 20
  for i = 1, size do
    local byte = (message:byte((i % #message) + 1) or 0) +
                 (key:byte((i % #key) + 1) or 0) + i
    hash[i] = string.char(byte % 256)
  end
  return table.concat(hash)
end

-- Helper to reset mock state
local function reset_mocks()
  for k in pairs(_G._test_mock_cookies) do _G._test_mock_cookies[k] = nil end
  for i = #_G._test_set_cookie_calls, 1, -1 do _G._test_set_cookie_calls[i] = nil end
  _G._test_mock_env.SECRET_KEY = "test-secret-key-12345"
  _G._test_mock_env.BEANS_ENV = "development"
end

-- Load session module AFTER mocks are set up
dofile(".lua/session.lua")

-- Each test resets mocks at the start to avoid interference
describe("Session SetSession", function()
  it("should set a signed session cookie", function()
    reset_mocks()
    SetSession({ user_id = 123 })

    local cookie = _G._test_mock_cookies["luaonbeans_session"]
    expect.truthy(cookie)
    expect.truthy(cookie:match("%.")) -- Has signature separator
  end)

  it("should set HttpOnly flag", function()
    reset_mocks()
    SetSession({ user_id = 123 })

    local call = _G._test_set_cookie_calls[#_G._test_set_cookie_calls]
    expect.truthy(call.options.HttpOnly)
  end)

  it("should set SameSite=Strict", function()
    reset_mocks()
    SetSession({ user_id = 123 })

    local call = _G._test_set_cookie_calls[#_G._test_set_cookie_calls]
    expect.eq(call.options.SameSite, "Strict")
  end)

  it("should set Secure flag in production", function()
    reset_mocks()
    _G._test_mock_env.BEANS_ENV = "production"
    SetSession({ user_id = 123 })

    local call = _G._test_set_cookie_calls[#_G._test_set_cookie_calls]
    expect.truthy(call.options.Secure)
  end)

  it("should not set Secure flag in development", function()
    reset_mocks()
    _G._test_mock_env.BEANS_ENV = "development"
    SetSession({ user_id = 123 })

    local call = _G._test_set_cookie_calls[#_G._test_set_cookie_calls]
    expect.falsy(call.options.Secure)
  end)

  it("should use default TTL of 60 minutes", function()
    reset_mocks()
    SetSession({ user_id = 123 })

    local call = _G._test_set_cookie_calls[#_G._test_set_cookie_calls]
    expect.eq(call.options.MaxAge, 60 * 60)
  end)

  it("should accept custom TTL", function()
    reset_mocks()
    SetSession({ user_id = 123 }, 120)

    local call = _G._test_set_cookie_calls[#_G._test_set_cookie_calls]
    expect.eq(call.options.MaxAge, 60 * 120)
  end)
end)

describe("Session GetSession", function()
  it("should return empty table when no session exists", function()
    reset_mocks()
    _G._test_mock_cookies["luaonbeans_session"] = nil

    local session = GetSession()
    expect.eq(type(session), "table")
    expect.nil_value(next(session))
  end)

  it("should return session data when valid", function()
    reset_mocks()
    SetSession({ user_id = 123, role = "admin" })

    local session = GetSession()
    expect.eq(session.user_id, 123)
    expect.eq(session.role, "admin")
  end)

  it("should return empty table for tampered signature", function()
    reset_mocks()
    SetSession({ user_id = 123 })

    local cookie = _G._test_mock_cookies["luaonbeans_session"]
    _G._test_mock_cookies["luaonbeans_session"] = cookie .. "tampered"

    local session = GetSession()
    expect.nil_value(next(session))
  end)

  it("should return empty table for invalid format", function()
    reset_mocks()
    _G._test_mock_cookies["luaonbeans_session"] = "invalid-no-dot"

    local session = GetSession()
    expect.nil_value(next(session))
  end)
end)

describe("Session DestroySession", function()
  it("should clear the session cookie", function()
    reset_mocks()
    SetSession({ user_id = 123 })
    DestroySession()

    local call = _G._test_set_cookie_calls[#_G._test_set_cookie_calls]
    expect.eq(call.options.MaxAge, -1)
  end)
end)

describe("Session HasSession", function()
  it("should return true when session exists", function()
    reset_mocks()
    SetSession({ user_id = 123 })

    local result = HasSession()
    expect.truthy(result)
  end)

  it("should return false when no session", function()
    reset_mocks()
    _G._test_mock_cookies["luaonbeans_session"] = nil

    local result = HasSession()
    expect.falsy(result)
  end)
end)

describe("Session Flash", function()
  it("should set a flash cookie", function()
    reset_mocks()
    SetFlash("success", "Item saved!")

    local cookie = _G._test_mock_cookies["luaonbeans_flash"]
    expect.truthy(cookie)
  end)

  it("should return flash data", function()
    reset_mocks()
    SetFlash("notice", "Please log in")

    local flash = GetFlash()
    expect.eq(flash.notice, "Please log in")
  end)

  it("should return specific flash type", function()
    reset_mocks()
    SetFlash("success", "Done!")

    local message = GetFlashMessage("success")
    expect.eq(message, "Done!")
  end)
end)

describe("Session Security", function()
  it("should error when SECRET_KEY is missing", function()
    reset_mocks()
    _G._test_mock_env.SECRET_KEY = nil

    expect.error(function()
      SetSession({ user_id = 123 })
    end)
  end)

  it("should error when SECRET_KEY is empty", function()
    reset_mocks()
    _G._test_mock_env.SECRET_KEY = ""

    expect.error(function()
      SetSession({ user_id = 123 })
    end)
  end)
end)

return Test
