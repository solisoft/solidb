-- Dashboard Collections Controller
-- Handles collections, documents, and columnar operations
local DashboardBaseController = require("dashboard.base_controller")
local CollectionsController = DashboardBaseController:extend()

-- Collections page
function CollectionsController:index()

  self.layout = "dashboard"
  self:render("dashboard/collections", {
    title = "Collections - " .. self:get_db(),
    db = self:get_db(),
    current_page = "collections"
  })
end

-- Collections table partial (HTMX)
function CollectionsController:table()
  local db = self:get_db()

  -- Fetch collections from API
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collection")
  local collections = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      -- API returns { collections: [...] }
      collections = data.collections or data or {}
    end
  end

  -- Filter out system collections (starting with _)
  local filtered = {}
  for _, c in ipairs(collections) do
    if c.name and c.name:sub(1, 1) ~= "_" then
      table.insert(filtered, c)
    end
  end

  self:render_partial("dashboard/_collections_table", {
    db = db,
    collections = filtered
  })
end

-- Create collection modal
function CollectionsController:modal_create()
  self:render_partial("dashboard/_modal_create_collection", {
    db = self:get_db(),
    params = self.params
  })
end

-- Create collection action
function CollectionsController:create()
  local db = self:get_db()
  local collection_type = self.params.type or "document"

  local status, headers, body
  local endpoint
  local payload

  if collection_type == "columnar" then
    -- Columnar collections use a different API endpoint
    endpoint = "/_api/database/" .. db .. "/columnar"

    -- Parse columns JSON
    local columns = {}
    if self.params.columns and self.params.columns ~= "" then
      local ok, parsed = pcall(DecodeJson, self.params.columns)
      if ok and type(parsed) == "table" then
        columns = parsed
      end
    end

    -- Validate at least one column is defined
    if #columns == 0 then
      SetHeader("HX-Trigger", '{"showToast": {"message": "Columnar collection requires at least one column", "type": "error"}}')
      return self:table()
    end

    payload = {
      name = self.params.name,
      columns = columns
    }

    Log(kLogInfo, "Creating columnar collection: " .. EncodeJson(payload))

    status, headers, body = self:fetch_api(endpoint, {
      method = "POST",
      headers = { ["Content-Type"] = "application/json" },
      body = EncodeJson(payload)
    })
  else
    -- Standard collection (document, edge, timeseries, blob)
    endpoint = "/_api/database/" .. db .. "/collection"
    payload = {
      name = self.params.name,
      type = collection_type
    }

    -- Only include sharding params if explicitly set (> 1)
    local shards = tonumber(self.params.shards)
    local repl = tonumber(self.params.replication_factor)
    if shards and shards > 1 then
      payload.numShards = shards
    end
    if repl and repl > 1 then
      payload.replicationFactor = repl
    end

    Log(kLogInfo, "Creating collection: " .. EncodeJson(payload))

    status, headers, body = self:fetch_api(endpoint, {
      method = "POST",
      headers = { ["Content-Type"] = "application/json" },
      body = EncodeJson(payload)
    })
  end

  Log(kLogInfo, "Create collection response: status=" .. tostring(status) .. " body=" .. tostring(body))

  if status == 200 or status == 201 then
    -- Return updated table with success header for toast
    SetHeader("HX-Trigger", '{"showToast": {"message": "Collection created successfully", "type": "success"}}')
    self:table()
  else
    -- Log error and show toast
    Log(kLogWarn, "Failed to create collection: " .. tostring(body))
    SetHeader("HX-Trigger", '{"showToast": {"message": "Failed to create collection: ' .. (body or "Unknown error"):gsub('"', '\\"') .. '", "type": "error"}}')
    self:table()
  end
end

-- Delete collection action
function CollectionsController:destroy()
  local db = self:get_db()
  local collection = self.params.collection

  Log(kLogInfo, "Deleting collection: " .. tostring(collection))

  local status, headers, body = self:fetch_api("/_api/database/" .. db .. "/collection/" .. collection, {
    method = "DELETE"
  })

  Log(kLogInfo, "Delete collection response: status=" .. tostring(status) .. " body=" .. tostring(body))

  if status == 200 or status == 204 then
    -- Return empty string to remove the row (hx-swap="outerHTML")
    SetHeader("HX-Trigger", '{"showToast": {"message": "Collection deleted successfully", "type": "success"}}')
    self:html("")
  else
    Log(kLogWarn, "Failed to delete collection: " .. tostring(body))
    SetHeader("HX-Trigger", '{"showToast": {"message": "Failed to delete collection", "type": "error"}}')
    -- Return the row unchanged (re-fetch would be better but this is simpler)
    self:html("")
  end
end

