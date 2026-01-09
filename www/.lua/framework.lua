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
  -- Standardize boundary
  local sep = "--" .. boundary
  
  -- Split by boundary using pattern matching
  for part in body:gmatch("(.-)" .. sep) do
    if part ~= "" and part ~= "--" then
      local name = part:match('name="([^"]+)"')
      local value = part:match("\r\n\r\n(.-)\r\n$") or part:match("\n\n(.-)\n$")
      if name and value then
        result[name] = value
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

-- Load a controller by name (DB-first, then filesystem)
local function load_controller(name)
  -- Try DB first
  local DbLoader = require("dbloader")
  local db_controller = DbLoader.load_controller(name)
  if db_controller then
    return db_controller
  end

  -- Fallback to filesystem
  local ok, controller = pcall(require, name .. "_controller")
  if ok then
    return controller
  end
  return nil, "Controller not found: " .. name .. " (" .. tostring(controller) .. ")"
end

function Framework.handle_request()
  local method = GetMethod()
  local path = GetPath()

  -- 1. Static file handling moved to .init.lua for performance

  -- 2. Handle method override for forms
  if method == "POST" then
    local override = GetParam("_method")
    if override then
      method = override:upper()
    end
  end

  -- 3. Create middleware context
  local ctx = Middleware.create_context(method, path)

  -- 4. Run global before middleware
  if not Middleware.run_before(ctx) then
    Middleware.send_response(ctx)
    return
  end

  -- 5. Try to match a route
  local matched, result = router.dispatch(method, path)

  if not matched then
    -- No route matched, try redbean's built-in routing
    if Route() then
      return
    end
    -- 404 Not Found
    SetStatus(404)
    SetHeader("Content-Type", "text/html; charset=utf-8")
    local content = view.render("errors/404", {}, { layout = false })
    Write(content)
    return
  end

  -- 6. Run route-specific middleware
  if result.middleware then
    ctx.route = result
    if not Middleware.run_route(ctx, result.middleware) then
      Middleware.send_response(ctx)
      return
    end
  end

  -- Handle function handlers
  if result.fn then
    local response = result.fn(result.params, ctx)
    if response then
      SetStatus(response.status or 200)
      
      -- Set headers if provided
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
    -- Run global after middleware
    Middleware.run_after(ctx)
    return
  end

  -- Handle controller#action
  local controller_class, err = load_controller(result.controller)
  if not controller_class then
    SetStatus(500)
    SetHeader("Content-Type", "text/plain")
    Write("Error: " .. tostring(err))
    return
  end

  -- Build parameter table
  local raw_params = result.params or {}

  -- Merge Query String params
  local i = 0
  while true do
    local key, value = GetParam(i)
    if key == nil then break end
    raw_params[key] = value
    i = i + 1
  end

  -- Also try direct GetParam by name for common query params
  local channel_param = GetParam("channel")
  if channel_param then
    raw_params["channel"] = channel_param
  end

  -- Merge POST body params
  local body_params = Framework.parse_request_body()
  for k, v in pairs(body_params) do
    raw_params[k] = v
  end

  -- Parse nested parameters
  local nested_params = Framework.parse_params(raw_params)

  -- Build request context (include middleware data)
  local request_context = {
    params = nested_params,
    request = {
      method = method,
      path = path
    },
    middleware_data = ctx.data  -- Pass middleware data to controller
  }

  -- Create controller instance
  local controller = controller_class:new(request_context)

  -- Run before action
  controller:before_action()
  if controller:rendered() then
    controller:send_response()
    Middleware.run_after(ctx)
    return
  end

  -- Run the action
  local action = result.action
  if not controller[action] then
    SetStatus(500)
    SetHeader("Content-Type", "text/plain")
    Write("Error: Action '" .. action .. "' not found in " .. result.controller .. " controller")
    return
  end

  local ok, action_err = pcall(function()
    controller[action](controller)
  end)

  if not ok then
    SetStatus(500)
    SetHeader("Content-Type", "text/html; charset=utf-8")
    Write("<h1>Error</h1><pre>" .. EscapeHtml(tostring(action_err)) .. "</pre>")
    Middleware.run_after(ctx)
    return
  end

  -- Run after action
  controller:after_action()

  -- Run global after middleware
  Middleware.run_after(ctx)

  -- Send response
  controller:send_response()
end

return Framework
