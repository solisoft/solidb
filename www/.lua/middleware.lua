-- Middleware system for Luaonbeans
-- Provides global and route-scoped middleware with next() pattern

local Middleware = {}

-- Registry
local global_before = {}  -- Run before route dispatch
local global_after = {}   -- Run after controller (reverse order)
local named_middleware = {} -- Named middleware for route-scoped use

-- ============================================================================
-- Context Object
-- ============================================================================

---Create a new middleware context
---@param method string HTTP method
---@param path string Request path
---@return table Context object
function Middleware.create_context(method, path)
  local ctx = {
    -- Request info
    method = method,
    path = path,
    params = {},
    headers = {},
    body = nil,

    -- Response (set by middleware to halt)
    status = nil,
    response_headers = {},
    response_body = nil,
    halted = false,

    -- Shared data between middleware and controller
    data = {},

    -- Route info (set after dispatch)
    route = nil,
    controller = nil,
    action = nil
  }

  -- Helper methods
  function ctx:get_header(name)
    return GetHeader(name)
  end

  function ctx:set_header(name, value)
    self.response_headers[name] = value
  end

  function ctx:halt(status, body)
    self.halted = true
    self.status = status or 200
    self.response_body = body or ""
  end

  function ctx:redirect(url, status)
    self.halted = true
    self.status = status or 302
    self.response_headers["Location"] = url
    self.response_body = ""
  end

  function ctx:json(data, status)
    self.halted = true
    self.status = status or 200
    self.response_headers["Content-Type"] = "application/json"
    self.response_body = EncodeJson(data)
  end

  function ctx:text(content, status)
    self.halted = true
    self.status = status or 200
    self.response_headers["Content-Type"] = "text/plain"
    self.response_body = content
  end

  function ctx:html(content, status)
    self.halted = true
    self.status = status or 200
    self.response_headers["Content-Type"] = "text/html; charset=utf-8"
    self.response_body = content
  end

  return ctx
end

-- ============================================================================
-- Middleware Registration
-- ============================================================================

---Register global before middleware (runs on all requests)
---@param name_or_fn string|function Middleware name or function
function Middleware.use(name_or_fn)
  local fn = Middleware.resolve(name_or_fn)
  if fn then
    table.insert(global_before, fn)
  end
end

---Register global after middleware (runs after controller)
---@param name_or_fn string|function Middleware name or function
function Middleware.after(name_or_fn)
  local fn = Middleware.resolve(name_or_fn)
  if fn then
    table.insert(global_after, fn)
  end
end

---Register a named middleware for route-scoped use
---@param name string Middleware name
---@param fn function Middleware function
function Middleware.register(name, fn)
  named_middleware[name] = fn
end

---Load middleware from .lua/middleware/ directory
---@param name string Middleware name (without .lua extension)
---@return function|nil Middleware function or nil
function Middleware.load(name)
  local ok, middleware = pcall(require, "middleware/" .. name)
  if ok and type(middleware) == "function" then
    return middleware
  end
  return nil
end

---Resolve middleware by name or return function as-is
---@param name_or_fn string|function
---@return function|nil
function Middleware.resolve(name_or_fn)
  if type(name_or_fn) == "function" then
    return name_or_fn
  end

  if type(name_or_fn) == "string" then
    -- Check named registry first
    if named_middleware[name_or_fn] then
      return named_middleware[name_or_fn]
    end
    -- Try to load from file
    local loaded = Middleware.load(name_or_fn)
    if loaded then
      named_middleware[name_or_fn] = loaded  -- Cache it
      return loaded
    end
  end

  return nil
end

-- ============================================================================
-- Middleware Chain Runner
-- ============================================================================

---Build a chain runner for middleware array
---@param middleware table Array of middleware functions
---@param final_fn function|nil Final function to call after all middleware
---@return function Runner function(ctx)
local function build_chain(middleware, final_fn)
  return function(ctx)
    local index = 0

    local function next()
      index = index + 1

      if ctx.halted then
        return  -- Stop if halted
      end

      if index <= #middleware then
        local mw = middleware[index]
        if mw then
          mw(ctx, next)
        else
          next()  -- Skip nil middleware
        end
      elseif final_fn then
        final_fn(ctx)
      end
    end

    next()
  end
end

---Run global before middleware
---@param ctx table Context object
---@return boolean True if should continue, false if halted
function Middleware.run_before(ctx)
  local chain = build_chain(global_before)
  chain(ctx)
  return not ctx.halted
end

---Run route-specific middleware
---@param ctx table Context object
---@param route_middleware table Array of middleware names/functions
---@return boolean True if should continue, false if halted
function Middleware.run_route(ctx, route_middleware)
  if not route_middleware or #route_middleware == 0 then
    return true
  end

  local resolved = {}
  for _, name_or_fn in ipairs(route_middleware) do
    local fn = Middleware.resolve(name_or_fn)
    if fn then
      table.insert(resolved, fn)
    else
      -- Debug: middleware not found - log to file
      local f = io.open("debug.log", "a")
      if f then
        f:write(string.format("[%s] Middleware '%s' not found in registry\n", os.date(), tostring(name_or_fn)))
        f:close()
      end
    end
  end

  local chain = build_chain(resolved)
  chain(ctx)
  return not ctx.halted
end

---Run global after middleware (in reverse order)
---@param ctx table Context object
function Middleware.run_after(ctx)
  -- Run in reverse order
  local reversed = {}
  for i = #global_after, 1, -1 do
    table.insert(reversed, global_after[i])
  end

  local chain = build_chain(reversed)
  chain(ctx)
end

-- ============================================================================
-- Response Helpers
-- ============================================================================

---Send halted response (called by framework when middleware halts)
---@param ctx table Context object
function Middleware.send_response(ctx)
  if not ctx.halted then
    return false
  end

  -- Set status
  SetStatus(ctx.status or 200)

  -- Set headers
  for name, value in pairs(ctx.response_headers) do
    SetHeader(name, value)
  end

  -- Write body
  if ctx.response_body then
    Write(ctx.response_body)
  end

  return true
end

-- ============================================================================
-- Utility Functions
-- ============================================================================

---Clear all middleware (useful for testing)
function Middleware.clear()
  global_before = {}
  global_after = {}
  named_middleware = {}
end

---Get count of registered middleware
---@return table Counts { before, after, named }
function Middleware.stats()
  local named_count = 0
  for _ in pairs(named_middleware) do
    named_count = named_count + 1
  end
  return {
    before = #global_before,
    after = #global_after,
    named = named_count
  }
end

---List all registered middleware names
---@return table { before = {...}, after = {...}, named = {...} }
function Middleware.list()
  local named_list = {}
  for name in pairs(named_middleware) do
    table.insert(named_list, name)
  end
  table.sort(named_list)

  return {
    before = #global_before,
    after = #global_after,
    named = named_list
  }
end

return Middleware
