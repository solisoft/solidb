-- Tests for collection and member scopes
-- test/resources_scopes_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local Router = require("router")
local describe, it, expect, before, after = Test.describe, Test.it, Test.expect, Test.before, Test.after

describe("Router Scopes", function()
  before(function()
    Router.clear()
  end)

  it("should support collection routes", function()
    Router.resources("users", function()
      Router.collection(function()
        Router.get("search", "users#search")
      end)
    end)
    
    -- /users/search
    local r, params = Router.match("GET", "/users/search")
    expect.not_nil(r)
    expect.eq(r.handler, "users#search")
    expect.nil_value(params.user_id) -- Should not have ID
  end)

  it("should support member routes explicit block", function()
    Router.resources("users", function()
      Router.member(function()
        Router.post("ban", "users#ban")
      end)
    end)
    
    -- /users/:user_id/ban
    local r, params = Router.match("POST", "/users/123/ban")
    expect.not_nil(r)
    expect.eq(r.handler, "users#ban")
    expect.eq(params.user_id, "123")
  end)

  it("should support nested collection routes", function()
    Router.resources("users", function()
       Router.resources("posts", function()
          Router.collection(function()
             Router.get("archive", "posts#archive")
          end)
       end)
    end)
    
    -- /users/:user_id/posts/archive (Collection of posts for a user)
    local r, params = Router.match("GET", "/users/1/posts/archive")
    expect.not_nil(r)
    expect.eq(r.handler, "posts#archive")
    expect.eq(params.user_id, "1")
    expect.nil_value(params.post_id) -- No post_id
  end)
end)
