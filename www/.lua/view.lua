-- View renderer for luaonbeans MVC framework
-- Handles template rendering with layout support using etlua

local etlua = require("etlua")
local helpers = require("helpers")

local View = {}
View.cache = {}  -- Template cache for performance
View.views_path = "app/views"
View.layouts_path = "app/views/layouts"

-- Read file contents (DB-first for views, then filesystem)
local function read_file(path)
  -- 0. Try DB first for views/partials/layouts
  if path:match("^app/views/") then
    local DbLoader = require("dbloader")
    local view_type, view_name = DbLoader.parse_view_path(path)
    if view_type and view_name then
      local code = DbLoader.load_view(view_type, view_name)
      if code then
        return code
      end
    end
  end

  -- 1. Try filesystem (for development/hot-reload and to avoid Redbean warnings)
  local file = io.open(path, "r")
  if file then
    local content = file:read("*all")
    file:close()
    return content
  end

  -- 2. Fallback to zip asset (production/bundled)
  if type(LoadAsset) == "function" then
    local content = LoadAsset("/" .. path)
    if content then
      return content
    end
  end

  return nil, "File not found: " .. path
end

-- Check if file exists (filesystem only, no LoadAsset to avoid warnings)
local function file_exists(path)
  local file = io.open(path, "r")
  if file then
    file:close()
    return true
  end
  return false
end

-- Compile a template (with caching)
local function compile_template(path)
  if View.cache[path] then
    return View.cache[path]
  end

  local content, err = read_file(path)
  if not content then
    return nil, err
  end

  local fn, err = etlua.compile(content)
  if not fn then
    return nil, "Template compilation error in " .. path .. ": " .. tostring(err)
  end

  View.cache[path] = fn
  return fn
end

-- Helper to capture content from an etlua block
local function capture_content(fn)
  local i = 1
  local b_idx, b_val, bi_idx, bi_val
  
  -- Introspect function to find _b (buffer) and _b_i (index) upvalues from etlua
  while true do
    local n, v = debug.getupvalue(fn, i)
    if not n then break end
    if n == "_b" then b_idx = i; b_val = v end
    if n == "_b_i" then bi_idx = i; bi_val = v end
    i = i + 1
  end
  
  -- If we can't find buffer (e.g. not called from template), just run it
  if not b_val or not bi_val then
     fn()
     return ""
  end
  
  local start_index = bi_val
  
  -- Execute block (writes to buffer)
  fn()
  
  -- Get updated index
  local _, current_bi = debug.getupvalue(fn, bi_idx)
  
  -- Extract captured content from buffer
  local captured = {}
  for k = start_index + 1, current_bi do
    table.insert(captured, b_val[k])
    b_val[k] = nil -- Clean buffer
  end
  
  -- Reset buffer index
  debug.setupvalue(fn, bi_idx, start_index)
  
  return table.concat(captured)
end

-- Render a template with locals
function View.render(template, locals, options)
  options = options or {}
  locals = locals or {}

  -- Build template path with variant support
  local template_path
  local variant = options.variant

  if variant then
    -- Try variant-specific template first (e.g., show.iphone.etlua)
    local variant_path = View.views_path .. "/" .. template .. "." .. variant .. ".etlua"
    if file_exists(variant_path) then
      template_path = variant_path
    end
  end

  -- Fall back to default template
  if not template_path then
    template_path = View.views_path .. "/" .. template .. ".etlua"
  end

  -- Compile the template
  local template_fn, err = compile_template(template_path)
  if not template_fn then
    error(err)
  end

  -- Add helper functions to locals
  locals.partial = function(partial_name, partial_locals)
    partial_locals = partial_locals or {}
    -- Forward content_for and t to partials to share context
    partial_locals.content_for = locals.content_for
    partial_locals.t = partial_locals.t or locals.t
    return View.partial(partial_name, partial_locals)
  end

  locals.escape = function(str)
    return EscapeHtml(tostring(str or ""))
  end

  locals.raw = function(str)
    return str
  end

  locals.public_path = helpers.public_path

  -- Inject all helpers
  for k, v in pairs(helpers) do
    if not locals[k] and type(v) == "function" then
      locals[k] = v
    end
  end

  -- I18n helper (uses global t function if available)
  locals.t = locals.t or _G.t or function(key) return key end
  
  -- content_for helper
  locals.content_for = function(name, content_or_fn)
    if content_or_fn then
      -- Setter
      local val
      if type(content_or_fn) == "function" then
        val = capture_content(content_or_fn)
      else
        val = tostring(content_or_fn)
      end
      
      local key = "_content_" .. name
      locals[key] = (locals[key] or "") .. val
      return ""
    else
      -- Getter
      return locals["_content_" .. name] or ""
    end
  end

  -- Duplicates removed (helpers are defined above)

  -- Render the template
  local content, err = template_fn(locals)
  if not content then
    error("Template render error in " .. template_path .. ": " .. tostring(err))
  end

  -- Handle layout
  local layout = options.layout
  if layout == nil then
    layout = "application"  -- Default layout
  end

  if layout and layout ~= false then
    -- Render with layout (layouts are in their own folders: layouts/name/name.etlua)
    local layout_path = View.layouts_path .. "/" .. layout .. "/" .. layout .. ".etlua"
    local layout_fn, err = compile_template(layout_path)
    if not layout_fn then
      error(err)
    end

    -- Create layout locals with yield function
    local layout_locals = {}
    for k, v in pairs(locals) do
      layout_locals[k] = v
    end

    layout_locals.yield = function()
      return content
    end
    
    -- Removed manual content_for definition here as it's inherited from locals

    local final_content, err = layout_fn(layout_locals)
    if not final_content then
      error("Layout render error in " .. layout_path .. ": " .. tostring(err))
    end

    return final_content
  end

  return content
end

-- Render a partial template
function View.partial(partial_name, locals)
  locals = locals or {}

  -- Partials are prefixed with underscore by convention
  local parts = {}
  for part in partial_name:gmatch("[^/]+") do
    table.insert(parts, part)
  end

  -- Add underscore to the last part (filename)
  if #parts > 0 then
    parts[#parts] = "_" .. parts[#parts]
  end

  local partial_path = View.views_path .. "/" .. table.concat(parts, "/") .. ".etlua"

  local template_fn, err = compile_template(partial_path)
  if not template_fn then
    error(err)
  end

  -- Add helper functions
  locals.partial = function(name, locs)
    return View.partial(name, locs)
  end

  locals.escape = function(str)
    return EscapeHtml(tostring(str or ""))
  end

  locals.public_path = helpers.public_path

  -- Inject all helpers
  for k, v in pairs(helpers) do
    if not locals[k] and type(v) == "function" then
      locals[k] = v
    end
  end

  local content, err = template_fn(locals)
  if not content then
    error("Partial render error in " .. partial_path .. ": " .. tostring(err))
  end

  return content
end

-- Clear template cache (useful for development)
function View.clear_cache()
  View.cache = {}
end

-- Set views path
function View.set_views_path(path)
  View.views_path = path
end

-- Set layouts path
function View.set_layouts_path(path)
  View.layouts_path = path
end

return View
