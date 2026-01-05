#!/usr/bin/env lua
-- Route listing script for Luaonbeans
-- Usage: ./luaonbeans.org -i scripts/routes.lua
--
-- Options (set as environment variables):
--   FILTER=admin    - Filter routes by path pattern
--   METHOD=GET      - Filter by HTTP method
--   FORMAT=json     - Output as JSON instead of table

package.path = package.path .. ";.lua/?.lua;config/?.lua"

-- Load router and routes
local router = require("router")
router.clear()
require("routes")

-- Parse options from environment or globals
local filter = os.getenv("FILTER") or _G.FILTER
local method_filter = os.getenv("METHOD") or _G.METHOD
local format = os.getenv("FORMAT") or _G.FORMAT or "table"

-- ANSI colors
local colors = {
  reset = "\27[0m",
  bold = "\27[1m",
  dim = "\27[2m",
  green = "\27[32m",
  yellow = "\27[33m",
  blue = "\27[34m",
  magenta = "\27[35m",
  cyan = "\27[36m",
  red = "\27[31m"
}

-- Method colors
local method_colors = {
  GET = colors.green,
  POST = colors.yellow,
  PUT = colors.blue,
  PATCH = colors.magenta,
  DELETE = colors.red
}

-- Pad string to length
local function pad(str, len, align)
  str = str or ""
  if #str >= len then return str:sub(1, len) end
  local padding = string.rep(" ", len - #str)
  if align == "right" then
    return padding .. str
  end
  return str .. padding
end

-- Format middleware list
local function format_middleware(middleware)
  if not middleware or #middleware == 0 then
    return ""
  end
  return table.concat(middleware, ", ")
end

-- Collect and filter routes
local function get_routes()
  local routes = {}

  for _, route in ipairs(router.routes) do
    local include = true

    -- Apply filters
    if filter and not route.pattern:find(filter, 1, true) then
      include = false
    end

    if method_filter and route.method ~= method_filter:upper() then
      include = false
    end

    if include then
      table.insert(routes, {
        method = route.method,
        path = route.pattern,
        handler = route.handler,
        middleware = route.middleware
      })
    end
  end

  return routes
end

-- Calculate column widths
local function calculate_widths(routes)
  local widths = {
    method = 6,
    path = 4,
    handler = 7,
    middleware = 10
  }

  for _, route in ipairs(routes) do
    widths.method = math.max(widths.method, #route.method)
    widths.path = math.max(widths.path, #route.path)
    widths.handler = math.max(widths.handler, #(type(route.handler) == "string" and route.handler or "[function]"))
    widths.middleware = math.max(widths.middleware, #format_middleware(route.middleware))
  end

  -- Cap widths for readability
  widths.path = math.min(widths.path, 50)
  widths.handler = math.min(widths.handler, 40)
  widths.middleware = math.min(widths.middleware, 30)

  return widths
end

-- Output as table
local function output_table(routes)
  if #routes == 0 then
    print(colors.dim .. "No routes found." .. colors.reset)
    return
  end

  local widths = calculate_widths(routes)

  -- Header
  print("")
  print(colors.bold ..
    pad("METHOD", widths.method) .. "  " ..
    pad("PATH", widths.path) .. "  " ..
    pad("HANDLER", widths.handler) .. "  " ..
    pad("MIDDLEWARE", widths.middleware) ..
    colors.reset
  )

  -- Separator
  print(colors.dim ..
    string.rep("-", widths.method) .. "  " ..
    string.rep("-", widths.path) .. "  " ..
    string.rep("-", widths.handler) .. "  " ..
    string.rep("-", widths.middleware) ..
    colors.reset
  )

  -- Routes
  for _, route in ipairs(routes) do
    local method_color = method_colors[route.method] or colors.reset
    local handler_str = type(route.handler) == "string" and route.handler or "[function]"
    local middleware_str = format_middleware(route.middleware)

    print(
      method_color .. pad(route.method, widths.method) .. colors.reset .. "  " ..
      colors.cyan .. pad(route.path, widths.path) .. colors.reset .. "  " ..
      colors.dim .. pad(handler_str, widths.handler) .. colors.reset .. "  " ..
      (middleware_str ~= "" and (colors.magenta .. middleware_str .. colors.reset) or colors.dim .. "-" .. colors.reset)
    )
  end

  -- Summary
  print("")
  print(colors.dim .. "Total: " .. #routes .. " route(s)" .. colors.reset)
  print("")
end

-- Output as JSON
local function output_json(routes)
  local json_routes = {}

  for _, route in ipairs(routes) do
    table.insert(json_routes, {
      method = route.method,
      path = route.path,
      handler = type(route.handler) == "string" and route.handler or nil,
      middleware = route.middleware
    })
  end

  -- Simple JSON encoding
  local function encode(val, indent)
    indent = indent or 0
    local spaces = string.rep("  ", indent)

    if type(val) == "nil" then
      return "null"
    elseif type(val) == "boolean" then
      return tostring(val)
    elseif type(val) == "number" then
      return tostring(val)
    elseif type(val) == "string" then
      return '"' .. val:gsub('"', '\\"'):gsub("\n", "\\n") .. '"'
    elseif type(val) == "table" then
      local is_array = #val > 0 or next(val) == nil
      local parts = {}

      if is_array then
        for _, v in ipairs(val) do
          table.insert(parts, spaces .. "  " .. encode(v, indent + 1))
        end
        if #parts == 0 then
          return "[]"
        end
        return "[\n" .. table.concat(parts, ",\n") .. "\n" .. spaces .. "]"
      else
        for k, v in pairs(val) do
          table.insert(parts, spaces .. '  "' .. k .. '": ' .. encode(v, indent + 1))
        end
        return "{\n" .. table.concat(parts, ",\n") .. "\n" .. spaces .. "}"
      end
    end
    return "null"
  end

  print(encode(json_routes))
end

-- Output as compact list
local function output_compact(routes)
  for _, route in ipairs(routes) do
    local middleware_str = format_middleware(route.middleware)
    local handler_str = type(route.handler) == "string" and route.handler or "[fn]"

    io.write(route.method .. " " .. route.path .. " -> " .. handler_str)
    if middleware_str ~= "" then
      io.write(" [" .. middleware_str .. "]")
    end
    print("")
  end
end

-- Main
local routes = get_routes()

if format == "json" then
  output_json(routes)
elseif format == "compact" then
  output_compact(routes)
else
  output_table(routes)
end
