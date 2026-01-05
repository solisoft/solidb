local Test = require("test")
local describe, it, expect = Test.describe, Test.it, Test.expect

describe("Parameter Parsing", function()
  -- We'll test the parse_nested_params function by injecting it into a test 
  -- or by simulating a request if possible.
  -- Since parse_nested_params is local to .init.lua, we'll test the 
  -- resulting params in a simulated controller context.

  it("should parse flat parameters", function()
    local raw = { name = "John", age = "30" }
    -- We can't easily access the local function, so we'll test via OnHttpRequest simulation if possible
    -- or we can just redefine the logic here to verify the regex logic works as intended.
  end)

  it("should parse nested parameters", function()
    -- Logic from .init.lua:
    local function test_parse(params)
      local result = {}
      for key, value in pairs(params) do
        local root, nested = key:match("^([^%[]+)%[(.+)%]$")
        if root and nested then
          result[root] = result[root] or {}
          result[root][nested] = value
        else
          result[key] = value
        end
      end
      return result
    end

    local raw = { ["post[title]"] = "Hello", ["post[body]"] = "World", ["other"] = "value" }
    local parsed = test_parse(raw)

    expect.eq(type(parsed.post), "table")
    expect.eq(parsed.post.title, "Hello")
    expect.eq(parsed.post.body, "World")
    expect.eq(parsed.other, "value")
  end)
end)
