-- Router module for luaonbeans MVC framework
-- Handles URL routing with pattern matching and parameter extraction

local Router = {}
Router.routes = {}

-- HTTP method helpers
local METHODS = { "GET", "POST", "PUT", "PATCH", "DELETE" }

-- Convert route pattern to regex pattern and extract param names
-- e.g., "/users/:id/posts/:post_id" -> "^/users/([^/]+)/posts/([^/]+)$", {"id", "post_id"}
-- Supports wildcard: "/files/*path" -> "^/files/(.+)$", {"path"}
local function compile_pattern(pattern)
  local params = {}

  -- Process pattern character by character to extract params in order
  local i = 1
  while i <= #pattern do
    local c = pattern:sub(i, i)
    if c == ":" or c == "*" then
      -- Extract param name
      local name_start = i + 1
      local name_end = name_start
      while name_end <= #pattern and pattern:sub(name_end, name_end):match("[%w_]") do
        name_end = name_end + 1
      end
      local name = pattern:sub(name_start, name_end - 1)
      if #name > 0 then
        table.insert(params, name)
      end
      i = name_end
    else
      i = i + 1
    end
  end

  -- Build regex pattern
  -- First replace splat (*name) with marker
  local temp_pattern = pattern:gsub("%*[%w_]+", "\0SPLAT\0")
  -- Then replace params (:name) with marker
  temp_pattern = temp_pattern:gsub(":[%w_]+", "\0PARAM\0")
  -- Escape special Lua pattern chars
  temp_pattern = temp_pattern:gsub("%-", "%%-")
  temp_pattern = temp_pattern:gsub("%.", "%%.")
  -- Replace markers with capture groups
  temp_pattern = temp_pattern:gsub("\0SPLAT\0", "(.+)")
  temp_pattern = temp_pattern:gsub("\0PARAM\0", "([^/]+)")

  local regex_pattern = "^" .. temp_pattern .. "$"
  return regex_pattern, params
end

-- Scope stack for nested routes
Router.scope_stack = {}

-- Middleware stack for nested scopes
Router.middleware_stack = {}

-- Create a scope with a path prefix and optional middleware
-- Usage: Router.scope("/admin", fn)
-- Usage: Router.scope("/admin", { middleware = { "auth" } }, fn)
function Router.scope(path, options_or_fn, fn)
  local options = {}
  if type(options_or_fn) == "function" then
    fn = options_or_fn
  else
    options = options_or_fn or {}
  end

  table.insert(Router.scope_stack, path)

  -- Push middleware to stack if provided
  if options.middleware then
    table.insert(Router.middleware_stack, options.middleware)
  end

  fn()

  -- Pop middleware from stack
  if options.middleware then
    table.remove(Router.middleware_stack)
  end

  table.remove(Router.scope_stack)
end

-- Helper to join URL paths cleanly
local function join_paths(p1, p2)
  if p1 == "" then return p2 end
  if p2 == "" then return p1 end
  
  local s1 = p1:sub(-1) == "/"
  local s2 = p2:sub(1,1) == "/"
  
  if s1 and s2 then
    return p1 .. p2:sub(2)
  elseif not s1 and not s2 then
    return p1 .. "/" .. p2
  else
    return p1 .. p2
  end
end

-- Collect middleware from scope stack and route options
local function collect_middleware(route_middleware)
  local middleware = {}

  -- First, add all middleware from scope stack
  for _, scope_mw in ipairs(Router.middleware_stack) do
    for _, mw in ipairs(scope_mw) do
      table.insert(middleware, mw)
    end
  end

  -- Then add route-specific middleware
  if route_middleware then
    for _, mw in ipairs(route_middleware) do
      table.insert(middleware, mw)
    end
  end

  return #middleware > 0 and middleware or nil
end

-- Add a route
-- Usage: add_route("GET", "/path", "controller#action")
-- Usage: add_route("GET", "/path", "controller#action", { middleware = { "auth" } })
local function add_route(method, pattern, handler, options)
  options = options or {}

  -- Prepend scope prefix
  if #Router.scope_stack > 0 then
    local prefix = table.concat(Router.scope_stack, "")
    pattern = join_paths(prefix, pattern)
  end

  local regex_pattern, params = compile_pattern(pattern)
  local middleware = collect_middleware(options.middleware)

  table.insert(Router.routes, {
    method = method,
    pattern = pattern,
    regex = regex_pattern,
    params = params,
    handler = handler,
    middleware = middleware
  })
end

-- Route definition methods
-- All methods support optional third parameter for options: { middleware = { "auth" } }
function Router.get(pattern, handler, options)
  add_route("GET", pattern, handler, options)
end

function Router.post(pattern, handler, options)
  add_route("POST", pattern, handler, options)
end

function Router.put(pattern, handler, options)
  add_route("PUT", pattern, handler, options)
end

function Router.patch(pattern, handler, options)
  add_route("PATCH", pattern, handler, options)
