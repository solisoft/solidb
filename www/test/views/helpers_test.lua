-- Example tests for the helpers module
-- test/helpers_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local describe, it, expect = Test.describe, Test.it, Test.expect

-- Mock redbean globals for testing
_G.EscapeHtml = function(str)
  return str:gsub("&", "&amp;"):gsub("<", "&lt;"):gsub(">", "&gt;"):gsub('"', "&quot;")
end

local helpers = require("helpers")

describe("Helpers", function()
  
  describe("link_to", function()
    it("should generate a basic link", function()
      local result = helpers.link_to("Home", "/")
      expect.matches(result, '<a href="/">')
      expect.matches(result, ">Home</a>")
    end)
    
    it("should include custom attributes", function()
      local result = helpers.link_to("Click", "/path", { class = "btn" })
      expect.matches(result, 'class="btn"')
    end)
  end)
  
  describe("input_tag", function()
    it("should generate an input field", function()
      local result = helpers.input_tag("email", "test@example.com")
      expect.matches(result, 'name="email"')
      expect.matches(result, 'value="test@example.com"')
    end)
    
    it("should support custom type", function()
      local result = helpers.input_tag("password", "", { type = "password" })
      expect.matches(result, 'type="password"')
    end)
  end)
  
  describe("truncate", function()
    it("should truncate long strings", function()
      local result = helpers.truncate("This is a very long string", 10)
      expect.eq(result, "This is a ...")
    end)
    
    it("should not truncate short strings", function()
      local result = helpers.truncate("Short", 100)
      expect.eq(result, "Short")
    end)
  end)
  
end)

-- Run tests if executed directly
if arg and arg[0] and arg[0]:match("helpers_test.lua$") then
  return Test.run()
end

return Test
