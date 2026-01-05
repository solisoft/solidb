-- Tests for middleware module
-- test/middleware/middleware_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local describe, it, expect = Test.describe, Test.it, Test.expect

-- Mock redbean functions
_G.GetHeader = _G.GetHeader or function(name) return nil end
_G.SetHeader = _G.SetHeader or function(name, value) end
_G.SetStatus = _G.SetStatus or function(status) end
_G.Write = _G.Write or function(body) end
_G.EncodeJson = _G.EncodeJson or function(data)
  if type(data) == "table" then return "{}" end
  return tostring(data)
end

-- Load middleware module fresh for each test run
package.loaded["middleware"] = nil
local Middleware = require("middleware")

-- ============================================================================
-- Context Tests
-- ============================================================================

describe("Middleware Context", function()
  it("should create context with method and path", function()
    local ctx = Middleware.create_context("GET", "/users")
    expect.eq(ctx.method, "GET")
    expect.eq(ctx.path, "/users")
  end)

  it("should initialize empty params and headers", function()
    local ctx = Middleware.create_context("GET", "/")
    expect.eq(type(ctx.params), "table")
    expect.eq(type(ctx.headers), "table")
  end)

  it("should have halted as false by default", function()
    local ctx = Middleware.create_context("GET", "/")
    expect.eq(ctx.halted, false)
  end)

  it("should have empty data table for middleware sharing", function()
    local ctx = Middleware.create_context("GET", "/")
    expect.eq(type(ctx.data), "table")
  end)
end)

describe("Context halt", function()
  it("should set halted to true", function()
    local ctx = Middleware.create_context("GET", "/")
    ctx:halt(403, "Forbidden")
    expect.truthy(ctx.halted)
  end)

  it("should set status and body", function()
    local ctx = Middleware.create_context("GET", "/")
    ctx:halt(403, "Forbidden")
    expect.eq(ctx.status, 403)
    expect.eq(ctx.response_body, "Forbidden")
  end)

  it("should default to 200 status", function()
    local ctx = Middleware.create_context("GET", "/")
    ctx:halt()
    expect.eq(ctx.status, 200)
  end)
end)

describe("Context redirect", function()
  it("should set halted to true", function()
    local ctx = Middleware.create_context("GET", "/")
    ctx:redirect("/login")
    expect.truthy(ctx.halted)
  end)

  it("should set Location header", function()
    local ctx = Middleware.create_context("GET", "/")
    ctx:redirect("/login")
    expect.eq(ctx.response_headers["Location"], "/login")
  end)

  it("should default to 302 status", function()
    local ctx = Middleware.create_context("GET", "/")
    ctx:redirect("/login")
    expect.eq(ctx.status, 302)
  end)

  it("should accept custom status", function()
    local ctx = Middleware.create_context("GET", "/")
    ctx:redirect("/login", 301)
    expect.eq(ctx.status, 301)
  end)
end)

describe("Context set_header", function()
  it("should add header to response_headers", function()
    local ctx = Middleware.create_context("GET", "/")
    ctx:set_header("X-Custom", "value")
    expect.eq(ctx.response_headers["X-Custom"], "value")
  end)
end)

-- ============================================================================
-- Middleware Registration Tests
-- ============================================================================

describe("Middleware Registration", function()
  it("should register named middleware", function()
    Middleware.clear()
    local called = false
    Middleware.register("test", function(ctx, next)
      called = true
      next()
    end)

    local fn = Middleware.resolve("test")
    expect.truthy(fn)
  end)

  it("should resolve function as-is", function()
    local fn = function(ctx, next) next() end
    local resolved = Middleware.resolve(fn)
    expect.eq(resolved, fn)
  end)

  it("should return nil for unknown middleware", function()
    Middleware.clear()
    local fn = Middleware.resolve("nonexistent")
    expect.nil_value(fn)
  end)
end)

-- ============================================================================
-- Middleware Chain Tests
-- ============================================================================

