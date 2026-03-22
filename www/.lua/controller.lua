-- Controller base class for luaonbeans MVC framework
-- Provides request/response helpers and view rendering

local view = require("view")

local Controller = {}
Controller.__index = Controller

-- Create a new controller class that extends Controller
function Controller:extend()
  local cls = {}
  cls.__index = cls
  setmetatable(cls, {
    __index = self,
    __call = function(c, ...)
      return c:new(...)
    end
  })
  return cls
end

-- Reusable empty table for avoiding allocations
local _empty = {}

-- Shared empty headers table (copy-on-write: replaced on first header set)
local _empty_headers = {}

-- Response defaults (shared via metatable, fields written on instance only when changed)
local _response_defaults = { status = 200, headers = _empty_headers, body = "" }
local _response_mt = { __index = _response_defaults }

-- Prototype defaults (found via __index chain, no per-instance allocation)
Controller.layout = "application"
Controller.cookies = _empty
Controller.session = _empty
Controller.flash = _empty
Controller.middleware_data = _empty

-- Create a new controller instance
-- Accepts either (request_context_table) or (params, request, middleware_data) for performance
function Controller:new(params_or_ctx, request, middleware_data)
  local instance = setmetatable({}, self)

  if request ~= nil then
    -- Fast path: positional arguments from framework
    instance.params = params_or_ctx
    instance.request = request
    if middleware_data then instance.middleware_data = middleware_data end
  else
    -- Legacy path: request_context table
    local rc = params_or_ctx or _empty
    instance.params = rc.params or _empty
    instance.request = rc.request or _empty
    if rc.middleware_data then instance.middleware_data = rc.middleware_data end
  end

  instance.response = setmetatable({}, _response_mt)

  return instance
end

-- Merge query params into controller params
function Controller:_merge_query_params()
  -- Get all query parameters from redbean
  local i = 0
  while true do
    local key, value = GetParam(i)
    if key == nil then break end
    if not self.params[key] then
      self.params[key] = value
    end
    i = i + 1
  end
end

-- Keys to skip when merging controller vars into view locals
local _skip_merge = {
  response = true, layout = true, variant = true,
  cookies = true, session = true, flash = true,
  request = true, middleware_data = true
}

-- Render a view with optional layout
function Controller:render(template, locals, options)
  if self._rendered then
    error("Double render detected! Can only render once per request.")
  end

  options = options or {}
  locals = locals or {}

  -- Merge controller instance variables into locals (skip internals)
  for k, v in pairs(self) do
    if not locals[k] and not _skip_merge[k] and type(v) ~= "function" and k:sub(1, 1) ~= "_" then
      locals[k] = v
    end
  end

  -- Ensure params are available in the view
  locals.params = self.params

  -- Determine layout
  local layout = options.layout
  if layout == nil then
    layout = self.layout
  end

  -- Determine variant
  local variant = options.variant
  if variant == nil then
    variant = self.variant
  end

  -- Render the view
  local content = view.render(template, locals, { layout = layout, variant = variant })

  self.response.body = content
  self._rendered = true

  return content
end

-- Render a partial (no layout) - useful for HTMX responses
function Controller:render_partial(template, locals, options)
  if self._rendered then
    error("Double render detected! Can only render once per request.")
  end

  locals = locals or {}
  options = options or {}

  -- Merge controller instance variables into locals (skip internals)
  for k, v in pairs(self) do
    if not locals[k] and not _skip_merge[k] and type(v) ~= "function" and k:sub(1, 1) ~= "_" then
      locals[k] = v
    end
  end

  -- Ensure params are available
  locals.params = self.params

  -- Determine variant
  local variant = options.variant
  if variant == nil then
    variant = self.variant
  end

  -- Render without layout
  local content = view.render(template, locals, { layout = false, variant = variant })

  self.response.body = content
  self._rendered = true

  return content
end

-- Check if request is from HTMX
function Controller:is_htmx_request()
  -- Use pcall for test environment safety (GetHeader may not be available)
  -- Try multiple case variations as HTTP headers are case-insensitive
  local ok, value = pcall(function()
    return GetHeader("HX-Request") or GetHeader("hx-request") or GetHeader("Hx-Request")
  end)
  return ok and value == "true"
end


