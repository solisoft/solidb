-- Migration module for Luaonbeans
-- Handles database migrations with up/down support for SoliDB

local Migrate = {}
Migrate.COLLECTION = "_migrations"
Migrate.MIGRATIONS_PATH = "db/migrations"

-- ANSI color codes for terminal output
local colors = {
  reset = "\27[0m",
  green = "\27[32m",
  red = "\27[31m",
  yellow = "\27[33m",
  blue = "\27[34m",
  dim = "\27[2m"
}

-- Helper to print colored output
local function print_color(color, text)
  print(color .. text .. colors.reset)
end

-- Ensure the migrations collection exists
function Migrate.ensure_collection()
  local result = Sdb:GetCollection(Migrate.COLLECTION)
  if not result or result.error then
    Sdb:CreateCollection(Migrate.COLLECTION)
  end
end

-- Get all executed migrations from the database
function Migrate.get_executed()
  Migrate.ensure_collection()

  local result = Sdb:Sdbql(
    "FOR doc IN `" .. Migrate.COLLECTION .. "` SORT doc.version ASC RETURN doc",
    {}
  )

  if result and result.result then
    return result.result
  end
  return {}
end

-- Get executed migration versions as a lookup table
function Migrate.get_executed_versions()
  local executed = Migrate.get_executed()
  local versions = {}
  for _, m in ipairs(executed) do
    versions[m.version] = true
  end
  return versions
end

-- Get the next batch number
function Migrate.get_next_batch()
  local result = Sdb:Sdbql(
    "FOR doc IN `" .. Migrate.COLLECTION .. "` SORT doc.batch DESC LIMIT 1 RETURN doc.batch",
    {}
  )

  if result and result.result and #result.result > 0 then
    return (result.result[1] or 0) + 1
  end
  return 1
end

-- Get the last batch number
function Migrate.get_last_batch()
  local result = Sdb:Sdbql(
    "FOR doc IN `" .. Migrate.COLLECTION .. "` SORT doc.batch DESC LIMIT 1 RETURN doc.batch",
    {}
  )

  if result and result.result and #result.result > 0 then
    return result.result[1] or 0
  end
  return 0
end

-- Scan migrations directory for migration files
function Migrate.get_migration_files()
  local files = {}

  -- Use unix.opendir if available (redbean), otherwise try io.popen
  if unix and unix.opendir then
    local dir = unix.opendir(Migrate.MIGRATIONS_PATH)
    if dir then
      for name, kind in dir do
        if kind == unix.DT_REG and name:match("%.lua$") then
          table.insert(files, name)
        end
      end
    end
  else
    -- Fallback using io.popen (may not work in all environments)
    local handle = io.popen('ls -1 "' .. Migrate.MIGRATIONS_PATH .. '" 2>/dev/null')
    if handle then
      for name in handle:lines() do
        if name:match("%.lua$") then
          table.insert(files, name)
        end
      end
      handle:close()
    end
  end

  -- Sort by filename (timestamp prefix ensures correct order)
  table.sort(files)
  return files
end

-- Parse migration filename into version and name
function Migrate.parse_filename(filename)
  local version, name = filename:match("^(%d+)_(.+)%.lua$")
  return version, name
end

-- Get pending migrations (not yet executed)
function Migrate.get_pending()
  local executed = Migrate.get_executed_versions()
  local files = Migrate.get_migration_files()
  local pending = {}

  for _, filename in ipairs(files) do
    local version, name = Migrate.parse_filename(filename)
    if version and not executed[version] then
      table.insert(pending, {
        filename = filename,
        version = version,
        name = name
      })
    end
  end

  return pending
end

-- Load a migration module
function Migrate.load_migration(filename)
  local path = Migrate.MIGRATIONS_PATH .. "/" .. filename

  -- Clear from cache to allow reloading
  local module_name = filename:gsub("%.lua$", "")
  package.loaded[module_name] = nil

  local fn, err = loadfile(path)
  if not fn then
    return nil, "Failed to load migration: " .. tostring(err)
  end

  local ok, migration = pcall(fn)
  if not ok then
    return nil, "Failed to execute migration file: " .. tostring(migration)
  end

  return migration
