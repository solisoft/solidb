-- Luaonbeans MVC Framework
-- Bootstrap file for redbean.dev

-- Configure package path for app modules
package.path = package.path .. ";.lua/?.lua;.lua/db/?.lua;app/?.lua;app/controllers/?.lua;app/models/?.lua;config/?.lua;config/locales/?.lua"

ProgramMaxPayloadSize(10485760 * 10) -- 100 MB

-- Environment (default to development)
BEANS_ENV = os.getenv("BEANS_ENV") or "development"

function RefreshPageForDevMode()
	if BEANS_ENV == "development" then
		return [[<script src="/live_reload.js"></script>]]
	else
		return ""
	end
end

require("session")

-- Load framework modules
local router = require("router")
local Controller = require("controller")
local view = require("view")
local helpers = require("helpers")
local I18n = require("i18n")
local Middleware = require("middleware")

-- Global debug helper
function P(...)
  local args = {...}
  local formatted = {}
  for _, v in ipairs(args) do
    if type(v) == "table" then
      table.insert(formatted, EncodeJson(v))
    else
      table.insert(formatted, tostring(v))
    end
  end

  local log_file = "debug.log"
  local current_content = Slurp(log_file) or ""
  local new_entry = string.format("[%s] %s\n", os.date(), table.concat(formatted, "\t"))

  Barf(log_file, current_content .. new_entry)
end

_G.ENV = {}

local env_data = LoadAsset(".env")
if env_data then
  for line in env_data:gmatch("[^\r\n]+") do
    local key, value = line:match("([^=]+)=(.+)")
    if key and value then
      _G.ENV[key:gsub("%s+", "")] = value:gsub("%s+", "")
    end
  end
end


-- Load database driver (optional - only if config exists)
local db_config_ok, db_config = pcall(require, "database")
if db_config_ok and db_config.solidb then
  local SoliDB = require("solidb")
  -- Global DB connection
  _G.Sdb = SoliDB.new(db_config.solidb)

  -- Add timing wrapper for Sdbql (for performance debugging)
  if _G.Sdb then
    local original_sdbql = _G.Sdb.Sdbql
    _G.Sdb.Sdbql = function(self, query, params)
      local start_time = GetTime()
      local result = original_sdbql(self, query, params)
      local elapsed_ms = (GetTime() - start_time) / 1000
      local params_str = params and EncodeJson(params) or "{}"
      P(string.format("[TIMING] Sdbql %.2fms: %s | params: %s", elapsed_ms, (query or ""):sub(1, 80), params_str:sub(1, 100)))
      return result
    end
  end

  -- Ensure _luaonbeans system collection exists
  local function ensure_luaonbeans_collection()
    if not _G.Sdb then return end
    -- Try to create the collection (will fail silently if it already exists)
    pcall(function()
      _G.Sdb:CreateCollection("_luaonbeans", { type = "document" })
    end)
  end
  ensure_luaonbeans_collection()
end

-- Initialize I18n
I18n:load_locale("en")
I18n:load_locale("fr")
I18n:make_global() -- Creates global t() function

-- Controller cache
local controllers = {}

-- Load a controller by name (DB-first, then filesystem)
local function load_controller(name)
  if controllers[name] then
    return controllers[name]
  end

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

  return nil, "Controller not found: " .. name
end

-- Clear caches (useful for development)
local function clear_caches()
  controllers = {}
  view.clear_cache()
  Middleware.clear()
  -- NOTE: We intentionally do NOT clear DbLoader cache here.
  -- DbLoader loads code from the _luaonbeans collection which rarely changes.
  -- It will be cleared only on explicit server reload (OnServerReload/SIGHUP).
  -- Clear loaded modules to allow reload
  -- Note: We DON'T clear 'router' or 'middleware' because they are singletons that
  -- framework.lua and other modules reference. routes.lua calls router.clear() itself.
  for k, _ in pairs(package.loaded) do
    if k:match("_controller$") or k:match("^middleware/") or k == "controller" or k == "view" or k == "helpers" or k == "framework" or k == "routes" or k == "middleware_config" then
      package.loaded[k] = nil
    end
  end