-- Auto-render: use partial for HTMX, full layout otherwise
function Controller:smart_render(template, locals, options)
  if self:is_htmx_request() then
    return self:render_partial(template, locals)
  else
    return self:render(template, locals, options)
  end
end


-- Get-or-create writable headers table (copy-on-write)
local function ensure_headers(self)
  local h = self.response.headers
  if h == _empty_headers then
    h = {}
    self.response.headers = h
  end
  return h
end

-- Render JSON response
function Controller:json(data, status)
  if self._rendered then
    error("Double render detected! Can only render once per request.")
  end

  local body = EncodeJson(data)
  self.response.status = status or 200
  self.response._ct = "application/json"
  self.response.body = body
  self._rendered = true

  return body
end

-- Alias for json
function Controller:render_json(data, status)
  return self:json(data, status)
end

-- Render plain text
function Controller:text(content, status)
  if self._rendered then
    error("Double render detected! Can only render once per request.")
  end

  self.response.status = status or 200
  self.response._ct = "text/plain; charset=utf-8"
  self.response.body = content
  self._rendered = true

  return content
end

-- Render HTML directly (without template)
function Controller:html(content, status)
  if self._rendered then
    error("Double render detected! Can only render once per request.")
  end

  self.response.status = status or 200
  self.response._ct = "text/html; charset=utf-8"
  self.response.body = content
  self._rendered = true

  return content
end

-- Redirect to another URL
function Controller:redirect(url, status)
  if self._rendered then
    error("Double render detected! Can only render once per request.")
  end

  self.response.status = status or 302
  ensure_headers(self)["Location"] = url
  self._rendered = true
end

-- Alias for redirect
function Controller:redirect_to(url, status)
  return self:redirect(url, status)
end

-- Set cookie (copy-on-write: allocate own table on first use)
function Controller:set_cookie(name, value, options)
  if self.cookies == _empty then self.cookies = {} end
  self.cookies[name] = { value = value, options = options }
  return self
end

-- Set session (copy-on-write)
function Controller:set_session(data, ttl)
  if self.session == _empty then self.session = {} end
  self.session.data = data or {}
  self.session.ttl = ttl
  return self
end

-- Set flash (copy-on-write)
function Controller:set_flash(name, value)
  if self.flash == _empty then self.flash = {} end
  self.flash[name] = value
  return self
end

-- Set response status code
function Controller:status(code)
  self.response.status = code
  return self
end

-- Set response header
function Controller:set_header(name, value)
  ensure_headers(self)[name] = value
  return self
end

-- Check if response has been rendered
function Controller:rendered()
  return self._rendered
end

-- Get a specific parameter
function Controller:param(name, default)
  return self.params[name] or default
end

-- Get request method
function Controller:request_method()
  return GetMethod()
end

-- Check if request is a specific method
function Controller:is_get()
  return GetMethod() == "GET"
end

function Controller:is_post()
  return GetMethod() == "POST"
end

function Controller:is_put()
  return GetMethod() == "PUT"
end

function Controller:is_delete()
  return GetMethod() == "DELETE"
end

-- Before action filters (to be overridden in subclasses)
function Controller:before_action()
  -- Override in subclass
end

-- After action filters (to be overridden in subclasses)
function Controller:after_action()
  -- Override in subclass
end

-- Send the response to the client (called by framework)
function Controller:send_response()
  local resp = self.response
  SetStatus(resp.status)

  -- Fast path: Content-Type set via _ct shortcut (no headers table allocated)
  local ct = rawget(resp, "_ct")
  if ct then
    SetHeader("Content-Type", ct)
  end

  -- Other headers (skip if using shared default via metatable)
  local h = rawget(resp, "headers")
  if h then
    for name, value in pairs(h) do
      SetHeader(name, value)
    end
  end

  -- Cookies, session, flash: skip when using prototype defaults
  local cookies = rawget(self, "cookies")
  if cookies then
    for name, cookie in pairs(cookies) do
      SetCookie(name, cookie.value, cookie.options)
    end
  end

  local session = rawget(self, "session")
  if session and session.data then
    SetSession(session.data, session.ttl)
  end

  local flash = rawget(self, "flash")
  if flash then
    for name, fl in pairs(flash) do
      SetFlash(name, fl)
    end
  end

  local body = resp.body
  if body and body ~= "" then
    Write(body)
  end
end

return Controller
