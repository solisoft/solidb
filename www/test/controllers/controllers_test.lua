-- Tests for controllers
-- test/controllers_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local TestHelpers = require("test_helpers")
local describe, it, expect, before, after = Test.describe, Test.it, Test.expect, Test.before, Test.after

describe("Controllers", function()

  describe("HomeController", function()
    local HomeController

    before(function()
      HomeController = require("home_controller")
    end)

    it("should render index page", function()
      local ctrl = TestHelpers.mock_controller(HomeController, "index")

      TestHelpers.assert_rendered(ctrl, "home/index")
    end)

    it("should pass features data to index view", function()
      local ctrl = TestHelpers.mock_controller(HomeController, "index")

      expect.not_nil(ctrl._response.locals.features)
      expect.eq(#ctrl._response.locals.features, 4)
    end)

    it("should render about page", function()
      local ctrl = TestHelpers.mock_controller(HomeController, "about")

      TestHelpers.assert_rendered(ctrl, "home/about")
    end)

    it("should set correct title for about page", function()
      local ctrl = TestHelpers.mock_controller(HomeController, "about")

      expect.eq(ctrl._response.locals.title, "About Luaonbeans")
    end)
  end)

  describe("DocsController", function()
    local DocsController

    before(function()
      DocsController = require("docs_controller")
    end)

    it("should render docs index", function()
      local ctrl = TestHelpers.mock_controller(DocsController, "index")

      TestHelpers.assert_rendered(ctrl, "docs/index")
    end)

    it("should use docs layout", function()
      local ctrl = TestHelpers.mock_controller(DocsController, "index")

      expect.eq(ctrl.layout, "docs")
    end)

    it("should render routing documentation", function()
      local ctrl = TestHelpers.mock_controller(DocsController, "routing")

      TestHelpers.assert_rendered(ctrl, "docs/routing")
    end)

    it("should render controllers documentation", function()
      local ctrl = TestHelpers.mock_controller(DocsController, "controllers")

      TestHelpers.assert_rendered(ctrl, "docs/controllers")
    end)

    it("should render views documentation", function()
      local ctrl = TestHelpers.mock_controller(DocsController, "views")

      TestHelpers.assert_rendered(ctrl, "docs/views")
    end)

    it("should render helpers documentation", function()
      local ctrl = TestHelpers.mock_controller(DocsController, "helpers")

      TestHelpers.assert_rendered(ctrl, "docs/helpers")
    end)

    it("should render models documentation", function()
      local ctrl = TestHelpers.mock_controller(DocsController, "models")

      TestHelpers.assert_rendered(ctrl, "docs/models")
    end)

    it("should render testing documentation", function()
      local ctrl = TestHelpers.mock_controller(DocsController, "testing")

      TestHelpers.assert_rendered(ctrl, "docs/testing")
    end)

    it("should render middleware documentation", function()
      local ctrl = TestHelpers.mock_controller(DocsController, "middleware")

      TestHelpers.assert_rendered(ctrl, "docs/middleware")
    end)
  end)

  describe("Controller filters", function()
    local TestController

    before(function()
      local Controller = require("controller")
      TestController = Controller:extend()

      function TestController:before_action()
        self.layout = "custom"
      end

      function TestController:index()
        self:render("test/index")
      end
    end)

    it("should run before_action filter", function()
      local ctrl = TestHelpers.mock_controller(TestController, "index")

      expect.eq(ctrl.layout, "custom")
    end)
  end)

end)

return Test
