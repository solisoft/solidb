-- Tests for mass assignment protection
-- test/models/mass_assignment_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local TestHelpers = require("test_helpers")
local describe, it, expect, before, after = Test.describe, Test.it, Test.expect, Test.before, Test.after

describe("Mass Assignment Protection", function()

  describe("permit() method", function()
    local Model
    local TestModel

    before(function()
      Model = require("model")
      TestModel = Model.create("test_items", {
        permitted_fields = { "title", "body", "status" },
        validations = {
          title = { presence = true }
        }
      })
    end)

    it("should filter out non-permitted fields", function()
      local input = {
        title = "Test Title",
        body = "Test Body",
        is_admin = true,
        role = "superuser"
      }

      local filtered = TestModel.permit(input)

      expect.eq(filtered.title, "Test Title")
      expect.eq(filtered.body, "Test Body")
      expect.eq(filtered.is_admin, nil)
      expect.eq(filtered.role, nil)
    end)

    it("should keep all permitted fields", function()
      local input = {
        title = "Test",
        body = "Content",
        status = "published"
      }

      local filtered = TestModel.permit(input)

      expect.eq(filtered.title, "Test")
      expect.eq(filtered.body, "Content")
      expect.eq(filtered.status, "published")
    end)

    it("should handle empty input", function()
      local filtered = TestModel.permit({})

      expect.not_nil(filtered)
      expect.eq(filtered.title, nil)
    end)

    it("should handle nil input", function()
      local filtered = TestModel.permit(nil)

      expect.eq(filtered, nil)
    end)

    it("should preserve nil values for permitted fields", function()
      local input = {
        title = "Test",
        body = nil,
        status = "draft"
      }

      local filtered = TestModel.permit(input)

      expect.eq(filtered.title, "Test")
      expect.eq(filtered.body, nil)
      expect.eq(filtered.status, "draft")
    end)

    it("should handle numeric values", function()
      local NumericModel = Model.create("numeric_items", {
        permitted_fields = { "name", "count", "price" }
      })

      local input = {
        name = "Item",
        count = 42,
        price = 19.99,
        secret_id = 12345
      }

      local filtered = NumericModel.permit(input)

      expect.eq(filtered.name, "Item")
      expect.eq(filtered.count, 42)
      expect.eq(filtered.price, 19.99)
      expect.eq(filtered.secret_id, nil)
    end)

    it("should handle boolean values", function()
      local BoolModel = Model.create("bool_items", {
        permitted_fields = { "name", "active", "published" }
      })

      local input = {
        name = "Test",
        active = true,
        published = false,
        is_admin = true
      }

      local filtered = BoolModel.permit(input)

      expect.eq(filtered.name, "Test")
      expect.eq(filtered.active, true)
      expect.eq(filtered.published, false)
      expect.eq(filtered.is_admin, nil)
    end)
  end)

  describe("Model without permitted_fields", function()
    local Model
    local OpenModel

    before(function()
      Model = require("model")
      OpenModel = Model.create("open_items", {
        validations = {}
      })
    end)

    it("should return data unchanged when no permitted_fields defined", function()
      local input = {
        title = "Test",
        anything = "allowed",
        secret = "value"
      }

      local result = OpenModel.permit(input)

      expect.eq(result.title, "Test")
      expect.eq(result.anything, "allowed")
      expect.eq(result.secret, "value")
    end)
  end)

  describe("Integration with model operations", function()
    local Model
    local Post
    local mock_db

    before(function()
      mock_db = TestHelpers.setup_test_db()
      Model = require("model")
      Post = Model.create("posts", {
        permitted_fields = { "title", "body" },
        validations = {
          title = { presence = true },
          body = { presence = true }
        }
      })
    end)

    after(function()
      TestHelpers.teardown_test_db()
    end)

    it("should create model with only permitted fields", function()
      local input = {
        title = "My Post",
        body = "Content here",
        user_id = 999,
        is_admin = true
      }

      local post = Post:new(Post.permit(input))

      expect.eq(post.data.title, "My Post")
      expect.eq(post.data.body, "Content here")
      expect.eq(post.data.user_id, nil)
      expect.eq(post.data.is_admin, nil)
    end)

    it("should save model with filtered data", function()
      local input = {
        title = "Safe Post",
        body = "Safe content",
        role = "admin",
        secret_key = "abc123"
      }

      local post = Post:new(Post.permit(input))
      post:save()

      expect.eq(post.data.title, "Safe Post")
      expect.eq(post.data.body, "Safe content")
      expect.eq(post.data.role, nil)
      expect.eq(post.data.secret_key, nil)
    end)

    it("should update model with only permitted fields", function()
      local post = Post:new({ title = "Original", body = "Original body" })
      post:save()

      local update_data = {
        title = "Updated Title",
        body = "Updated body",
        is_admin = true
      }

      post:update(Post.permit(update_data))

      expect.eq(post.data.title, "Updated Title")
      expect.eq(post.data.body, "Updated body")
      expect.eq(post.data.is_admin, nil)
    end)
  end)

  describe("Security scenarios", function()
    local Model
    local User

    before(function()
      TestHelpers.setup_test_db()
      Model = require("model")
      User = Model.create("users", {
        permitted_fields = { "email", "username", "password" },
        validations = {
          email = { presence = true },
          username = { presence = true }
        }
      })
    end)

    after(function()
      TestHelpers.teardown_test_db()
    end)

    it("should block privilege escalation via is_admin field", function()
      local malicious_input = {
        email = "hacker@example.com",
        username = "hacker",
        password = "password123",
        is_admin = true
      }

      local filtered = User.permit(malicious_input)

      expect.eq(filtered.email, "hacker@example.com")
      expect.eq(filtered.username, "hacker")
      expect.eq(filtered.password, "password123")
      expect.eq(filtered.is_admin, nil)
    end)

    it("should block role manipulation", function()
      local malicious_input = {
        email = "user@example.com",
        username = "normaluser",
        role = "superadmin",
        permissions = { "all" }
      }

      local filtered = User.permit(malicious_input)

      expect.eq(filtered.role, nil)
      expect.eq(filtered.permissions, nil)
    end)

    it("should block id manipulation", function()
      local malicious_input = {
        email = "user@example.com",
        username = "user",
        _id = "users/1",
        _key = "1",
        id = 1
      }

      local filtered = User.permit(malicious_input)

      expect.eq(filtered._id, nil)
      expect.eq(filtered._key, nil)
      expect.eq(filtered.id, nil)
    end)

    it("should block timestamp manipulation", function()
      local malicious_input = {
        email = "user@example.com",
        username = "user",
        c_at = 0,
        u_at = 0,
        created_at = "2020-01-01"
      }

      local filtered = User.permit(malicious_input)

      expect.eq(filtered.c_at, nil)
      expect.eq(filtered.u_at, nil)
      expect.eq(filtered.created_at, nil)
    end)
  end)

end)

return Test
