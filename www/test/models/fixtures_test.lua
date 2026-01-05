-- Tests for fixtures
-- test/fixtures_test.lua

package.path = package.path .. ";.lua/?.lua;app/models/?.lua"

local Test = require("test")
local TestHelpers = require("test_helpers")
local describe, it, expect, before, after = Test.describe, Test.it, Test.expect, Test.before, Test.after

describe("Fixtures", function()
  local User
  local mock_db

  before(function()
    mock_db = TestHelpers.setup_test_db()
    User = require("user")
    TestHelpers.fixtures("users")
  end)

  after(function()
    TestHelpers.teardown_test_db()
  end)

  it("should load users fixture", function()
    local users = User.all().data
    expect.truthy(#users > 0)
    expect.eq(#users, 2)
  end)

  it("should find fixture data by query", function()
    local john = User.find_by({ email = "john@example.com" })
    expect.not_nil(john.data)
    expect.eq(john.data.username, "johndoe")
    expect.eq(john.data.role, "user")
  end)
  
  it("should find active users", function()
    local active_users = User.where({ active = true }):all().data
    expect.eq(#active_users, 1)
    expect.eq(active_users[1].username, "johndoe")
  end)
end)
