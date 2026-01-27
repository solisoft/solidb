-- Controller base class for luaonbeans MVC framework
-- Provides request/response helpers and view rendering

local etlua = require("etlua")
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

-- Create a new controller instance
function Controller:new(request_context)
  local instance = setmetatable({}, self)
  instance.params = request_context.params or {}
  instance.request = request_context.request or {}
  instance.middleware_data = request_context.middleware_data or {} -- Data from middleware
  instance.response = {
    status = 200,
    headers = {},
    body = ""
  }
  instance.layout = "application" -- Default layout
  instance.variant = nil -- View variant (e.g., "iphone", "tablet")
  instance._rendered = false
  instance.cookies = {}
  instance.session = {}
  instance.flash = {}

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

-- Render a view with optional layout
function Controller:render(template, locals, options)
  if not self.response then
      error("self.response is nil")
  end
  if self._rendered then
    error("Double render detected! Can only render once per request.")
  end

  options = options or {}
  locals = locals or {}

  -- Merge controller instance variables into locals
  for k, v in pairs(self) do
    if type(v) ~= "function" and k:sub(1, 1) ~= "_" and not locals[k] then
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

  -- Merge controller instance variables into locals
  for k, v in pairs(self) do
    if type(v) ~= "function" and k:sub(1, 1) ~= "_" and not locals[k] then
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


-- Render JSON response
function Controller:json(data, status)
  if self._rendered then
    error("Double render detected! Can only render once per request.")
  end

  self.response.status = status or 200
  self.response.headers["Content-Type"] = "application/json"
  self.response.body = EncodeJson(data)
  self._rendered = true

  return self.response.body
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
  self.response.headers["Content-Type"] = "text/plain; charset=utf-8"
  self.response.body = content
  self._rendered = true

  return self.response.body
end

-- Render HTML directly (without template)
function Controller:html(content, status)
  if self._rendered then
    error("Double render detected! Can only render once per request.")
  end

  self.response.status = status or 200
  self.response.headers["Content-Type"] = "text/html; charset=utf-8"
  self.response.body = content
  self._rendered = true

  return self.response.body
end

-- Redirect to another URL
function Controller:redirect(url, status)
  if self._rendered then
    error("Double render detected! Can only render once per request.")
  end

  self.response.status = status or 302
  self.response.headers["Location"] = url
  self._rendered = true
end

-- Alias for redirect
function Controller:redirect_to(url, status)
  return self:redirect(url, status)
end

-- Set cookie
function Controller:set_cookie(name, value, options)
  self.cookies[name] = { value = value, options = options }
  return self
end

-- Set session
function Controller:set_session(data, ttl)
  self.session.data = data or {}
  self.session.ttl = ttl
  return self
end

-- Set flash
function Controller:set_flash(name, value)
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
  self.response.headers[name] = value
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
  SetStatus(self.response.status)

  for name, value in pairs(self.response.headers) do
    SetHeader(name, value)
  end

  for name, cookie in pairs(self.cookies) do
    SetCookie(name, cookie.value, cookie.options)
  end

  if self.session.data then
    SetSession(self.session.data, self.session.ttl)
  end

  for name, flash in pairs(self.flash) do
    SetFlash(name, flash)
  end

  if self.response.body and self.response.body ~= "" then
    Write(self.response.body)
  end
end

return Controller
