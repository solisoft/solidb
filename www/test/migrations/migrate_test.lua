-- Tests for migration module
-- test/migrations/migrate_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local describe, it, expect, before, after = Test.describe, Test.it, Test.expect, Test.before, Test.after

-- Mock state
local mock_collections = {}
local mock_documents = {}
local mock_indexes = {}

-- Mock SoliDB
local MockSdb = {}

function MockSdb:GetCollection(name)
  if mock_collections[name] then
    return { name = name }
  end
  return { error = true }
end

function MockSdb:CreateCollection(name, options)
  mock_collections[name] = { name = name, options = options or {} }
  mock_documents[name] = {}
  return { name = name }
end

function MockSdb:DeleteCollection(name)
  mock_collections[name] = nil
  mock_documents[name] = nil
  return {}
end

function MockSdb:TruncateCollection(name)
  mock_documents[name] = {}
  return {}
end

-- Key counter for auto-generated keys
local key_counter = 0

function MockSdb:CreateDocument(collection, data)
  if not mock_documents[collection] then
    mock_documents[collection] = {}
  end
  key_counter = key_counter + 1
  local key = data._key or tostring(key_counter)
  data._key = key
  data._id = collection .. "/" .. key
  mock_documents[collection][key] = data
  return { new = data, _key = key, _id = data._id }
end

function MockSdb:GetDocument(handle)
  local collection, key = handle:match("([^/]+)/(.+)")
  if mock_documents[collection] and mock_documents[collection][key] then
    return mock_documents[collection][key]
  end
  return nil
end

function MockSdb:UpdateDocument(handle, data)
  local collection, key = handle:match("([^/]+)/(.+)")
  if mock_documents[collection] and mock_documents[collection][key] then
    for k, v in pairs(data) do
      mock_documents[collection][key][k] = v
    end
    return { new = mock_documents[collection][key] }
  end
  return { error = true }
end

function MockSdb:DeleteDocument(handle)
  local collection, key = handle:match("([^/]+)/(.+)")
  if mock_documents[collection] then
    mock_documents[collection][key] = nil
    return {}
  end
  return { error = true }
end

function MockSdb:CreateIndex(collection, params)
  if not mock_indexes[collection] then
    mock_indexes[collection] = {}
  end
  local name = params.name or table.concat(params.fields, "_")
  mock_indexes[collection][name] = params
  return { name = name }
end

function MockSdb:DeleteIndex(handle)
  local collection, name = handle:match("([^/]+)/(.+)")
  if mock_indexes[collection] then
    mock_indexes[collection][name] = nil
    return {}
  end
  return { error = true }
end

