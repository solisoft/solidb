#!/usr/bin/env lua
-- Migration CLI runner for Luaonbeans
-- Usage:
--   ./luaonbeans.org -i db/migrate.lua up [steps]
--   ./luaonbeans.org -i db/migrate.lua down [steps]
--   ./luaonbeans.org -i db/migrate.lua status
--   ./luaonbeans.org -i db/migrate.lua create <name>

-- Configure package path
package.path = package.path .. ";.lua/?.lua;.lua/db/?.lua;config/?.lua"

-- Load database configuration
local db_config_ok, db_config = pcall(require, "database")
if not db_config_ok or not db_config.solidb then
  print("\27[31mError: Database configuration not found.\27[0m")
  print("Please ensure config/database.lua exists with solidb configuration.")
  os.exit(1)
end

-- Initialize SoliDB connection
local SoliDB = require("solidb")
_G.Sdb = SoliDB.new(db_config.solidb)

-- Load migration module
local Migrate = require("migrate")

-- Parse command line arguments
local command = arg[1]
local param = arg[2]

-- Help text
local function print_help()
  print([[
Luaonbeans Migration Tool

Usage:
  ./luaonbeans.org -i db/migrate.lua <command> [options]

Commands:
  up [steps]       Run pending migrations (all or specified number)
  down [steps]     Rollback migrations (default: 1)
  status           Show migration status
  create <name>    Create a new migration file

Examples:
  ./luaonbeans.org -i db/migrate.lua up
  ./luaonbeans.org -i db/migrate.lua up 1
  ./luaonbeans.org -i db/migrate.lua down
  ./luaonbeans.org -i db/migrate.lua down 2
  ./luaonbeans.org -i db/migrate.lua status
  ./luaonbeans.org -i db/migrate.lua create add_users_collection
]])
end

-- Execute command
if command == "up" then
  local steps = param and tonumber(param) or nil
  local ok = Migrate.up(steps)
  os.exit(ok and 0 or 1)

elseif command == "down" then
  local steps = param and tonumber(param) or 1
  local ok = Migrate.down(steps)
  os.exit(ok and 0 or 1)

elseif command == "status" then
  Migrate.status()
  os.exit(0)

elseif command == "create" then
  if not param then
    print("\27[31mError: Migration name required\27[0m")
    print("Usage: ./luaonbeans.org -i db/migrate.lua create <name>")
    os.exit(1)
  end
  local ok = Migrate.create(param)
  os.exit(ok and 0 or 1)

elseif command == "help" or command == "-h" or command == "--help" then
  print_help()
  os.exit(0)

else
  if command then
    print("\27[31mUnknown command: " .. command .. "\27[0m\n")
  end
  print_help()
  os.exit(1)
end
