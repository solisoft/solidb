-- Example tests for the router module
-- test/router_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local describe, it, expect = Test.describe, Test.it, Test.expect

-- Load the router
local router = require("router")

describe("Router", function()
  
  it("should register GET routes", function()
    router.clear()
    router.get("/users", "users#index")
    
    local matched, result = router.dispatch("GET", "/users")
    expect.truthy(matched, "Route should match")
    expect.eq(result.controller, "users")
    expect.eq(result.action, "index")
  end)
  
  it("should register POST routes", function()
    router.clear()
    router.post("/users", "users#create")
    
    local matched, result = router.dispatch("POST", "/users")
    expect.truthy(matched, "Route should match")
    expect.eq(result.controller, "users")
    expect.eq(result.action, "create")
  end)
  
  it("should extract URL parameters", function()
    router.clear()
    router.get("/users/:id", "users#show")
    
    local matched, result = router.dispatch("GET", "/users/123")
    expect.truthy(matched, "Route should match")
    expect.eq(result.params.id, "123")
  end)
  
  it("should extract multiple URL parameters", function()
    router.clear()
    router.get("/posts/:post_id/comments/:id", "comments#show")
    
    local matched, result = router.dispatch("GET", "/posts/42/comments/7")
    expect.truthy(matched, "Route should match")
    expect.eq(result.params.post_id, "42")
    expect.eq(result.params.id, "7")
  end)
  
  it("should not match incorrect methods", function()
    router.clear()
    router.get("/users", "users#index")
    
    local matched = router.dispatch("POST", "/users")
    expect.falsy(matched, "Should not match POST for GET route")
  end)
  
  it("should not match non-existent routes", function()
    router.clear()
    router.get("/users", "users#index")
    
    local matched = router.dispatch("GET", "/posts")
    expect.falsy(matched, "Should not match non-existent route")
  end)
  
end)

-- Run tests if executed directly
if arg and arg[0] and arg[0]:match("router_test.lua$") then
  return Test.run()
end

return Test
