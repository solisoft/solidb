-- Dashboard Indexes Controller
-- Handles index management for collections
local DashboardBaseController = require("dashboard.base_controller")
local IndexesController = DashboardBaseController:extend()

-- Collection indexes page
function IndexesController:collection_indexes()
  local db = self:get_db()
  local collection = self.params.collection
  local is_columnar = self:is_columnar_collection(db, collection)

  self.layout = "dashboard"
  self:render("dashboard/collection_indexes", {
    title = "Indexes - " .. collection .. " - " .. db,
    db = db,
    collection = collection,
    is_columnar = is_columnar,
    current_page = "collections"
  })
end

-- Collection indexes table (HTMX)
function IndexesController:collection_indexes_table()
  local db = self:get_db()
  local collection = self.params.collection
  local is_columnar = self:is_columnar_collection(db, collection)

  local all_indexes = {}

  if is_columnar then
    -- Fetch columnar indexes
    local status, _, body = self:fetch_api("/_api/database/" .. db .. "/columnar/" .. collection .. "/indexes")
    if status == 200 then
      local ok, data = pcall(DecodeJson, body)
      if ok and data then
        local indexes = data.indexes or data or {}
        for _, idx in ipairs(indexes) do
          idx.index_type = "columnar"
          table.insert(all_indexes, idx)
        end
      end
    end
  else
    -- Regular indexes
    local status, _, body = self:fetch_api("/_api/database/" .. db .. "/index/" .. collection)
    if status == 200 then
      local ok, data = pcall(DecodeJson, body)
      if ok and data then
        local indexes = data.indexes or data or {}
        for _, idx in ipairs(indexes) do
          idx.index_type = idx.type or "hash"
          table.insert(all_indexes, idx)
        end
      end
    end

    -- Geo indexes
    status, _, body = self:fetch_api("/_api/database/" .. db .. "/geo/" .. collection)
    if status == 200 then
      local ok, data = pcall(DecodeJson, body)
      if ok and data then
        local indexes = data.indexes or data or {}
        for _, idx in ipairs(indexes) do
          idx.index_type = "geo"
          table.insert(all_indexes, idx)
        end
      end
    end

    -- TTL indexes
    status, _, body = self:fetch_api("/_api/database/" .. db .. "/ttl/" .. collection)
    if status == 200 then
      local ok, data = pcall(DecodeJson, body)
      if ok and data then
        local indexes = data.indexes or data or {}
        for _, idx in ipairs(indexes) do
          idx.index_type = "ttl"
          table.insert(all_indexes, idx)
        end
      end
    end
  end

  self:render_partial("dashboard/_collection_indexes_table", {
    db = db,
    collection = collection,
    indexes = all_indexes,
    is_columnar = is_columnar
  })
end

-- Create index modal
function IndexesController:collection_indexes_modal_create()
  local db = self:get_db()
  local collection = self.params.collection
  local is_columnar = self:is_columnar_collection(db, collection)

  self:render_partial("dashboard/_modal_create_index", {
    db = db,
    collection = collection,
    is_columnar = is_columnar
  })
end

-- Create collection index action
function IndexesController:create_collection_index()
  local db = self:get_db()
  local collection = self.params.collection
  local index_type = self.params.index_type
  local fields = self.params.fields
  local name = self.params.name
  local unique = self.params.unique == "on" or self.params.unique == "true"

  -- Build index payload based on type
  local payload = {}
  local endpoint = ""

  if index_type == "columnar" then
    -- Columnar index - just needs column name
    local column = self.params.column
    endpoint = "/_api/database/" .. db .. "/columnar/" .. collection .. "/index"
    payload = {
      column = column
    }
  elseif index_type == "geo" then
    endpoint = "/_api/database/" .. db .. "/geo/" .. collection
    payload = {
      fields = { fields },  -- geo expects single field
      name = name
    }
  elseif index_type == "ttl" then
    endpoint = "/_api/database/" .. db .. "/ttl/" .. collection
    local ttl = tonumber(self.params.ttl) or 3600
    payload = {
      field = fields,
      ttl = ttl,
      name = name
    }
  else
    -- hash or persistent or fulltext
    endpoint = "/_api/database/" .. db .. "/index/" .. collection
    -- Parse comma-separated fields
    local field_list = {}
    for field in fields:gmatch("[^,]+") do
      table.insert(field_list, field:match("^%s*(.-)%s*$"))  -- trim
    end
    payload = {
      type = index_type,
      fields = field_list,
      name = name,
      unique = unique
    }
  end

  local status, _, body = self:fetch_api(endpoint, {
    method = "POST",
    body = EncodeJson(payload)
  })

  if status == 200 or status == 201 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Index created", "type": "success"}}')
  else
    local err_msg = "Failed to create index"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg .. '", "type": "error"}}')
  end

  self:collection_indexes_table()
end

-- Delete collection index action
function IndexesController:delete_collection_index()
  local db = self:get_db()
  local collection = self.params.collection
  local index_name = self.params.index_name
  local index_type = self.params.type or "hash"

  -- Determine endpoint based on type
  local endpoint
  if index_type == "columnar" then
    endpoint = "/_api/database/" .. db .. "/columnar/" .. collection .. "/index/" .. index_name
  elseif index_type == "geo" then
    endpoint = "/_api/database/" .. db .. "/geo/" .. collection .. "/" .. index_name
  elseif index_type == "ttl" then
    endpoint = "/_api/database/" .. db .. "/ttl/" .. collection .. "/" .. index_name
  else
    endpoint = "/_api/database/" .. db .. "/index/" .. collection .. "/" .. index_name
  end

  local status, _, body = self:fetch_api(endpoint, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Index deleted", "type": "success"}}')
    self:html("")  -- Remove the row
  else
    SetHeader("HX-Trigger", '{"showToast": {"message": "Failed to delete index", "type": "error"}}')
    self:html("")
  end
end

-- Database-wide indexes page
function IndexesController:index()
  self.layout = "dashboard"
  self:render("dashboard/indexes", {
    title = "Indexes - " .. self:get_db(),
    db = self:get_db(),
    current_page = "indexes"
  })
end

-- Indexes table
function IndexesController:table()
  -- Fetch indexes (placeholder)
  self:render_partial("dashboard/_empty_state", { message = "No indexes found" })
end

return IndexesController
