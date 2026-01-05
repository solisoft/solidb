-- Tests for router middleware integration
-- test/middleware/router_middleware_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local describe, it, expect = Test.describe, Test.it, Test.expect

-- Load router fresh
package.loaded["router"] = nil
local Router = require("router")

-- ============================================================================
-- Route Middleware Tests
-- ============================================================================

describe("Router Middleware Options", function()
  it("should accept middleware option on get", function()
    Router.clear()
    Router.get("/admin", "admin#index", { middleware = { "auth" } })

    local route, params = Router.match("GET", "/admin")
    expect.truthy(route)
    expect.truthy(route.middleware)
    expect.eq(route.middleware[1], "auth")
  end)

  it("should accept middleware option on post", function()
    Router.clear()
    Router.post("/admin", "admin#create", { middleware = { "auth", "csrf" } })

    local route, params = Router.match("POST", "/admin")
    expect.truthy(route)
    expect.eq(#route.middleware, 2)
  end)

  it("should accept middleware option on put", function()
    Router.clear()
    Router.put("/admin/:id", "admin#update", { middleware = { "auth" } })

    local route, params = Router.match("PUT", "/admin/123")
    expect.truthy(route)
    expect.truthy(route.middleware)
  end)

  it("should accept middleware option on patch", function()
    Router.clear()
    Router.patch("/admin/:id", "admin#update", { middleware = { "auth" } })

    local route, params = Router.match("PATCH", "/admin/123")
    expect.truthy(route)
    expect.truthy(route.middleware)
  end)

  it("should accept middleware option on delete", function()
    Router.clear()
    Router.delete("/admin/:id", "admin#destroy", { middleware = { "auth" } })

    local route, params = Router.match("DELETE", "/admin/123")
    expect.truthy(route)
    expect.truthy(route.middleware)
  end)

  it("should have nil middleware when no option provided", function()
    Router.clear()
    Router.get("/public", "public#index")

    local route, params = Router.match("GET", "/public")
    expect.truthy(route)
    expect.nil_value(route.middleware)
  end)
end)

-- ============================================================================
-- Scope Middleware Tests
-- ============================================================================

describe("Router Scope Middleware", function()
  it("should inherit middleware from scope", function()
    Router.clear()
    Router.scope("/admin", { middleware = { "auth" } }, function()
      Router.get("/dashboard", "admin#dashboard")
    end)

    local route, params = Router.match("GET", "/admin/dashboard")
    expect.truthy(route)
    expect.truthy(route.middleware)
    expect.eq(route.middleware[1], "auth")
  end)

  it("should combine scope and route middleware", function()
    Router.clear()
    Router.scope("/admin", { middleware = { "auth" } }, function()
      Router.get("/users", "admin#users", { middleware = { "admin_only" } })
    end)

    local route, params = Router.match("GET", "/admin/users")
    expect.truthy(route)
    expect.eq(#route.middleware, 2)
    expect.eq(route.middleware[1], "auth")
    expect.eq(route.middleware[2], "admin_only")
  end)

  it("should not leak scope middleware outside scope", function()
    Router.clear()
    Router.scope("/admin", { middleware = { "auth" } }, function()
      Router.get("/dashboard", "admin#dashboard")
    end)
    Router.get("/public", "public#index")

    local admin_route = Router.match("GET", "/admin/dashboard")
    local public_route = Router.match("GET", "/public")

    expect.truthy(admin_route.middleware)
    expect.nil_value(public_route.middleware)
  end)

  it("should handle nested scopes with middleware", function()
    Router.clear()
    Router.scope("/api", { middleware = { "api_auth" } }, function()
      Router.scope("/v1", { middleware = { "rate_limit" } }, function()
        Router.get("/users", "api/v1/users#index")
      end)
    end)

    local route, params = Router.match("GET", "/api/v1/users")
    expect.truthy(route)
    expect.eq(#route.middleware, 2)
    expect.eq(route.middleware[1], "api_auth")
    expect.eq(route.middleware[2], "rate_limit")
  end)

  it("should work with scope without middleware", function()
    Router.clear()
    Router.scope("/api", function()
      Router.get("/status", "api#status")
    end)

    local route, params = Router.match("GET", "/api/status")
    expect.truthy(route)
    expect.nil_value(route.middleware)
  end)
end)

-- ============================================================================
-- Dispatch Middleware Tests
-- ============================================================================

describe("Router Dispatch with Middleware", function()
  it("should include middleware in dispatch result", function()
    Router.clear()
    Router.get("/admin", "admin#index", { middleware = { "auth" } })

    local matched, result = Router.dispatch("GET", "/admin")
    expect.truthy(matched)
    expect.truthy(result.middleware)
    expect.eq(result.middleware[1], "auth")
  end)

  it("should include middleware in function handler dispatch", function()
    Router.clear()
    Router.get("/test", function(params) return { status = 200 } end, { middleware = { "test" } })

    local matched, result = Router.dispatch("GET", "/test")
    expect.truthy(matched)
    expect.truthy(result.fn)
    expect.truthy(result.middleware)
    expect.eq(result.middleware[1], "test")
  end)

  it("should have nil middleware in dispatch when not specified", function()
    Router.clear()
    Router.get("/public", "public#index")

    local matched, result = Router.dispatch("GET", "/public")
    expect.truthy(matched)
    expect.nil_value(result.middleware)
  end)
end)

return Test