function MockSdb:Sdbql(query, bindvars)
  local collection = query:match("IN `([^`]+)`")

  if query:match("^FOR doc IN") then
    local results = {}
    if mock_documents[collection] then
      for _, doc in pairs(mock_documents[collection]) do
        local include = true
        if bindvars.version and doc.version ~= bindvars.version then
          include = false
        end
        if bindvars.batch and doc.batch ~= bindvars.batch then
          include = false
        end
        if include then
          table.insert(results, doc)
        end
      end

      -- Sort
      if query:match("SORT doc%.version ASC") then
        table.sort(results, function(a, b) return (a.version or "") < (b.version or "") end)
      elseif query:match("SORT doc%.version DESC") then
        table.sort(results, function(a, b) return (a.version or "") > (b.version or "") end)
      elseif query:match("SORT doc%.batch DESC") then
        table.sort(results, function(a, b) return (a.batch or 0) > (b.batch or 0) end)
      end

      -- LIMIT
      local limit = query:match("LIMIT (%d+)") or (query:match("LIMIT @steps") and bindvars.steps)
      if limit then
        limit = tonumber(limit)
        local limited = {}
        for i = 1, math.min(limit, #results) do
          table.insert(limited, results[i])
        end
        results = limited
      end

      -- RETURN doc.batch
      if query:match("RETURN doc%.batch") then
        local batches = {}
        for _, doc in ipairs(results) do
          table.insert(batches, doc.batch)
        end
        return { result = batches }
      end
    end
    return { result = results }
  end
  return { result = {} }
end

-- Set up mock globally before loading migrate
_G.Sdb = MockSdb

-- Load migrate module
local Migrate = require("migrate")

-- Helper to reset mock state (clear tables, don't reassign)
local function reset_mocks()
  for k in pairs(mock_collections) do mock_collections[k] = nil end
  for k in pairs(mock_documents) do mock_documents[k] = nil end
  for k in pairs(mock_indexes) do mock_indexes[k] = nil end
end

describe("Migrate", function()
  before(function()
    reset_mocks()
  end)

  describe("ensure_collection", function()
    it("should create _migrations collection if not exists", function()
      Migrate.ensure_collection()
      expect.truthy(mock_collections["_migrations"])
    end)

    it("should not error if collection already exists", function()
      mock_collections["_migrations"] = { name = "_migrations" }
      expect.no_error(function()
        Migrate.ensure_collection()
      end)
    end)
  end)

  describe("get_executed", function()
    it("should return empty array when no migrations executed", function()
      local executed = Migrate.get_executed()
      expect.eq(#executed, 0)
    end)

    it("should return executed migrations", function()
      mock_collections["_migrations"] = { name = "_migrations" }
      mock_documents["_migrations"] = {
        ["20251231120000_test"] = {
          _key = "20251231120000_test",
          version = "20251231120000",
          name = "test",
          batch = 1
        }
      }

      local executed = Migrate.get_executed()
      expect.eq(#executed, 1)
      expect.eq(executed[1].version, "20251231120000")
    end)
  end)

  describe("parse_filename", function()
    it("should parse valid migration filename", function()
      local version, name = Migrate.parse_filename("20251231120000_create_users.lua")
      expect.eq(version, "20251231120000")
      expect.eq(name, "create_users")
    end)

    it("should return nil for invalid filename", function()
      local version, name = Migrate.parse_filename("invalid.lua")
      expect.nil_value(version)
      expect.nil_value(name)
    end)
  end)

  describe("helpers", function()
    it("should create collection", function()
      Migrate.helpers.create_collection("users")
      expect.truthy(mock_collections["users"])
    end)

    it("should drop collection", function()
      mock_collections["posts"] = { name = "posts" }
      mock_documents["posts"] = {}
      Migrate.helpers.drop_collection("posts")
      expect.nil_value(mock_collections["posts"])
    end)

    it("should add index", function()
      mock_collections["users"] = { name = "users" }
      mock_indexes["users"] = {}
      Migrate.helpers.add_index("users", { "email" }, { unique = true })
      expect.truthy(mock_indexes["users"]["email"])
    end)

    it("should seed documents", function()
      mock_collections["users"] = { name = "users" }
      mock_documents["users"] = {}

      Migrate.helpers.seed("users", {
        { email = "admin@example.com", role = "admin" },
        { email = "user@example.com", role = "user" }
      })

      local count = 0
      for _ in pairs(mock_documents["users"]) do
        count = count + 1
      end
      expect.eq(count, 2)
    end)
  end)

  describe("get_next_batch", function()
    it("should return 1 when no migrations exist", function()
      mock_collections["_migrations"] = { name = "_migrations" }
      mock_documents["_migrations"] = {}

      local batch = Migrate.get_next_batch()
      expect.eq(batch, 1)
    end)

    it("should return next batch number", function()
      mock_collections["_migrations"] = { name = "_migrations" }
      mock_documents["_migrations"] = {
        ["20251231120000_test"] = {
          _key = "20251231120000_test",
          version = "20251231120000",
          name = "test",
          batch = 2
        }
      }

      local batch = Migrate.get_next_batch()
      expect.eq(batch, 3)
    end)
  end)

  describe("get_executed_versions", function()
    it("should return lookup table of executed versions", function()
      mock_collections["_migrations"] = { name = "_migrations" }
      mock_documents["_migrations"] = {
        ["20251231120000_test"] = {
          _key = "20251231120000_test",
          version = "20251231120000",
          name = "test",
          batch = 1
        },
        ["20251231120100_another"] = {
          _key = "20251231120100_another",
          version = "20251231120100",
          name = "another",
          batch = 1
        }
      }

      local versions = Migrate.get_executed_versions()
      expect.truthy(versions["20251231120000"])
      expect.truthy(versions["20251231120100"])
      expect.nil_value(versions["20251231120200"])
    end)
  end)
end)

return Test
