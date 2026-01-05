-- Tests for view variants
-- test/views/variants_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local describe, it, expect, before, after = Test.describe, Test.it, Test.expect, Test.before, Test.after

describe("View Variants", function()
  local View
  local test_views_path = "test/fixtures/views"

  before(function()
    View = require("view")
    View.set_views_path(test_views_path)
    View.clear_cache()

    -- Create test fixture directories and files
    os.execute("mkdir -p " .. test_views_path .. "/posts")
    os.execute("mkdir -p " .. test_views_path .. "/layouts/application")

    -- Create default template
    local f = io.open(test_views_path .. "/posts/show.etlua", "w")
    f:write("<h1>Desktop View</h1>")
    f:close()

    -- Create iPhone variant
    f = io.open(test_views_path .. "/posts/show.iphone.etlua", "w")
    f:write("<h1>iPhone View</h1>")
    f:close()

    -- Create tablet variant
    f = io.open(test_views_path .. "/posts/show.tablet.etlua", "w")
    f:write("<h1>Tablet View</h1>")
    f:close()

    -- Create layout (in folder structure: layouts/application/application.etlua)
    f = io.open(test_views_path .. "/layouts/application/application.etlua", "w")
    f:write("<%- yield() %>")
    f:close()
  end)

  after(function()
    -- Cleanup test fixtures
    os.remove(test_views_path .. "/posts/show.etlua")
    os.remove(test_views_path .. "/posts/show.iphone.etlua")
    os.remove(test_views_path .. "/posts/show.tablet.etlua")
    os.remove(test_views_path .. "/layouts/application/application.etlua")
    os.remove(test_views_path .. "/layouts/application")
    os.remove(test_views_path .. "/posts")
    os.remove(test_views_path .. "/layouts")
    os.remove(test_views_path)
    View.set_views_path("app/views")
  end)

  it("should render default template without variant", function()
    local content = View.render("posts/show", {}, { layout = false })
    expect.matches(content, "Desktop View")
  end)

  it("should render iPhone variant when specified", function()
    local content = View.render("posts/show", {}, { layout = false, variant = "iphone" })
    expect.matches(content, "iPhone View")
  end)

  it("should render tablet variant when specified", function()
    local content = View.render("posts/show", {}, { layout = false, variant = "tablet" })
    expect.matches(content, "Tablet View")
  end)

  it("should fall back to default when variant not found", function()
    local content = View.render("posts/show", {}, { layout = false, variant = "android" })
    expect.matches(content, "Desktop View")
  end)

  it("should fall back to default when variant is nil", function()
    local content = View.render("posts/show", {}, { layout = false, variant = nil })
    expect.matches(content, "Desktop View")
  end)
end)

describe("Controller Variants", function()
  local Controller
  local View
  local test_views_path = "test/fixtures/views"

  before(function()
    Controller = require("controller")
    View = require("view")
    View.set_views_path(test_views_path)
    View.clear_cache()

    -- Create test fixture directories and files
    os.execute("mkdir -p " .. test_views_path .. "/users")
    os.execute("mkdir -p " .. test_views_path .. "/layouts/application")

    -- Create default template
    local f = io.open(test_views_path .. "/users/index.etlua", "w")
    f:write("<h1>Users Desktop</h1>")
    f:close()

    -- Create mobile variant
    f = io.open(test_views_path .. "/users/index.mobile.etlua", "w")
    f:write("<h1>Users Mobile</h1>")
    f:close()

    -- Create layout (in folder structure: layouts/application/application.etlua)
    f = io.open(test_views_path .. "/layouts/application/application.etlua", "w")
    f:write("<%- yield() %>")
    f:close()
  end)

  after(function()
    -- Cleanup test fixtures
    os.remove(test_views_path .. "/users/index.etlua")
    os.remove(test_views_path .. "/users/index.mobile.etlua")
    os.remove(test_views_path .. "/layouts/application/application.etlua")
    os.remove(test_views_path .. "/layouts/application")
    os.remove(test_views_path .. "/users")
    os.remove(test_views_path .. "/layouts")
    os.remove(test_views_path)
    View.set_views_path("app/views")
  end)

  it("should use controller variant property", function()
    local TestController = Controller:extend()

    function TestController:index()
      self.variant = "mobile"
      self:render("users/index", {})
    end

    local ctx = { params = {}, request = {} }
    local c = TestController:new(ctx)
    c:index()

    expect.matches(c.response.body, "Users Mobile")
  end)

  it("should allow variant override in render options", function()
    local TestController = Controller:extend()

    function TestController:index()
      self.variant = "desktop" -- This would be overridden
      self:render("users/index", {}, { variant = "mobile" })
    end

    local ctx = { params = {}, request = {} }
    local c = TestController:new(ctx)
    c:index()

    expect.matches(c.response.body, "Users Mobile")
  end)

  it("should render default when no variant set", function()
    local TestController = Controller:extend()

    function TestController:index()
      self:render("users/index", {})
    end

    local ctx = { params = {}, request = {} }
    local c = TestController:new(ctx)
    c:index()

    expect.matches(c.response.body, "Users Desktop")
  end)
end)

return Test
