-- Tests for models
-- test/models_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local TestHelpers = require("test_helpers")
local describe, it, expect, before, after = Test.describe, Test.it, Test.expect, Test.before, Test.after

describe("Models", function()

  describe("Model", function()
    local User

    before(function()
      User = require("user")
    end)

    it("should create a model class", function()
      expect.not_nil(User)
      expect.eq(User.COLLECTION, "users")
    end)

    it("should have validations", function()
      expect.not_nil(User._validations)
      expect.not_nil(User._validations.email)
      expect.not_nil(User._validations.username)
    end)
  end)

  describe("Model with mock database", function()
    local User
    local mock_db

    before(function()
      mock_db = TestHelpers.setup_test_db()
      User = require("user")
    end)

    after(function()
      TestHelpers.teardown_test_db()
    end)

    it("should create a new user instance", function()
      local user = User.new()

      expect.not_nil(user)
      expect.eq(user.COLLECTION, "users")
    end)

    it("should create a user via class method", function()
      local user = User.create({
        email = "test@example.com",
        username = "testuser"
      })

      expect.not_nil(user)
    end)

    it("should validate presence of email", function()
      local user = User.new()
      user.data = {}
      user:validates_each({ username = "test" })

      expect.truthy(#user.errors > 0)
    end)

    it("should validate presence of username", function()
      local user = User.new()
      user.data = {}
      user:validates_each({ email = "test@example.com" })

      expect.truthy(#user.errors > 0)
    end)

    it("should validate email format", function()
      local user = User.new()
      user.data = {}
      user:validates_each({
        email = "invalid-email",
        username = "test"
      })

      expect.truthy(#user.errors > 0)
    end)

    it("should validate username length", function()
      local user = User.new()
      user.data = {}
      user:validates_each({
        email = "test@example.com",
        username = "ab" -- Too short (min 3)
      })

      expect.truthy(#user.errors > 0)
    end)
  end)

  describe("Model query methods", function()
    local User
    local mock_db

    before(function()
      mock_db = TestHelpers.setup_test_db()
      User = require("user")
    end)

    after(function()
      TestHelpers.teardown_test_db()
    end)

    it("should support find method", function()
      User.create({ email = "test@example.com", username = "testuser" })
      local user = User.find("users/1")

      expect.not_nil(user)
    end)

    it("should support find_by method", function()
      User.create({ email = "test@example.com", username = "testuser" })
      local user = User.find_by({ email = "test@example.com" })

      expect.not_nil(user)
    end)

    it("should support where method", function()
      local users = User.where({ active = true }):all()

      expect.not_nil(users)
    end)

    it("should support all method", function()
      local users = User.all()

      expect.not_nil(users)
    end)

    it("should support count method", function()
      local count = User.count()

      expect.not_nil(count)
      expect.eq(type(count), "number")
    end)
  end)

  describe("Model callbacks", function()
    local TestModel
    local mock_db

    before(function()
      mock_db = TestHelpers.setup_test_db()
      local Model = require("model")

      TestModel = Model.create("test_items", {
        before_create = {},
        after_create = {},
        before_update = {},
        after_update = {}
      })
    end)

    after(function()
      TestHelpers.teardown_test_db()
    end)

    it("should run before_create callbacks", function()
      local test_data = { name = "Test" }

      local instance = TestModel.new()
      expect.truthy(instance.callbacks.before_create)
    end)

    it("should support custom methods", function()
      TestModel.custom_method = function()
        return "custom result"
      end

      local result = TestModel.custom_method()
      expect.eq(result, "custom result")
    end)
  end)

end)

return Test
