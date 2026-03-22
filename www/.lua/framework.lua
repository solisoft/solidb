-- Framework request handling logic
-- This file is reloadable during development

local router = require("router")
local view = require("view")
local Controller = require("controller")
local Middleware = require("middleware")

local Framework = {}

-- Helper to parse nested parameters like post[title] or user[profile][name]
function Framework.parse_params(params)
  local result = {}
  for key, value in pairs(params) do
    local parts = {}
    local root = key:match("^([^%[]+)")
    if root then
      table.insert(parts, root)
      for part in key:gmatch("%[([^%]]*)%]") do
        table.insert(parts, part)
      end
    end

    if #parts > 1 then
      local curr = result
      for i = 1, #parts - 1 do
        local part = parts[i]
        if part == "" then part = #curr + 1 end
        if type(curr[part]) ~= "table" then curr[part] = {} end
        curr = curr[part]
      end
      local last_part = parts[#parts]
      if last_part == "" then
        table.insert(curr, value)
      else
        curr[last_part] = value
      end
    else
      result[key] = value
    end
  end
  return result
end

-- Helper to parse multipart/form-data
function Framework.parse_multipart(body, boundary)
  local result = {}
  local sep = "--" .. boundary
  
  for part in body:gmatch("(.-)" .. sep) do
    if part ~= "" and part ~= "--" then
      local name = part:match('name="([^"]+)"')
      local filename = part:match('filename="([^"]+)"')
      
      -- Extract content after headers (double newline)
      local content = part:match("\r\n\r\n(.*)$") or part:match("\n\n(.*)$")
      
      if name and content then
        -- Remove trailing newlines from content
        content = content:gsub("\r\n$", ""):gsub("\n$", "")
        
        if filename then
          -- File field: create object with name and content
          result[name] = {
            name = filename,
            content = content
          }
        else
          -- Regular field: just store the value
          result[name] = content
        end
      end
    end
  end
  return result
end

-- Simple URL decoder fallback
function Framework.url_decode(str)
  str = str:gsub("+", " ")
  str = str:gsub("%%(%x%x)", function(h) return string.char(tonumber(h, 16)) end)
  return str
end

-- Helper to parse x-www-form-urlencoded
function Framework.parse_url_encoded(body)
  local result = {}
  for pair in body:gmatch("([^&]+)") do
    local key, value = pair:match("([^=]+)=(.*)")
    if key and value then
      result[Framework.url_decode(key)] = Framework.url_decode(value)
    end
  end
  return result
end

-- Helper to parse request body based on content type
function Framework.parse_request_body()
  local body = GetBody()
  if not body or body == "" then return {} end

  local content_type = GetHeader("Content-Type") or ""
  local ct_lower = content_type:lower()

  if ct_lower:find("application/json", 1, true) then
    local ok, data = pcall(DecodeJson, body)
    return ok and data or {}
  elseif ct_lower:find("application/x-www-form-urlencoded", 1, true) then
    -- Try built-in first
    local ok, data = pcall(function() return DecodeUrlEncoded(body) end)
    if ok and data and next(data) then
      return data
    end
    -- Fallback
    return Framework.parse_url_encoded(body)
  elseif ct_lower:find("multipart/form-data", 1, true) then
    local boundary = content_type:match("boundary=(%S+)")
    if boundary then
      return Framework.parse_multipart(body, boundary)
    end
  end

  return {}
end

-- Controller cache (managed by .init.lua via clear_caches)
local controllers = {}

-- Load a controller by name (cached, DB-first, then filesystem)
local function load_controller(name)
  local cached = controllers[name]
  if cached then return cached end

  -- Try DB first
  local DbLoader = require("dbloader")
  local db_controller = DbLoader.load_controller(name)
  if db_controller then
    controllers[name] = db_controller
    return db_controller
  end

  -- Fallback to filesystem
  local ok, controller = pcall(require, name .. "_controller")
  if ok then
    controllers[name] = controller
    return controller
  end
  return nil, "Controller not found: " .. name .. " (" .. tostring(controller) .. ")"
end

-- Reusable request table (avoid allocation per request)
local _reuse_request = { method = nil, path = nil }

-- Cache global middleware state at module load time (framework.lua is reloaded in dev mode)
local _mw_stats = Middleware.stats()
local _has_any_global_mw = _mw_stats.before > 0 or _mw_stats.after > 0

function Framework.handle_request(req_path)
  local method = GetMethod()
  local path = req_path or GetPath()

  -- Handle method override for forms
  if method == "POST" then
    local override = GetParam("_method")
    if override then
      method = override:upper()
    end
  end

  -- Match route FIRST (before any table allocation)
  local route, params = router.match(method, path)

  if not route then
    if Route() then
      return
    end
    SetStatus(404)
    SetHeader("Content-Type", "text/html; charset=utf-8")
    Write(view.render("errors/404", {}, { layout = false }))
    return
  end

  -- Create middleware context only when needed (lazy)
  local ctx
  if _has_any_global_mw or route.middleware then
    ctx = Middleware.create_context(method, path)
    if not Middleware.run_before(ctx) then
      Middleware.send_response(ctx)
      return
    end
    if route.middleware then
      ctx.route = route
      if not Middleware.run_route(ctx, route.middleware) then
        Middleware.send_response(ctx)
        return
      end
    end
  end

  -- Handle function handlers (pre-parsed on route object, or parse now)
  local route_fn = route.fn
  local route_controller = route.controller
  local route_action = route.action

  -- If route wasn't pre-parsed (old router version), parse handler now
  if not route_fn and not route_controller and route.handler then
    if type(route.handler) == "function" then
      route_fn = route.handler
    else
      route_controller, route_action = route.handler:match("^([%w_/]+)#([%w_]+)$")
    end
  end

  if route_fn then
    local response = route_fn(params, ctx)
    if response then
      SetStatus(response.status or 200)
      if response.headers then
        for k, v in pairs(response.headers) do
          SetHeader(k, v)
        end
      end
      if response.json then
        SetHeader("Content-Type", "application/json")
        Write(EncodeJson(response.json))
      elseif response.body then
        Write(response.body)
      end
    end
    if ctx then Middleware.run_after(ctx) end
    return
  end

  -- Handle controller#action
  local controller_class, err = load_controller(route_controller)
  if not controller_class then
    SetStatus(500)
    SetHeader("Content-Type", "text/plain")
    Write("Error: " .. tostring(err))
    return
  end

  -- Build parameter table
  local raw_params = params

  -- Merge Query String params (only if there are any)
  local qkey, qval = GetParam(0)
  if qkey then
    -- Copy params if needed (exact-match routes share a frozen empty table)
    if not next(raw_params) then
      raw_params = { [qkey] = qval }
    else
      raw_params[qkey] = qval
    end
    local i = 1
    while true do
      qkey, qval = GetParam(i)
      if qkey == nil then break end
      raw_params[qkey] = qval
      i = i + 1
    end
  end

  -- Merge POST/PUT/PATCH body params (skip for GET/DELETE)
  if method == "POST" or method == "PUT" or method == "PATCH" then
    local body_params = Framework.parse_request_body()
    for k, v in pairs(body_params) do
      raw_params[k] = v
    end
  end

  -- Parse nested parameters only if there are bracket keys
  local has_nested = false
  for k in pairs(raw_params) do
    if type(k) == "string" and k:find("[", 1, true) then
      has_nested = true
      break
    end
  end
  if has_nested then
    raw_params = Framework.parse_params(raw_params)
  end

  -- Reuse request table instead of allocating
  _reuse_request.method = method
  _reuse_request.path = path

  -- Create controller instance (minimal context)
  local ctrl = controller_class:new(raw_params, _reuse_request, ctx and ctx.data)

  -- Run before action
  ctrl:before_action()
  if ctrl._rendered then
    ctrl:send_response()
    if ctx then Middleware.run_after(ctx) end
    return
  end

  -- Run the action (pcall without closure allocation)
  local action_fn = ctrl[route_action]
  if not action_fn then
    SetStatus(500)
    SetHeader("Content-Type", "text/plain")
    Write("Error: Action '" .. route_action .. "' not found in " .. route_controller .. " controller")
    return
  end

  local ok, action_err = pcall(action_fn, ctrl)

  if not ok then
    SetStatus(500)
    SetHeader("Content-Type", "text/html; charset=utf-8")
    Write("<h1>Error</h1><pre>" .. EscapeHtml(tostring(action_err)) .. "</pre>")
    if ctx then Middleware.run_after(ctx) end
    return
  end

  -- Run after action
  ctrl:after_action()

  if ctx then Middleware.run_after(ctx) end

  -- Send response
  ctrl:send_response()
end

return Framework
