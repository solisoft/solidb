-- Tests for content_for helper
-- test/content_for_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local View = require("view")
local describe, it, expect, before, after = Test.describe, Test.it, Test.expect, Test.before, Test.after

describe("View content_for", function()
  before(function()
    View.clear_cache()
    -- Set up fixtures for view/layout
    -- Assuming we can configure paths or write to files?
    -- View.render reads from filesystem or LoadAsset.
    -- We'll rely on existing paths or maybe mocking?
    
    -- But View.render uses `read_file` which tries io.open.
    -- I can write temporary view files.
  end)

  -- Creating temporary files for testing.
  -- Warning: This writes to the workspace. cleanup needed.
  local test_view_dir = "test/views_temp"
  local view_path = test_view_dir .. "/content_test.etlua"
  local layout_path = test_view_dir .. "/layouts/content_layout/content_layout.etlua"
  local partial_path = test_view_dir .. "/_partial_content.etlua"

  local function setup_files()
    os.execute("mkdir -p " .. test_view_dir .. "/layouts/content_layout")
    
    local f = io.open(view_path, "w")
    f:write([[
<h1>Main Content</h1>
<% content_for("header", function() %>
  <title>Captured Title</title>
<% end) %>

<% content_for("footer", "Copyright 2025") %>

<% partial("partial_content") %>
]])
    f:close()
    
    local f = io.open(layout_path, "w")
    f:write([[
<html>
<head>
  <%- content_for("header") %>
</head>
<body>
  <%- yield() %>
  <footer><%- content_for("footer") %></footer>
  <sidebar><%- content_for("sidebar") %></sidebar>
</body>
</html>
]])
    f:close()
    
    local f = io.open(partial_path, "w")
    f:write([[
<% content_for("sidebar", function() %>
  <ul><li>Item</li></ul>
<% end) %>
]])
    f:close()
  end
  
  local function cleanup_files()
    os.remove(layout_path)
    os.remove(test_view_dir .. "/layouts/content_layout")
    os.remove(test_view_dir .. "/layouts")
    os.remove(view_path)
    os.remove(partial_path)
    os.remove(test_view_dir)
  end

  it("should capture and yield content correctly", function()
    setup_files()
    
    View.set_views_path(test_view_dir)
    View.set_layouts_path(test_view_dir .. "/layouts")
    
    local result = View.render("content_test", {}, { layout = "content_layout" })
    
    -- Check Header capture
    expect.matches(result, "<head>%s*<title>Captured Title</title>%s*</head>")
    
    -- Check Footer (string set)
    expect.matches(result, "<footer>Copyright 2025</footer>")
    
    -- Check Sidebar (from partial)
    expect.matches(result, "<sidebar>%s*<ul><li>Item</li></ul>%s*</sidebar>")
    
    -- Check Main Yield
    expect.matches(result, "<h1>Main Content</h1>")
    
    -- Check that captured content is NOT in main body where it was defined
    -- This is tricky to match negatively with regex safely, but order check implies it.
    -- We verify that the text "Captured Title" appears ONLY in Head?
    local _, count = result:gsub("Captured Title", "")
    expect.eq(count, 1)

    cleanup_files()
  end)
  
end)
