#!/usr/bin/env lua
-- Test runner for Luaonbeans
-- Run with: ./luaonbeans.org -i test/run.lua

-- Configure package path
package.path = ".lua/?.lua;.lua/db/?.lua;app/?.lua;app/controllers/?.lua;app/models/?.lua;config/?.lua;config/locales/?.lua;test/?.lua;" .. package.path

-- Load test framework
local Test = require("test")

-- Load I18n if available (provides real translations)
if not _G.I18n then
  local ok, I18n = pcall(require, "i18n")
  if ok then
    I18n:load_locale("en")
    I18n:make_global()
  end
end
_G.EncodeJson = function(data)
  -- Simple JSON encoder for testing
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
_G.Md5 = function(str)
  -- Simple hash for testing (not real MD5)
  local hash = 0
  for i = 1, #str do
    hash = (hash * 31 + string.byte(str, i)) % 2147483647
  end
  return string.format("%08x", hash)
end

-- Collect test files (organized by category)
local test_files = {
  -- Controllers
  "test/controllers/controllers_test.lua",
  
  -- Models
  "test/models/models_test.lua",
  "test/models/fixtures_test.lua",
  "test/models/mass_assignment_test.lua",
  
  -- Views
  "test/views/helpers_test.lua",
  "test/views/content_for_test.lua",
  "test/views/i18n_test.lua",
  "test/views/variants_test.lua",
  
  -- Router
  "test/router/router_test.lua",
  "test/router/nested_resources_test.lua",
  "test/router/resources_options_test.lua",
  "test/router/resources_scopes_test.lua",
  "test/router/params_parsing_test.lua",
  
  -- Integration
  "test/integration/posts_crud_test.lua",

  -- Migrations
  "test/migrations/migrate_test.lua",

  -- Auth
  "test/auth/totp_test.lua",
  "test/auth/session_test.lua",

  -- Middleware
  "test/middleware/middleware_test.lua",
  "test/middleware/router_middleware_test.lua"
}


-- Run each test file
print("Found " .. #test_files .. " test file(s)")

for _, file in ipairs(test_files) do
  print("\n📁 Loading: " .. file)
  -- Test.clear() -- Don't clear tests, accumulate them

  local ok, err = pcall(function()
    dofile(file)
  end)
  
  if not ok then
    print("❌ Error loading " .. file .. ": " .. tostring(err))
  end
end

-- Run all collected tests
local exit_code = Test.run()
os.exit(exit_code)