end

function Router.delete(pattern, handler, options)
  add_route("DELETE", pattern, handler, options)
end

-- Resource helper - creates RESTful routes for a resource
-- Supports nesting via callback function
function Router.resources(name, options, fn)
  if type(options) == "function" then
    fn = options
    options = {}
  end
  options = options or {}
  
  local controller = options.controller or name
  local path = options.path or "/" .. name
  local id_param = options.id or "id"

  -- Execute callback first to ensure custom routes (e.g. collection routes) 
  -- take precedence over wildcard routes like :id
  if fn then
     -- Determine parent param name for nested scope (e.g. users -> user_id)
     local singular = name
     if singular:sub(-1) == "s" then singular = singular:sub(1, -2) end
     
     local parent_param
     if options.id then
        parent_param = options.id
     else
        parent_param = singular .. "_id"
     end
     
     Router.scope(path .. "/:" .. parent_param, fn)
  end

  -- Define all standard actions
  local actions = {
    index   = { method="GET",    path=path,                     action="#index"   },
    new     = { method="GET",    path=path.."/new",             action="#new_resource" },
    create  = { method="POST",   path=path,                     action="#create"  },
    show    = { method="GET",    path=path.."/:"..id_param,     action="#show"    },
    edit    = { method="GET",    path=path.."/:"..id_param.."/edit", action="#edit" },
    update  = { method="PUT",    path=path.."/:"..id_param,     action="#update"  },
    update2 = { method="PATCH",  path=path.."/:"..id_param,     action="#update"  }, -- patch alias
    destroy = { method="DELETE", path=path.."/:"..id_param,     action="#destroy" }
  }

  -- Filter actions
  local function should_generate(action_name)
    if options.only then
      -- check if in only list
      for _, a in ipairs(options.only) do
        if a == action_name then return true end
      end
      return false
    elseif options.except then
      -- check if in except list
      for _, a in ipairs(options.except) do
        if a == action_name then return false end
      end
      return true
    end
    return true
  end

  if should_generate("index")   then Router.get(actions.index.path, controller..actions.index.action) end
  if should_generate("new")     then Router.get(actions.new.path, controller..actions.new.action) end
  if should_generate("create")  then Router.post(actions.create.path, controller..actions.create.action) end
  if should_generate("show")    then Router.get(actions.show.path, controller..actions.show.action) end
  if should_generate("edit")    then Router.get(actions.edit.path, controller..actions.edit.action) end
  if should_generate("update")  then 
     Router.put(actions.update.path, controller..actions.update.action)
     Router.patch(actions.update2.path, controller..actions.update2.action)
  end
  if should_generate("destroy") then Router.delete(actions.destroy.path, controller..actions.destroy.action) end
end

-- Define collection routes (strips the last resource ID from scope)
function Router.collection(fn)
  local current = Router.scope_stack[#Router.scope_stack]
  if current and current:match("/:[%w_]+$") then
     -- Found ID param at end. Strip it temporarily.
     local new_scope = current:gsub("/:[%w_]+$", "")
     Router.scope_stack[#Router.scope_stack] = new_scope
     fn()
     Router.scope_stack[#Router.scope_stack] = current -- restore
  else
     -- Not in a resource scope? Just run.
     fn()
  end
end

-- Define member routes (explicit alias, assumes member scope)
function Router.member(fn)
  fn()
end

-- Match a request against registered routes
-- Returns: matched_route, params_table or nil
function Router.match(method, path)
  for _, route in ipairs(Router.routes) do
    if route.method == method then
      -- For routes without params, match returns the full match or nil
      -- For routes with params, match returns captures
      local match_start, match_end, c1, c2, c3, c4, c5 = path:find(route.regex)
      if match_start then
        -- Build params table from captures
        local params = {}
        local captures = {c1, c2, c3, c4, c5}
        for i, param_name in ipairs(route.params) do
          params[param_name] = captures[i]
        end
        return route, params
      end
    end
  end
  return nil, nil
end

-- Parse handler string "controller#action" into controller name and action
-- Supports nested controllers like "dashboard/collections#index"
function Router.parse_handler(handler)
  if type(handler) == "function" then
    return nil, nil, handler
  end
  local controller, action = handler:match("^([%w_/]+)#([%w_]+)$")
  return controller, action, nil
end

-- Dispatch a request to the appropriate controller/action
function Router.dispatch(method, path)
  local route, params = Router.match(method, path)

  if not route then
    return false, nil
  end

  local controller_name, action, fn = Router.parse_handler(route.handler)

  if fn then
    -- Direct function handler
    return true, { fn = fn, params = params, middleware = route.middleware }
  else
    -- Controller#action handler
    return true, {
      controller = controller_name,
      action = action,
      params = params,
      middleware = route.middleware
    }
  end
end

-- Clear all routes (useful for testing)
function Router.clear()
  Router.routes = {}
end

return Router