-- Documents page
function CollectionsController:documents()
  local db = self:get_db()
  local collection = self.params.collection

  -- Get API server URL from cookie
  local api_server = GetCookie("sdb_server") or "http://localhost:6745"

  -- Fetch collection type from list endpoint
  local collection_type = "document"
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collection")
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      local collections = data.collections or data or {}
      for _, c in ipairs(collections) do
        if c.name == collection then
          collection_type = c.type or "document"
          break
        end
      end
    end
  end

  -- Check if schema exists
  local has_schema = false
  local status_schema, _, body_schema = self:fetch_api("/_api/database/" .. db .. "/collection/" .. collection .. "/schema")
  if status_schema == 200 then
    local ok, data = pcall(DecodeJson, body_schema)
    if ok and data and data.schema and next(data.schema) ~= nil then
      has_schema = true
    end
  end

  self.layout = "dashboard"
  self:render("dashboard/documents", {
    title = "Documents - " .. db,
    db = db,
    collection = collection,
    collection_type = collection_type,
    has_schema = has_schema,
    api_server = api_server,
    current_page = "documents"
  })
end

-- Documents table partial (HTMX)
function CollectionsController:documents_table()
  local db = self:get_db()
  local collection = self.params.collection
  local page = tonumber(self.params.page) or 1
  local limit = tonumber(self.params.limit) or 25
  local offset = (page - 1) * limit
  local search = self.params.search

  -- Get API server URL from cookie
  local api_server = GetCookie("sdb_server") or "http://localhost:6745"

  -- Fetch collection type from list endpoint
  local collection_type = "document"
  local coll_status, _, coll_body = self:fetch_api("/_api/database/" .. db .. "/collection")
  if coll_status == 200 then
    local ok, data = pcall(DecodeJson, coll_body)
    if ok and data then
      local collections = data.collections or data or {}
      for _, c in ipairs(collections) do
        if c.name == collection then
          collection_type = c.type or "document"
          break
        end
      end
    end
  end

  -- Construct SDBQL query (ArangoDB-like syntax)
  local query = "FOR doc IN " .. collection
  if search and search ~= "" then
    query = query .. " FILTER CONTAINS(doc._key, '" .. search .. "')"
  end
  query = query .. " LIMIT " .. limit .. " RETURN doc"

  -- Execute query via API
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/cursor", {
    method = "POST",
    body = EncodeJson({ query = query })
  })

  local documents = {}
  if status == 200 then
    local ok, res = pcall(DecodeJson, body)
    if ok and res.result then
      documents = res.result
    end
  end

  self:render_partial("dashboard/_documents_table", {
    db = db,
    collection = collection,
    collection_type = collection_type,
    api_server = api_server,
    documents = documents
  })
end

-- Create document modal
function CollectionsController:documents_modal_create()
  self:render_partial("dashboard/_modal_document", {
    db = self:get_db(),
    collection = self.params.collection,
    mode = "create",
    document_json = "{\n  \n}"
  })
end

-- Edit document modal
function CollectionsController:documents_modal_edit()
  local db = self:get_db()
  local collection = self.params.collection
  local key = self.params.key

  -- Fetch document
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/document/" .. collection .. "/" .. key)
  local doc_str = "{}"
  local meta_fields = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and type(data) == "table" then
      -- Separate system fields (starting with _) from user data
      local user_data = {}
      for k, v in pairs(data) do
        if k:sub(1, 1) == "_" then
          meta_fields[k] = v
        else
          user_data[k] = v
        end
      end
      doc_str = EncodeJson(user_data)
    end
  end

  self:render_partial("dashboard/_modal_document", {
    db = db,
    collection = collection,
    key = key,
    mode = "edit",
    meta_fields = meta_fields,
    document_json = doc_str
  })
end

-- Create document action
function CollectionsController:create_document()
  local db = self:get_db()
  local collection = self.params.collection
  local doc_json = self.params.document

  -- Parse the document
  local ok, doc = pcall(DecodeJson, doc_json)
  if not ok then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Invalid JSON", "type": "error"}}')
    return self:documents_table()
  end

  -- POST to API
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/document/" .. collection, {
    method = "POST",
    body = EncodeJson(doc)
  })

  if status == 200 or status == 201 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Document created", "type": "success"}}')
  else
    local err_msg = "Failed to create document"
    local ok_err, err_data = pcall(DecodeJson, body)
    if ok_err and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:documents_table()
end

-- Update document action
function CollectionsController:update_document()
  local db = self:get_db()
  local collection = self.params.collection
  local key = self.params.key
  local doc_json = self.params.document

  -- Parse the document
  local ok, doc = pcall(DecodeJson, doc_json)
  if not ok then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Invalid JSON", "type": "error"}}')
    return self:documents_table()
  end

  -- PUT to API
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/document/" .. collection .. "/" .. key, {
    method = "PUT",
    body = EncodeJson(doc)
  })

  if status == 200 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Document updated", "type": "success"}}')
  else
    local err_msg = "Failed to update document"
    local ok_err, err_data = pcall(DecodeJson, body)
    if ok_err and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:documents_table()
end

