-- Tests for nested resources
-- test/nested_resources_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local Router = require("router")
local describe, it, expect, before, after = Test.describe, Test.it, Test.expect, Test.before, Test.after

describe("Router Nested Resources", function()
  before(function()
    Router.clear()
  end)

  it("should support basic nested resources", function()
    Router.resources("users", function()
      Router.resources("posts")
    end)
    
    -- Check user route
    local route, params = Router.match("GET", "/users")
    expect.not_nil(route)
    expect.eq(route.handler, "users#index")

    -- Check nested index route
    -- /users/:user_id/posts
    local route, params = Router.match("GET", "/users/123/posts")
    expect.not_nil(route)
    expect.eq(route.handler, "posts#index")
    expect.eq(params.user_id, "123")
  end)

  it("should support deeply nested resources", function()
    Router.resources("users", function()
      Router.resources("posts", function()
         Router.resources("comments")
      end)
    end)
    
    -- /users/:user_id/posts/:post_id/comments/:id
    local route, params = Router.match("GET", "/users/1/posts/2/comments/3")
    expect.not_nil(route)
    expect.eq(route.handler, "comments#show")
    expect.eq(params.user_id, "1")
    expect.eq(params.post_id, "2")
    expect.eq(params.id, "3")
  end)
  
  it("should support custom param name in nesting", function()
    Router.resources("categories", { param = "slug" }, function()
       Router.resources("articles")
    end)
    
    -- /categories/:category_slug/articles
    -- Wait, logic: if param="slug", then parent param becomes ":category_slug" or just ":slug"?
    -- Usually "slug_id" is weird.
    -- If option `id_param` is provided, use that for parent param?
    -- Rails: param: 'slug' -> /categories/:slug/articles
    -- But if I nest, I need to know what the parent param is called in the child URL.
    
    -- Proposed implementation:
    -- default: derived from name (user -> user_id)
    -- option `parent_param`: override the parent param NAME when nesting.
    -- option `id`: override the :id param name for THIS resource.
    
    -- If user does: resources("categories", { id = "slug" })
    -- Then URL is /categories/:slug
    -- Nested URL: /categories/:slug/articles
    -- Params: { slug = "...", ... }
    
    -- Let's test this assumption
    Router.clear()
    Router.resources("categories", { id = "slug" }, function()
       Router.resources("articles")
    end)
    
    local route, params = Router.match("GET", "/categories/tech/articles")
    expect.not_nil(route)
    expect.eq(params.slug, "tech")
  end)
end)
