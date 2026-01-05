-- Test Helpers for Luaonbeans
-- Provides utilities for testing controllers, models, and requests

local TestHelpers = {}

-- Mock request/response for controller testing
function TestHelpers.mock_request(method, path, params)
  return {
    method = method or "GET",
    path = path or "/",
    params = params or {},
    headers = {}
  }
end

-- Create a generic request context
function TestHelpers.create_context(method, path, params)
  return {
    params = params or {},
    request = {
      method = method or "GET",
      path = path or "/"
    }
  }
end

-- Setup router (clears previous routes)
function TestHelpers.setup_router()
  local Router = require("router")
  Router.clear()
end

-- Mock controller context
function TestHelpers.mock_controller(controller_class, action, params)
  local mock_ctx = {
    params = params or {},
    request = TestHelpers.mock_request()
  }
  local instance = controller_class:new(mock_ctx)
  instance.params = params or {}
  instance._response = nil
  instance._status = 200
  instance._headers = {}
  instance._rendered = false
  
  -- Override render to capture output
  local original_render = instance.render
  instance.render = function(self, template, locals, options)
    self._rendered = true
    self._response = {
      type = "render",
      template = template,
      locals = locals or {},
      options = options or {}
    }
    
    -- Also emulate rendering logic by reading template?
    -- For CRUD tests, we might want to check rendered content.
    -- If we use real View.render inside controller, we need proper paths.
    
    -- BUT Controller:render implementation normally calls View.render.
    -- mock_controller overrides it to capture metadata instead of full HTML.
    -- If we want to verify output HTML (like matching titles), we should call ORIGINAL render?
    -- OR capture the provided locals and verify data.
    
    -- The test expects: expect.matches(c.response.body, "P1")
    -- PostsController calls self:render("posts/index", ...).
    -- If using mock_controller logic here:
    -- it sets c.response = { type="render", template="...", ... }.
    -- it does NOT generate .body.
    
    -- PostsCRUDTest creates instance manually:
    -- 'local c = PostsController:new(ctx)'
    -- It does NOT use mock_controller helper.
    -- So 'c' is a REAL controller instance using REAL render method (unless overridden?).
    -- PostsController inherits from Controller. 
    -- Controller:render calls View.render.
    -- View.render calls template file.
    
    -- So manually creating controller is better for Integration test that checks output.
    -- And create_context provides the input.
    -- But Controller:render writes to `self.response.body`?
    -- Standard Controller:render sets self.response.body = content.
    
    -- So using create_context and PostsController:new(ctx) should work fine
    -- provided that views exist (we created them).
  end
  
  -- Override json to capture output
  instance.json = function(self, data, status)
    self._rendered = true
    self._status = status or 200
    self._response = {
      type = "json",
      data = data
    }
  end
  
  -- Override redirect to capture output
  instance.redirect = function(self, url, status)
    self._rendered = true
    self._status = status or 302
    self._response = {
      type = "redirect",
      url = url
    }
  end
  
  -- Override text to capture output
  instance.text = function(self, content)
    self._rendered = true
    self._response = {
      type = "text",
      content = content
    }
  end
  
  -- Run action if provided
  if action and instance[action] then
    -- Run before filters
    if instance.before_action then instance:before_action() end
    
    -- Run action
    instance[action](instance)
    
    -- Run after filters
    if instance.after_action then instance:after_action() end
  end
  
  return instance
end

-- Assert controller rendered a template
function TestHelpers.assert_rendered(controller, template)
  if not controller._response then
    error("Controller did not render anything", 2)
  end
  if controller._response.type ~= "render" then
    error("Controller did not render a template (got " .. controller._response.type .. ")", 2)
  end
  if template and controller._response.template ~= template then
    error("Expected template '" .. template .. "', got '" .. controller._response.template .. "'", 2)
  end
end

-- Assert controller returned JSON
function TestHelpers.assert_json(controller, expected_data)
  if not controller._response then
    error("Controller did not respond", 2)
  end
  if controller._response.type ~= "json" then
    error("Controller did not return JSON (got " .. controller._response.type .. ")", 2)
  end
  if expected_data then
    for k, v in pairs(expected_data) do
      if controller._response.data[k] ~= v then
        error("JSON key '" .. k .. "' expected '" .. tostring(v) .. "', got '" .. tostring(controller._response.data[k]) .. "'", 2)
      end
    end
  end
end

-- Assert controller redirected
function TestHelpers.assert_redirected_to(controller, url)
  if not controller._response then
    error("Controller did not respond", 2)
  end
  if controller._response.type ~= "redirect" then
    error("Controller did not redirect (got " .. controller._response.type .. ")", 2)
  end
  if url and controller._response.url ~= url then
    error("Expected redirect to '" .. url .. "', got '" .. controller._response.url .. "'", 2)
  end
