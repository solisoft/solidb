-- DB Loader for LuaOnBeans
-- Loads controllers, views, models, routes, middleware from _luaonbeans collection
-- DB has priority over filesystem

local DbLoader = {}
DbLoader.cache = {}
DbLoader.collection = "_luaonbeans"

-- Query the _luaonbeans collection for a specific type and name
function DbLoader.load_code(doc_type, name)
  -- Check cache first
  local cache_key = doc_type .. ":" .. name
  if DbLoader.cache[cache_key] then
    return DbLoader.cache[cache_key]
  end

  -- Check if we have a DB connection
  if not _G.Sdb then
    return nil
  end

  -- Query the collection
  local query = string.format(
    'FOR doc IN %s FILTER doc.type == @type AND doc.name == @name RETURN doc',
    DbLoader.collection
  )

  local ok, result = pcall(function()
    return _G.Sdb:Sdbql(query, { type = doc_type, name = name })
  end)

  if not ok or not result or result.error then
    return nil
  end

  local docs = result.result or {}
  if #docs == 0 then
    return nil
  end

  local doc = docs[1]
  DbLoader.cache[cache_key] = doc
  return doc
end

-- Load and compile a controller from DB
function DbLoader.load_controller(name)
  local doc = DbLoader.load_code("controller", name)
  if not doc or not doc.code then
    return nil
  end

  -- Compile the Lua code
  local fn, err = load(doc.code, "db_controller:" .. name, "t", _G)
  if not fn then
    Log(kLogWarn, "DbLoader: Failed to compile controller '" .. name .. "': " .. tostring(err))
    return nil
  end

  -- Execute and return the controller class
  local ok, controller = pcall(fn)
  if not ok then
    Log(kLogWarn, "DbLoader: Failed to execute controller '" .. name .. "': " .. tostring(controller))
    return nil
  end

  return controller
end

-- Load a view/partial/layout template from DB
function DbLoader.load_view(view_type, name)
  local doc = DbLoader.load_code(view_type, name)
  if not doc or not doc.code then
    return nil
  end
  return doc.code
end

-- Parse a view path to determine type and name
-- app/views/home/index.etlua -> view, home/index
-- app/views/home/_sidebar.etlua -> partial, home/_sidebar
-- app/views/layouts/application/application.etlua -> layout, application
function DbLoader.parse_view_path(path)
  -- Remove app/views/ prefix and .etlua suffix
  local view_path = path:match("^app/views/(.+)%.etlua$")
  if not view_path then
    return nil, nil
  end

  -- Check if it's a layout
  if view_path:match("^layouts/") then
    -- layouts/application/application -> application
    local layout_name = view_path:match("^layouts/([^/]+)/[^/]+$")
    if layout_name then
      return "layout", layout_name
    end
    return nil, nil
  end

  -- Check if it's a partial (filename starts with _)
  local parts = {}
  for part in view_path:gmatch("[^/]+") do
    table.insert(parts, part)
  end

  if #parts > 0 then
    local filename = parts[#parts]
    if filename:sub(1, 1) == "_" then
      return "partial", view_path
    end
  end

  -- Regular view
  return "view", view_path
end

-- Load and compile a model from DB
function DbLoader.load_model(name)
  local doc = DbLoader.load_code("model", name)
  if not doc or not doc.code then
    return nil
  end

  local fn, err = load(doc.code, "db_model:" .. name, "t", _G)
  if not fn then
    Log(kLogWarn, "DbLoader: Failed to compile model '" .. name .. "': " .. tostring(err))
    return nil
  end

  local ok, model = pcall(fn)
  if not ok then
    Log(kLogWarn, "DbLoader: Failed to execute model '" .. name .. "': " .. tostring(model))
    return nil
  end

  return model
end

-- Load and compile a middleware from DB
function DbLoader.load_middleware(name)
  local doc = DbLoader.load_code("middleware", name)
  if not doc or not doc.code then
    return nil
  end

  local fn, err = load(doc.code, "db_middleware:" .. name, "t", _G)
  if not fn then
    Log(kLogWarn, "DbLoader: Failed to compile middleware '" .. name .. "': " .. tostring(err))
    return nil
  end

  local ok, middleware = pcall(fn)
  if not ok then
    Log(kLogWarn, "DbLoader: Failed to execute middleware '" .. name .. "': " .. tostring(middleware))
    return nil
  end

  return middleware
end

-- Load and compile a lib from DB
function DbLoader.load_lib(name)
  local doc = DbLoader.load_code("lib", name)
  if not doc or not doc.code then
    return nil
  end

  local fn, err = load(doc.code, "db_lib:" .. name, "t", _G)
  if not fn then
    Log(kLogWarn, "DbLoader: Failed to compile lib '" .. name .. "': " .. tostring(err))
    return nil
  end

  local ok, lib = pcall(fn)
  if not ok then
    Log(kLogWarn, "DbLoader: Failed to execute lib '" .. name .. "': " .. tostring(lib))
    return nil
  end

  return lib
end

-- Load and execute routes from DB
function DbLoader.load_routes()
  local doc = DbLoader.load_code("routes", "main")
  if not doc or not doc.code then
    return false
  end

  -- Make router available in the execution environment
  local env = setmetatable({
    router = require("router")
  }, { __index = _G })

  local fn, err = load(doc.code, "db_routes", "t", env)
  if not fn then
    Log(kLogWarn, "DbLoader: Failed to compile routes: " .. tostring(err))
    return false
  end

  local ok, exec_err = pcall(fn)
  if not ok then
    Log(kLogWarn, "DbLoader: Failed to execute routes: " .. tostring(exec_err))
    return false
  end

  return true
end

-- Load and execute middleware config from DB
function DbLoader.load_middleware_config()
  local doc = DbLoader.load_code("middleware_config", "main")
  if not doc or not doc.code then
    return false
  end

  -- Make Middleware available in the execution environment
  local env = setmetatable({
    Middleware = require("middleware")
  }, { __index = _G })

  local fn, err = load(doc.code, "db_middleware_config", "t", env)
  if not fn then
    Log(kLogWarn, "DbLoader: Failed to compile middleware config: " .. tostring(err))
    return false
  end

  local ok, exec_err = pcall(fn)
  if not ok then
    Log(kLogWarn, "DbLoader: Failed to execute middleware config: " .. tostring(exec_err))
    return false
  end

  return true
end

-- Clear the cache
function DbLoader.clear_cache()
  DbLoader.cache = {}
end

return DbLoader
