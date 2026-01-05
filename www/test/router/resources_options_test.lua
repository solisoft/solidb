-- Tests for resources options and custom routes
-- test/resources_options_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local Router = require("router")
local describe, it, expect, before, after = Test.describe, Test.it, Test.expect, Test.before, Test.after

describe("Router Resources Options", function()
  before(function()
    Router.clear()
  end)

  it("should respect 'only' option", function()
    Router.resources("users", { only = {"index", "show"} })
    
    -- Should exist
    local r, _ = Router.match("GET", "/users")
    expect.not_nil(r)
    local r, _ = Router.match("GET", "/users/1")
    expect.not_nil(r)
    
    -- Should NOT exist
    local r, _ = Router.match("POST", "/users")
    expect.nil_value(r)
    local r, _ = Router.match("DELETE", "/users/1")
    expect.nil_value(r)
  end)

  it("should respect 'except' option", function()
    Router.resources("posts", { except = {"destroy"} })
    
    -- Should exist
    local r, _ = Router.match("GET", "/posts")
    expect.not_nil(r)
    
    -- Should NOT exist
    local r, _ = Router.match("DELETE", "/posts/1")
    expect.nil_value(r)
  end)

  it("should allow custom routes definitions inside block", function()
    Router.resources("articles", { only = {"index"} }, function()
       -- Defines a custom member route
       Router.get("publish", "articles#publish")
       
       -- Defines nested resource
       Router.resources("comments", { only = {"index"} })
    end)
    
    -- Default route
    local r, _ = Router.match("GET", "/articles")
    expect.not_nil(r)
    
    -- Custom member route: /articles/:article_id/publish
    local r, params = Router.match("GET", "/articles/123/publish")
    expect.not_nil(r)
    expect.eq(r.handler, "articles#publish")
    expect.eq(params.article_id, "123")
    
    -- Nested resource: /articles/:article_id/comments
    local r, params = Router.match("GET", "/articles/123/comments")
    expect.not_nil(r)
    expect.eq(r.handler, "comments#index")
  end)
end)
