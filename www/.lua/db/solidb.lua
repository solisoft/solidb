local JSON = require("json")

Solidb = {}
Solidb.__index = Solidb

function Solidb.new(db_config)
  local self = setmetatable({}, Solidb)

  self._lastDBConnect = GetTime()
  self._db_config = db_config
  self._token = ""

  self:Auth()

  return self
end

function Solidb:Api_url(path)
  return self._db_config.url .. path
end

function Solidb:Api_run(path, method, params, headers)
  params = params or {}
  headers = headers or {}
  
  -- Add Authorization header if we have a token
  if self._token ~= "" then
    headers = table.append({ ["Authorization"] = "Bearer " .. self._token }, headers)
  end

  local ok, h, body = Fetch(self:Api_url(path), {
    method = method,
    body = EncodeJson(params) or "",
    headers = headers,
  })
  
  -- Handle empty body or error
  if not body or body == "" then
    return {}, ok, h
  end

  return DecodeJson(body), ok, h
end

function Solidb:Auth()
  local ok, headers, body = Fetch(self._db_config.url .. "/auth/login", {
    method = "POST",
    body = '{ "username": "' .. self._db_config.username .. '", "password": "' .. self._db_config.password .. '" }',
    headers = { ["Content-Type"] = "application/json" }
  })

  if ok == 200 then
    local res = DecodeJson(body)
    if res then
      self._token = res["token"]
    end
  end

  return self._token
end

function Solidb:_get_db_path(suffix)
  return "/_api/database/" .. self._db_config.db_name .. suffix
end

function Solidb:Raw_sdbql(stm)
  -- SolidDB cursor endpoint: POST /_api/database/{db}/cursor
  local body, status_code = self:Api_run(self:_get_db_path("/cursor"), "POST", stm)
  -- assert(body, "Failed to execute SDBQL")
  if not body then return { error = true, errorMessage = "No response" } end

  local result = body["result"]
  local has_more = body["hasMore"]
  local extra = body["extra"] or {}

  while has_more do
    -- SolidDB next batch endpoint: PUT /_api/cursor/{id}
    body = self:Api_run("/_api/cursor/" .. body["id"], "PUT")
    if not body then break end
    
    if body["result"] then
        result = table.append(result, body["result"])
    end
    has_more = body["hasMore"]
  end

  if result == nil then
    result = {}
  end

  if body.error then
    return body
  else
    return { result = result, extra = extra }
  end
end

function Solidb:Sdbql(str, bindvars, options)
  bindvars = bindvars or {}
  options = options or { fullCount = true }
  -- SolidDB expects { query: "...", bindVars: {...}, ... } similar to Arango
  local request = self:Raw_sdbql({ query = str, bindVars = bindvars, count = options.fullCount, batchSize = options.batchSize })
  return request
end

-- Helper for simple path params
function Solidb:with_Params(path, method, params)
  return self:Api_run(path, method, params)
end

function Solidb:without_Params(path, method)
  return self:Api_run(path, method)
end

-- Documents

function Solidb:UpdateDocument(handle, params, options)
  -- handle should be "collection/key"
  local collection, key = handle:match("([^/]+)/([^/]+)")
  if not collection or not key then return nil, 400, "Invalid handle format (expected collection/key)" end
  
  -- PATCH /_api/database/{db}/document/{collection}/{key}
  return self:with_Params(self:_get_db_path("/document/" .. collection .. "/" .. key), "PUT", params) 
  -- Note: SolidDB might use PUT for update_document (route line 141) or PATCH?
  -- Routes line 141: put(update_document). Arango uses PATCH for storage update, PUT for replace. 
  -- Checking routes.rs: line 141 `put(update_document)`. 
  -- I will use PUT.
end

function Solidb:CreateDocument(collection, params, options)
    -- POST /_api/database/{db}/document/{collection}
    -- options? query params?
  return self:with_Params(self:_get_db_path("/document/" .. collection), "POST", params)
end

function Solidb:GetDocument(handle)
  local collection, key = handle:match("([^/]+)/([^/]+)")
  if not collection or not key then return nil, 400, "Invalid handle format" end

  -- GET /_api/database/{db}/document/{collection}/{key}
  return self:without_Params(self:_get_db_path("/document/" .. collection .. "/" .. key), "GET")