end

-- Run a single migration
function Migrate.run_migration(filename, direction, batch)
  local version, name = Migrate.parse_filename(filename)
  if not version then
    return false, "Invalid migration filename: " .. filename
  end

  local migration, err = Migrate.load_migration(filename)
  if not migration then
    return false, err
  end

  local method = migration[direction]
  if not method then
    return false, "Migration missing " .. direction .. "() function"
  end

  -- Execute the migration
  local ok, exec_err = pcall(method, Sdb, Migrate.helpers)
  if not ok then
    return false, "Migration failed: " .. tostring(exec_err)
  end

  -- Update tracking
  if direction == "up" then
    Sdb:CreateDocument(Migrate.COLLECTION, {
      _key = version .. "_" .. name,
      version = version,
      name = name,
      executed_at = os.date("!%Y-%m-%dT%H:%M:%SZ"),
      batch = batch
    })
  else
    Sdb:DeleteDocument(Migrate.COLLECTION .. "/" .. version .. "_" .. name)
  end

  return true
end

-- Run all pending migrations (or specified number of steps)
function Migrate.up(steps)
  Migrate.ensure_collection()

  local pending = Migrate.get_pending()
  if #pending == 0 then
    print_color(colors.green, "Nothing to migrate.")
    return true
  end

  local batch = Migrate.get_next_batch()
  local count = steps or #pending
  local executed = 0

  print("Running migrations...")

  for i = 1, math.min(count, #pending) do
    local m = pending[i]
    io.write("  " .. colors.blue .. "up " .. colors.reset .. m.version .. "_" .. m.name .. " ... ")
    io.flush()

    local ok, err = Migrate.run_migration(m.filename, "up", batch)
    if ok then
      print_color(colors.green, "OK")
      executed = executed + 1
    else
      print_color(colors.red, "FAILED")
      print_color(colors.red, "  Error: " .. tostring(err))
      return false
    end
  end

  print_color(colors.green, "Done. " .. executed .. " migration(s) executed.")
  return true
end

-- Rollback migrations
function Migrate.down(steps)
  Migrate.ensure_collection()

  steps = steps or 1
  local last_batch = Migrate.get_last_batch()

  if last_batch == 0 then
    print_color(colors.yellow, "Nothing to rollback.")
    return true
  end

  -- Get migrations from the last batch (or specified steps)
  local result
  if steps then
    result = Sdb:Sdbql(
      "FOR doc IN `" .. Migrate.COLLECTION .. "` SORT doc.version DESC LIMIT @steps RETURN doc",
      { steps = steps }
    )
  else
    result = Sdb:Sdbql(
      "FOR doc IN `" .. Migrate.COLLECTION .. "` FILTER doc.batch == @batch SORT doc.version DESC RETURN doc",
      { batch = last_batch }
    )
  end

  if not result or not result.result or #result.result == 0 then
    print_color(colors.yellow, "Nothing to rollback.")
    return true
  end

  print("Rolling back...")
  local rolled_back = 0

  for _, m in ipairs(result.result) do
    local filename = m.version .. "_" .. m.name .. ".lua"
    io.write("  " .. colors.yellow .. "down " .. colors.reset .. m.version .. "_" .. m.name .. " ... ")
    io.flush()

    local ok, err = Migrate.run_migration(filename, "down", m.batch)
    if ok then
      print_color(colors.green, "OK")
      rolled_back = rolled_back + 1
    else
      print_color(colors.red, "FAILED")
      print_color(colors.red, "  Error: " .. tostring(err))
      return false
    end
  end

  print_color(colors.green, "Done. " .. rolled_back .. " migration(s) rolled back.")
  return true
end

-- Show migration status
function Migrate.status()
  Migrate.ensure_collection()

  local executed = Migrate.get_executed()
  local executed_versions = {}
  for _, m in ipairs(executed) do
    executed_versions[m.version] = m
  end

  local files = Migrate.get_migration_files()

  print("\nMigration Status")
  print("================")

  if #files == 0 then
    print_color(colors.dim, "  No migrations found in " .. Migrate.MIGRATIONS_PATH)
    return
  end

  for _, filename in ipairs(files) do
    local version, name = Migrate.parse_filename(filename)
    if version then
      local m = executed_versions[version]
      if m then
        print(colors.green .. "  [x] " .. colors.reset .. version .. "_" .. name ..
              colors.dim .. " (batch " .. m.batch .. ")" .. colors.reset)
      else
        print(colors.yellow .. "  [ ] " .. colors.reset .. version .. "_" .. name ..
              colors.dim .. " (pending)" .. colors.reset)
      end
    end
  end
  print("")
end

-- Create a new migration file
function Migrate.create(name)
  if not name or name == "" then
    print_color(colors.red, "Error: Migration name required")
    print("Usage: migrate create <name>")
    return false
  end

  -- Sanitize name
  name = name:gsub("[^%w_]", "_"):lower()

  -- Generate timestamp
  local timestamp = os.date("!%Y%m%d%H%M%S")
  local filename = timestamp .. "_" .. name .. ".lua"
  local filepath = Migrate.MIGRATIONS_PATH .. "/" .. filename

  -- Ensure directory exists
  os.execute("mkdir -p " .. Migrate.MIGRATIONS_PATH)

  -- Create migration file
  local template = [[-- Migration: ]] .. name .. [[

local M = {}

function M.up(db, helpers)
  -- Add your migration code here
  -- Examples:
  -- helpers.create_collection("users")
  -- helpers.add_index("users", { "email" }, { unique = true })
  -- helpers.seed("users", {{ email = "admin@example.com", role = "admin" }})
end

function M.down(db, helpers)
  -- Add your rollback code here
  -- Examples:
  -- helpers.drop_collection("users")
end

return M
]]

  local file = io.open(filepath, "w")
  if not file then
    print_color(colors.red, "Error: Could not create file " .. filepath)
    return false
  end

  file:write(template)
  file:close()

  print_color(colors.green, "Created: " .. filepath)
  return true
end

-- Helper functions for use in migrations
Migrate.helpers = {}

function Migrate.helpers.create_collection(name, options)
  return Sdb:CreateCollection(name, options)
end

function Migrate.helpers.drop_collection(name)
  return Sdb:DeleteCollection(name)
end

function Migrate.helpers.truncate_collection(name)
  return Sdb:TruncateCollection(name)
end

function Migrate.helpers.add_index(collection, fields, options)
  options = options or {}
  local params = {
    type = options.type or "persistent",
    fields = fields,
    unique = options.unique or false,
    sparse = options.sparse or false
  }
  if options.name then
    params.name = options.name
  end
  return Sdb:CreateIndex(collection, params)
end

function Migrate.helpers.drop_index(collection, index_name)
  return Sdb:DeleteIndex(collection .. "/" .. index_name)
end

function Migrate.helpers.seed(collection, documents)
  local results = {}
  for _, doc in ipairs(documents) do
    local result = Sdb:CreateDocument(collection, doc)
    table.insert(results, result)
  end
  return results
end

function Migrate.helpers.transform(collection, callback)
  -- Fetch all documents and transform them
  local result = Sdb:Sdbql(
    "FOR doc IN `" .. collection .. "` RETURN doc",
    {}
  )

  if not result or not result.result then
    return false, "Failed to fetch documents"
  end

  local updated = 0
  for _, doc in ipairs(result.result) do
    local new_data = callback(doc)
    if new_data then
      Sdb:UpdateDocument(collection .. "/" .. doc._key, new_data)
      updated = updated + 1
    end
  end

  return true, updated
end

function Migrate.helpers.execute(query, bindvars)
  return Sdb:Sdbql(query, bindvars or {})
end

return Migrate