-- Delete document action
function CollectionsController:delete_document()
  local db = self:get_db()
  local collection = self.params.collection
  local key = self.params.key

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/document/" .. collection .. "/" .. key, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Document deleted", "type": "success"}}')
  else
    SetHeader("HX-Trigger", '{"showToast": {"message": "Failed to delete document", "type": "error"}}')
  end

  -- Refresh the full table to show empty state if needed
  self:documents_table()
end

-- Truncate collection (delete all documents)
function CollectionsController:truncate_collection()
  local db = self:get_db()
  local collection = self.params.collection

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collection/" .. collection .. "/truncate", {
    method = "PUT"
  })

  if status == 200 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Collection truncated successfully", "type": "success"}}')
  else
    SetHeader("HX-Trigger", '{"showToast": {"message": "Failed to truncate collection", "type": "error"}}')
  end

  -- Return refreshed empty table
  self:documents_table()
end

-- Blob upload modal
function CollectionsController:blob_upload_modal()
  local api_server = GetCookie("sdb_server") or "http://localhost:6745"
  self:render_partial("dashboard/_modal_blob_upload", {
    db = self:get_db(),
    collection = self.params.collection,
    api_server = api_server
  })
end

-- Columnar storage page
function CollectionsController:columnar()
  self.layout = "dashboard"
  self:render("dashboard/columnar", {
    title = "Columnar Storage - " .. self:get_db(),
    db = self:get_db(),
    current_page = "columnar"
  })
end

-- Columnar table
function CollectionsController:columnar_table()

  -- Fetch all collections to filter for columnar ones
  local status, _, body = self:fetch_api("/_api/database/" .. self:get_db() .. "/collection")
  local columnar_collections = {}

  if status == 200 then
     pcall(function()
        local all = DecodeJson(body)
        if all then
          for _, c in ipairs(all) do
            -- Check for type "columnar" or special columnar flag
            if c.type == "columnar" then
              table.insert(columnar_collections, c)
            end
          end
        end
     end)
  end

  self:render_partial("dashboard/_collections_table", {
    db = self:get_db(),
    collections = columnar_collections
  })
end

-- Schema Modal
function CollectionsController:schema_modal()
  local db = self:get_db()
  local collection = self.params.collection
  
  -- Fetch current schema
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collection/" .. collection .. "/schema")
  local current_schema = "{}"
  
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data and data.schema then
      current_schema = EncodeJson(data.schema, true) -- Pretty print
    end
  end
  
  -- If schema is empty/default, provide a sample
  if current_schema == "{}" then
    local sample = {
      ["$schema"] = "http://json-schema.org/draft-07/schema#",
      type = "object",
      properties = {
        _key = { type = "string" },
        name = { type = "string" },
        age = { type = "integer", minimum = 0 }
      },
      required = { "name" },
      additionalProperties = true
    }
    current_schema = EncodeJson(sample, true)
  end

  -- Fix escaped slashes in URL
  current_schema = current_schema:gsub("\\/", "/")
  
  self:render_partial("dashboard/_modal_schema", {
    db = db,
    collection = collection,
    schema = current_schema
  })
end

-- Update Schema Action
function CollectionsController:update_schema()
  local db = self:get_db()
  local collection = self.params.collection
  local schema_json = self.params.schema
  
  -- Validate JSON
  local ok, schema_obj = pcall(DecodeJson, schema_json)
  if not ok then
      SetHeader("HX-Trigger", '{"showToast": {"message": "Invalid JSON schema syntax", "type": "error"}}')
      -- Return re-rendered modal with error? Or just keep it open?
      -- For now, just error toast.
      return self:schema_modal()
  end
  
  local payload = {
    schema = schema_obj,
    validationMode = "strict"
  }

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collection/" .. collection .. "/schema", {
    method = "POST",
    headers = { ["Content-Type"] = "application/json" },
    body = EncodeJson(payload)
  })
  
  if status == 200 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Schema updated successfully", "type": "success"}, "closeModal": true}')
    self:html("")
    return
  else
    local err_msg = "Failed to update schema"
    local ok_err, err_data = pcall(DecodeJson, body)
    if ok_err and err_data and err_data.error then
      err_msg = err_data.error
    end
    
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg .. '", "type": "error"}}')
    return self:schema_modal()
  end
end
  
-- Delete Schema Action
function CollectionsController:delete_schema()
  local db = self:get_db()
  local collection = self.params.collection
  
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collection/" .. collection .. "/schema", {
    method = "DELETE"
  })
  
  if status == 200 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Schema removed successfully", "type": "success"}, "closeModal": true}')
    self:html("")
  else
    local err_msg = "Failed to remove schema"
    local ok_err, err_data = pcall(DecodeJson, body)
    if ok_err and err_data and err_data.error then
      err_msg = err_data.error
    end
    
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
    -- Re-render modal to keep it open
    return self:schema_modal()
  end
end

return CollectionsController