end

function Solidb:DeleteDocument(handle)
  local collection, key = handle:match("([^/]+)/([^/]+)")
  if not collection or not key then return nil, 400, "Invalid handle format" end

  -- DELETE /_api/database/{db}/document/{collection}/{key}
  return self:without_Params(self:_get_db_path("/document/" .. collection .. "/" .. key), "DELETE")
end

---Collections

function Solidb:UpdateCollection(collection, params)
    -- PUT /_api/database/{db}/collection/{name}/properties
  return self:with_Params(self:_get_db_path("/collection/" .. collection .. "/properties"), "PUT", params)
end

function Solidb:RenameCollection(collection, params)
    -- SolidDB might not support rename yet? Checked routes.rs, didn't see explicit rename route.
    -- Routes check:
    -- create, list, delete, truncate, compact, recount, repair, stats, sharding, count, properties, export, import, _copy_shard.
    -- No rename.
    return nil, 404, "Not implemented in SolidDB"
end

function Solidb:CreateCollection(collection, options)
  options = options or {}
  local params = { name = collection }
  params = table.merge(params, options)
  -- POST /_api/database/{db}/collection
  return self:with_Params(self:_get_db_path("/collection"), "POST", params)
end

function Solidb:GetCollection(collection)
    return self:without_Params(self:_get_db_path("/collection/" .. collection .. "/stats"), "GET")
end

function Solidb:DeleteCollection(collection)
    -- DELETE /_api/database/{db}/collection/{name}
  return self:without_Params(self:_get_db_path("/collection/" .. collection), "DELETE")
end

function Solidb:TruncateCollection(collection, params)
  -- PUT /_api/database/{db}/collection/{name}/truncate
  return self:with_Params(self:_get_db_path("/collection/" .. collection .. "/truncate"), "PUT", params)
end

-- Databases

function Solidb:CreateDatabase(name, options, users)
  local params = { name = name, options = (options or {}) }
  if users then
    params.users = users
  end
  -- POST /_api/database
  return self:with_Params("/_api/database", "POST", params)
end

function Solidb:DeleteDatabase(name)
  -- DELETE /_api/database/{name}
  return self:without_Params("/_api/database/" .. name, "DELETE")
end

-- Indexes

function Solidb:GetAllIndexes(collection)
    -- GET /_api/database/{db}/index/{collection}
  return self:without_Params(self:_get_db_path("/index/" .. collection), "GET")
end

function Solidb:CreateIndex(collection, params)
    -- POST /_api/database/{db}/index/{collection}
  return self:with_Params(self:_get_db_path("/index/" .. collection), "POST", params)
end

function Solidb:DeleteIndex(handle)
  -- Handle probably "collection/indexName"
  local collection, indexName = handle:match("([^/]+)/([^/]+)")
  if not collection or not indexName then return nil, 400, "Invalid index handle" end

  -- DELETE /_api/database/{db}/index/{collection}/{name}
  return self:without_Params(self:_get_db_path("/index/" .. collection .. "/" .. indexName), "DELETE")
end

-- Transactions
-- SolidDB routes.rs lines 182+:
-- /_api/database/{db}/transaction/begin (POST) -> begin_transaction
-- /_api/database/{db}/transaction/{tx_id}/commit (POST) -> commit_transaction (Note: POST, not PUT like Arango)
-- /_api/database/{db}/transaction/{tx_id}/rollback (POST) -> rollback_transaction (Note: POST, not DELETE like Arango)

function Solidb:BeginTransaction(params)
  return self:with_Params(self:_get_db_path("/transaction/begin"), "POST", params)
end

function Solidb:CommitTransaction(transaction_id)
  -- POST /_api/database/{db}/transaction/{tx_id}/commit
  return self:without_Params(self:_get_db_path("/transaction/" .. transaction_id .. "/commit"), "POST")
end

function Solidb:AbortTransaction(transaction_id)
  -- POST /_api/database/{db}/transaction/{tx_id}/rollback
  return self:without_Params(self:_get_db_path("/transaction/" .. transaction_id .. "/rollback"), "POST")
end

-- Token

function Solidb:RefreshToken()
  if GetTime() - self._lastDBConnect > 600 then
    self:Auth()
    self._lastDBConnect = GetTime()
  end
end

return Solidb