describe("Middleware Chain", function()
  it("should run global before middleware", function()
    Middleware.clear()
    local called = false

    Middleware.use(function(ctx, next)
      called = true
      next()
    end)

    local ctx = Middleware.create_context("GET", "/")
    Middleware.run_before(ctx)

    expect.truthy(called)
  end)

  it("should run middleware in order", function()
    Middleware.clear()
    local order = {}

    Middleware.use(function(ctx, next)
      table.insert(order, 1)
      next()
    end)

    Middleware.use(function(ctx, next)
      table.insert(order, 2)
      next()
    end)

    local ctx = Middleware.create_context("GET", "/")
    Middleware.run_before(ctx)

    expect.eq(order[1], 1)
    expect.eq(order[2], 2)
  end)

  it("should stop chain when halted", function()
    Middleware.clear()
    local second_called = false

    Middleware.use(function(ctx, next)
      ctx:halt(401, "Unauthorized")
      -- Don't call next()
    end)

    Middleware.use(function(ctx, next)
      second_called = true
      next()
    end)

    local ctx = Middleware.create_context("GET", "/")
    local should_continue = Middleware.run_before(ctx)

    expect.falsy(should_continue)
    expect.falsy(second_called)
  end)

  it("should return true when not halted", function()
    Middleware.clear()

    Middleware.use(function(ctx, next)
      next()
    end)

    local ctx = Middleware.create_context("GET", "/")
    local should_continue = Middleware.run_before(ctx)

    expect.truthy(should_continue)
  end)
end)

describe("Route Middleware", function()
  it("should run route-specific middleware", function()
    Middleware.clear()
    local called = false

    Middleware.register("auth", function(ctx, next)
      called = true
      next()
    end)

    local ctx = Middleware.create_context("GET", "/admin")
    Middleware.run_route(ctx, {"auth"})

    expect.truthy(called)
  end)

  it("should run multiple route middleware in order", function()
    Middleware.clear()
    local order = {}

    Middleware.register("first", function(ctx, next)
      table.insert(order, "first")
      next()
    end)

    Middleware.register("second", function(ctx, next)
      table.insert(order, "second")
      next()
    end)

    local ctx = Middleware.create_context("GET", "/")
    Middleware.run_route(ctx, {"first", "second"})

    expect.eq(order[1], "first")
    expect.eq(order[2], "second")
  end)

  it("should return true for empty middleware list", function()
    Middleware.clear()
    local ctx = Middleware.create_context("GET", "/")
    local result = Middleware.run_route(ctx, {})
    expect.truthy(result)
  end)

  it("should return true for nil middleware list", function()
    Middleware.clear()
    local ctx = Middleware.create_context("GET", "/")
    local result = Middleware.run_route(ctx, nil)
    expect.truthy(result)
  end)
end)

describe("After Middleware", function()
  it("should run after middleware in reverse order", function()
    Middleware.clear()
    local order = {}

    Middleware.after(function(ctx, next)
      table.insert(order, 1)
      next()
    end)

    Middleware.after(function(ctx, next)
      table.insert(order, 2)
      next()
    end)

    local ctx = Middleware.create_context("GET", "/")
    Middleware.run_after(ctx)

    -- Should be reversed: 2, then 1
    expect.eq(order[1], 2)
    expect.eq(order[2], 1)
  end)
end)

-- ============================================================================
-- Middleware Data Sharing Tests
-- ============================================================================

describe("Middleware Data Sharing", function()
  it("should share data between middleware via ctx.data", function()
    Middleware.clear()

    Middleware.use(function(ctx, next)
      ctx.data.user_id = 123
      next()
    end)

    Middleware.use(function(ctx, next)
      ctx.data.user_id = ctx.data.user_id + 1
      next()
    end)

    local ctx = Middleware.create_context("GET", "/")
    Middleware.run_before(ctx)

    expect.eq(ctx.data.user_id, 124)
  end)
end)

-- ============================================================================
-- Stats and List Tests
-- ============================================================================

describe("Middleware Stats", function()
  it("should return correct counts", function()
    Middleware.clear()

    Middleware.use(function(ctx, next) next() end)
    Middleware.use(function(ctx, next) next() end)
    Middleware.after(function(ctx, next) next() end)
    Middleware.register("auth", function(ctx, next) next() end)

    local stats = Middleware.stats()
    expect.eq(stats.before, 2)
    expect.eq(stats.after, 1)
    expect.eq(stats.named, 1)
  end)
end)

describe("Middleware List", function()
  it("should list named middleware", function()
    Middleware.clear()

    Middleware.register("auth", function(ctx, next) next() end)
    Middleware.register("cors", function(ctx, next) next() end)

    local list = Middleware.list()
    expect.eq(#list.named, 2)
  end)
end)

-- ============================================================================
-- Clear Tests
-- ============================================================================

describe("Middleware Clear", function()
  it("should clear all middleware", function()
    Middleware.use(function(ctx, next) next() end)
    Middleware.after(function(ctx, next) next() end)
    Middleware.register("test", function(ctx, next) next() end)

    Middleware.clear()

    local stats = Middleware.stats()
    expect.eq(stats.before, 0)
    expect.eq(stats.after, 0)
    expect.eq(stats.named, 0)
  end)
end)

return Test
