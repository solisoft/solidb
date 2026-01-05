-- Integration test for Post CRUD
-- test/posts_crud_test.lua

package.path = package.path .. ";.lua/?.lua;app/controllers/?.lua;app/models/?.lua"

local Test = require("test")
local Router = require("router")
local Post = require("post")
local PostsController = require("posts_controller")
local describe, it, expect, before, after = Test.describe, Test.it, Test.expect, Test.before, Test.after
local TestHelpers = require("test_helpers")

describe("Post CRUD Integration", function()
  local ctx
  
  before(function()
    TestHelpers.setup_test_db()
    TestHelpers.setup_router()
    -- Register routes explicitly if not loading .init.lua
    Router.clear()
    Router.resources("posts")
  end)

  it("should create a post via controller", function()
    local params = {
      post = {
        title = "Integration Test",
        body = "Body Content"
      }
    }
    
    ctx = TestHelpers.create_context("POST", "/posts", params)
    
    -- Force reload to ensure we get the latest file from disk
    package.loaded["posts_controller"] = nil
    local PostsController = require("posts_controller")

    local c = PostsController:new(ctx)
    c:create()
    
    expect.not_nil(c.response)
    expect.eq(c.response.status, 302)
    local location = c.response.headers["Location"]
    expect.matches(location, "/posts/%d+")
    -- Check database
    local posts = Post:all()
    expect.eq(#posts, 1)
  end)
  
  it("should list posts", function()
    -- Create fixture data
    Post:create({ title = "P1", body = "B1" })
    Post:create({ title = "P2", body = "B2" })
    
    local ctx = TestHelpers.create_context("GET", "/posts")
    local c = PostsController:new(ctx)
    c:index()
    
    expect.eq(c.response.status, 200)
    -- Verify rendered content contains titles
    expect.matches(c.response.body, "P1")
    expect.matches(c.response.body, "P2")
  end)
  
  it("should show a post", function()
    local p = Post:create({
      title = "Show Me",
      body = "Content"
    })
    
    local ctx = TestHelpers.create_context("GET", "/posts/" .. p.id)
    ctx.params.id = p.id -- Router usually extracts this
    
    local c = PostsController:new(ctx)
    c:show()
    
    expect.eq(c.response.status, 200)
    expect.matches(c.response.body, "Show Me")
  end)
  
  it("should update a post", function()
    local p = Post:create({ title = "Original", body = "Content" })
    
    local ctx = TestHelpers.create_context("PUT", "/posts/" .. p.id, {
      post = { title = "Updated" }
    })
    ctx.params.id = p.id
    
    local c = PostsController:new(ctx)
    c:update()
    
    expect.eq(c.response.status, 302)
    local updated = Post:find(p.id)
    expect.eq(updated.title, "Updated")
  end)
  
  it("should delete a post", function()
    local p = Post:create({ title = "To Delete", body = "Content" })
    
    local ctx = TestHelpers.create_context("DELETE", "/posts/" .. p.id)
    ctx.params.id = p.id
    
    local c = PostsController:new(ctx)
    c:destroy()
    
    expect.eq(c.response.status, 302)
    local found = Post:find(p.id)
    expect.eq(found, nil)
  end)
  
end)