end

-- Load routes (reloadable, DB-first then filesystem)
local function load_routes()
  package.loaded["routes"] = nil

  -- Load DB routes first (they define routes that take priority)
  local DbLoader = require("dbloader")
  DbLoader.load_routes()

  -- Then load filesystem routes (will add any routes not already defined)
  require("routes")
end

-- Load middleware config (reloadable, DB-first then filesystem)
local function load_middleware()
  package.loaded["middleware_config"] = nil

  -- Load DB middleware config first
  local DbLoader = require("dbloader")
  DbLoader.load_middleware_config()

  -- Then load filesystem middleware config
  local ok, err = pcall(require, "middleware_config")
  if not ok then
    -- Middleware config is optional
    Log(kLogDebug, "Middleware config not found or error: " .. tostring(err))
  end
end

-- ============================================
-- LOAD ROUTES AND MIDDLEWARE
-- ============================================
load_middleware()  -- Register middleware names first
load_routes()      -- Then routes can reference them

-- ============================================
-- REDBEAN HOOKS
-- ============================================

function OnServerStart()
  Log(kLogInfo, "Luaonbeans MVC Framework started")
end

function OnServerReload()
  Log(kLogInfo, "Reloading Luaonbeans...")
  clear_caches()
  -- Clear DbLoader cache on explicit reload (SIGHUP)
  local DbLoader = require("dbloader")
  DbLoader.clear_cache()
  load_routes()
  load_middleware()
end

function OnServerHeartbeat()
end

function OnWorkerStart()
end

function OnError(status, message, details)
  SetStatus(status)
  SetHeader("Content-Type", "text/html; charset=utf-8")

  local content = view.render("errors/error", {
    status = status,
    message = message,
    details = details and EscapeHtml(details) or nil
  }, { layout = false })

  Write(content)
end

function OnHttpRequest()
  -- Hot reload routes and middleware in development mode
  if BEANS_ENV == "development" then
    clear_caches()
    load_middleware()  -- Load middleware FIRST so names are registered
    load_routes()      -- Then routes can reference middleware by name
  end

  SetHeader("X-Framework-Version", "2.0-reload-test")
  -- 1. First, try to serve static files from public/ folder
  local path = GetPath()
  local public_path = "public" .. path
  local file_content = nil

  -- Try zip asset if not found in filesystem
  local file_content = LoadAsset("/public" .. path)

  if file_content then
    -- Determine content type based on extension
    local ext = path:match("%.([^%.]+)$")
    local content_types = {
      css = "text/css; charset=utf-8",
      js = "application/javascript; charset=utf-8",
      json = "application/json; charset=utf-8",
      html = "text/html; charset=utf-8",
      htm = "text/html; charset=utf-8",
      xml = "application/xml; charset=utf-8",
      txt = "text/plain; charset=utf-8",
      png = "image/png",
      jpg = "image/jpeg",
      jpeg = "image/jpeg",
      gif = "image/gif",
      svg = "image/svg+xml",
      ico = "image/x-icon",
      webp = "image/webp",
      woff = "font/woff",
      woff2 = "font/woff2",
      ttf = "font/ttf",
      eot = "application/vnd.ms-fontobject",
      otf = "font/otf",
      mp4 = "video/mp4",
      webm = "video/webm",
      mp3 = "audio/mpeg",
      wav = "audio/wav",
      pdf = "application/pdf",
      zip = "application/zip",
      wasm = "application/wasm"
    }

    local content_type = content_types[ext] or "application/octet-stream"
    SetStatus(200)
    SetHeader("Content-Type", content_type)
    -- Cache static assets for 1 year
    SetHeader("Cache-Control", "public, max-age=31536000, immutable")
    Write(file_content)
    return
  end

  -- 2. Handle the request via the reloadable framework module
  local framework = require("framework")
  framework.handle_request()
end
