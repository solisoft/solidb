-- Router module for luaonbeans MVC framework
-- Handles URL routing with pattern matching and parameter extraction

local Router = {}
Router.routes = {}
Router.routes_by_method = { GET = {}, POST = {}, PUT = {}, PATCH = {}, DELETE = {} }

-- Exact-match index: method -> path -> route (for static routes with no params)
Router.exact_routes = { GET = {}, POST = {}, PUT = {}, PATCH = {}, DELETE = {} }

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

  -- Pre-parse handler to avoid per-request string matching
  local controller_name, action_name, fn_handler
  if type(handler) == "function" then
    fn_handler = handler
  elseif type(handler) == "string" then
    controller_name, action_name = handler:match("^([%w_/]+)#([%w_]+)$")
  end

  local route = {
    method = method,
    pattern = pattern,
    regex = regex_pattern,
    params = params,
    handler = handler,
    middleware = middleware,
    controller = controller_name,
    action = action_name,
    fn = fn_handler
  }

  table.insert(Router.routes, route)

  -- Index by method for faster lookup
  if Router.routes_by_method[method] then
    table.insert(Router.routes_by_method[method], route)
  end

  -- Index exact (static) routes for O(1) lookup
  if #params == 0 and Router.exact_routes[method] then
    Router.exact_routes[method][pattern] = route
  end
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

-- Shared empty params table for exact-match routes (no allocations)
local _empty_params = {}

-- Match a request against registered routes
-- Returns: matched_route, params_table or nil
function Router.match(method, path)
  -- Fast path: exact match for static routes (O(1) lookup, zero alloc)
  local exact = Router.exact_routes[method]
  if exact then
    local route = exact[path]
    if route then
      return route, _empty_params
    end
  end

  -- Slow path: regex match only against routes for this method
  local method_routes = Router.routes_by_method[method]
  if not method_routes then
    return nil, nil
  end

  for _, route in ipairs(method_routes) do
    local match_start, match_end, c1, c2, c3, c4, c5 = path:find(route.regex)
    if match_start then
      -- Build params table from captures (avoid captures table allocation)
      local params = {}
      local rparams = route.params
      local n = #rparams
      if n >= 1 then params[rparams[1]] = c1 end
      if n >= 2 then params[rparams[2]] = c2 end
      if n >= 3 then params[rparams[3]] = c3 end
      if n >= 4 then params[rparams[4]] = c4 end
      if n >= 5 then params[rparams[5]] = c5 end
      return route, params
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
-- Returns: route, params (route has .controller, .action, .fn, .middleware pre-parsed)
-- Returns nil, nil if no match
function Router.dispatch(method, path)
  local route, params = Router.match(method, path)
  if not route then
    return nil, nil
  end
  return route, params
end

-- Clear all routes (useful for testing)
function Router.clear()
  Router.routes = {}
  Router.routes_by_method = { GET = {}, POST = {}, PUT = {}, PATCH = {}, DELETE = {} }
  Router.exact_routes = { GET = {}, POST = {}, PUT = {}, PATCH = {}, DELETE = {} }
end

return Router