end

-- Assert response status
function TestHelpers.assert_status(controller, status)
  if controller._status ~= status then
    error("Expected status " .. status .. ", got " .. controller._status, 2)
  end
end

-- Mock database for model testing
function TestHelpers.mock_db()
  local mock = {
    _documents = {},
    _queries = {},
    _last_id = 0
  }
  
  function mock:CreateDocument(collection, data)
    self._last_id = self._last_id + 1
    local id = collection .. "/" .. self._last_id
    data._id = id
    data._key = tostring(self._last_id)
    self._documents[id] = data
    return { new = data }
  end
  
  function mock:GetDocument(id)
    return self._documents[id]
  end
  
  function mock:UpdateDocument(id, data)
    if self._documents[id] then
      for k, v in pairs(data) do
        self._documents[id][k] = v
      end
      return { new = self._documents[id] }
    end
    return nil
  end
  
  function mock:DeleteDocument(id)
    local doc = self._documents[id]
    self._documents[id] = nil
    return doc
  end
  
  function mock:Sdbql(query, bindvars)
    table.insert(self._queries, { query = query, bindvars = bindvars })
    
    -- AQL-lite query parser for mock DB
    local collection = bindvars and bindvars["@collection"]
    
    -- Also check for FOR doc IN `collection` format
    if not collection then
      collection = query:match("FOR%s+doc%s+IN%s+`([%w_]+)`")
    end
    
    if not collection and query:match("SELECT .+ FROM") then
      collection = query:match("FROM%s+([%w_]+)")
    end

    if not collection then
      return { result = {} }
    end

    
    local candidates = {}
    for id, doc in pairs(self._documents) do
      if id:sub(1, #collection + 1) == collection .. "/" then
        table.insert(candidates, doc)
      end
    end
    
    -- Check if it's a COUNT query
    if query:match("COLLECT WITH COUNT") then
      return { result = { #candidates } }
    end
    
    -- Apply filters: FILTER doc.field == @bindvar
    local filtered = {}
    for _, doc in ipairs(candidates) do
      local match = true
      
      -- Match patterns like: FILTER doc.field == @varname
      for field, bindvar in query:gmatch("FILTER%s+doc%.([%w_]+)%s*==%s*@([%w_]+)") do
        local expected = bindvars[bindvar]
        
        if doc[field] ~= expected then
          match = false
          break
        end
      end
      
      if match then
        table.insert(filtered, doc)
      end
    end

    
    -- Apply ORDER BY
    local sort_field, sort_dir
    local sort_match = query:match("SORT%s+([^%s]+)")
    if sort_match then
      sort_field, sort_dir = sort_match:match("doc%.([%w_]+)%s*(%a*)")
    end
    if not sort_field then
      sort_field = "_id"
      sort_dir = "ASC"
    end
    
    table.sort(filtered, function(a, b)
      local a_val = a[sort_field] or a._id
      local b_val = b[sort_field] or b._id
      if sort_dir:upper() == "ASC" then
        return a_val < b_val
      else
        return a_val > b_val
      end
    end)
    
    -- Parse LIMIT
    local offset = bindvars["offset"] or bindvars["@offset"] or 0
    local limit = bindvars["per_page"] or bindvars["@per_page"] or bindvars["limit"] or 1000
    
    local result = {}
    for i = offset + 1, math.min(offset + limit, #filtered) do
      table.insert(result, filtered[i])
    end
    
    return { result = result }
  end
  
  return mock
end

-- Setup test database (creates mock and injects into global Sdb)
function TestHelpers.setup_test_db()
  local mock = TestHelpers.mock_db()
  
  -- Save original Sdb global (if any)
  TestHelpers._original_sdb = _G.Sdb
  
  -- Inject mock db into global Sdb (used by SoliDBModel)
  _G.Sdb = mock
  
  return mock
end

-- Teardown test database (restores original Sdb)
function TestHelpers.teardown_test_db()
  _G.Sdb = TestHelpers._original_sdb
  TestHelpers._original_sdb = nil
end

-- Load fixture data into mock database
function TestHelpers.fixtures(name)
  -- Add test/fixtures to path for loading
  package.path = package.path .. ";test/fixtures/?.lua"
  
  local fixture_data = require(name)
  
  -- Current Sdb (should be mock)
  local db = _G.Sdb
  if not db then
    error("No mock database set up. Call setup_test_db() first.", 2)
  end
  
  -- Infer collection from fixture name (users.lua -> users)
  local collection = name
  
  -- Insert each fixture record (sorted by key for deterministic order)
  local keys = {}
  for key in pairs(fixture_data) do
    table.insert(keys, key)
  end
  table.sort(keys)

  for _, key in ipairs(keys) do
    db:CreateDocument(collection, fixture_data[key])
  end
end

return TestHelpers
