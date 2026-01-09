-- CRUDs Controller
-- Dynamic CRUD builder for admin applications

local Controller = require("controller")
local CrudsController = Controller:extend()
local AuthHelper = require("helpers.auth_helper")
local Datatype = require("models.datatype")

-- Before action: set layout and load datatypes for sidebar
function CrudsController:before_action()
  self.layout = "cruds"
  -- Load all datatypes for sidebar
  self.datatypes = Datatype:new():order("doc.name ASC"):all()
  -- Get current user (optional auth check)
  self.current_user = AuthHelper.get_current_user()
end

-- List all datatypes
function CrudsController:index()
  local datatypes = Datatype:new():order("doc.name ASC"):all()

  -- Get record counts for each datatype
  for _, dt in ipairs(datatypes) do
    dt.record_count = dt:records_count()
  end

  self.current_page = "index"
  self:render("cruds/index", {
    title = "Datatypes - CRUDs Builder",
    datatypes = datatypes
  })
end

-- New datatype form
function CrudsController:new_datatype()
  self.current_page = "new_datatype"
  self:render("cruds/new_datatype", {
    title = "New Datatype - CRUDs Builder",
    field_types = Datatype.FIELD_TYPES,
    datatype = {}
  })
end

-- Create datatype
function CrudsController:create_datatype()
  local name = self.params.name
  local slug = self.params.slug or Datatype.slugify(name)
  local description = self.params.description or ""
  local collection_name = self.params.collection_name
  if collection_name == "" then collection_name = nil end

  -- Parse fields JSON
  local fields = {}
  if self.params.fields and self.params.fields ~= "" then
    local ok, parsed = pcall(DecodeJson, self.params.fields)
    if ok and parsed then
      fields = parsed
    end
  end

  -- Parse relations JSON
  local relations = {}
  if self.params.relations and self.params.relations ~= "" then
    local ok, parsed = pcall(DecodeJson, self.params.relations)
    if ok and parsed then
      relations = parsed
    end
  end

  local datatype = Datatype:new({
    name = name,
    slug = slug,
    description = description,
    collection_name = collection_name,
    fields = fields,
    relations = relations
  })

  -- Generate JSON schema
  datatype.data.json_schema = datatype:generate_json_schema()

  datatype:save()

  if #datatype.errors > 0 then
    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = "Error creating datatype: " .. datatype.errors[1].message, type = "error" }
    }))
    self.current_page = "new_datatype"
    self:render("cruds/new_datatype", {
      title = "New Datatype - CRUDs Builder",
      field_types = Datatype.FIELD_TYPES,
      datatype = datatype,
      errors = datatype.errors
    })
  else
    self:set_header("HX-Redirect", "/cruds")
    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = "Datatype created successfully", type = "success" }
    }))
    self:html("")
  end
end

-- Edit datatype form
function CrudsController:edit_datatype()
  local slug = self.params.slug
  local datatype = Datatype.find_by_slug(slug)

  if not datatype then
    self:set_header("HX-Redirect", "/cruds")
    return self:html("")
  end

  self.current_page = "edit_datatype"
  self.current_datatype = slug
  self:render("cruds/edit_datatype", {
    title = "Edit " .. datatype.name .. " - CRUDs Builder",
    field_types = Datatype.FIELD_TYPES,
    datatype = datatype
  })
end

-- Update datatype
function CrudsController:update_datatype()
  local slug = self.params.slug
  local datatype = Datatype.find_by_slug(slug)

  if not datatype then
    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = "Datatype not found", type = "error" }
    }))
    return self:html("")
  end

  local name = self.params.name
  local new_slug = self.params.new_slug or slug
  local description = self.params.description or ""
  local collection_name = self.params.collection_name
  if collection_name == "" then collection_name = nil end

  -- Parse fields JSON
  local fields = {}
  if self.params.fields and self.params.fields ~= "" then
    local ok, parsed = pcall(DecodeJson, self.params.fields)
    if ok and parsed then
      fields = parsed
    end
  end

  -- Parse relations JSON
  local relations = {}
  if self.params.relations and self.params.relations ~= "" then
    local ok, parsed = pcall(DecodeJson, self.params.relations)
    if ok and parsed then
      relations = parsed
    end
  end

  local update_data = {
    name = name,
    slug = new_slug,
    description = description,
    collection_name = collection_name,
    fields = fields,
    relations = relations
  }

  -- Regenerate JSON schema
  datatype.data.fields = fields
  update_data.json_schema = datatype:generate_json_schema()

  local success = datatype:update(update_data)

  if success then
    self:set_header("HX-Redirect", "/cruds")
    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = "Datatype updated successfully", type = "success" }
    }))
    self:html("")
  else
    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = "Error updating datatype", type = "error" }
    }))
    self.current_page = "edit_datatype"
    self:render("cruds/edit_datatype", {
      title = "Edit " .. datatype.name .. " - CRUDs Builder",
      field_types = Datatype.FIELD_TYPES,
      datatype = datatype,
      errors = datatype.errors
    })
  end
end

-- Delete datatype
function CrudsController:delete_datatype()
  local slug = self.params.slug
  local datatype = Datatype.find_by_slug(slug)

  if not datatype then
    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = "Datatype not found", type = "error" }
    }))
    return self:html("")
  end

  -- Optionally delete associated data
  if self.params.delete_data == "true" then
    local collection = datatype:target_collection()
    if collection == "datasets" then
      Sdb:Sdbql("FOR doc IN datasets FILTER doc._type == @type REMOVE doc IN datasets", { type = slug })
    else
      -- Be careful with custom collections
      Sdb:Sdbql("FOR doc IN `" .. collection .. "` REMOVE doc IN `" .. collection .. "`", {})
    end
  end

  datatype:destroy()

  self:set_header("HX-Redirect", "/cruds")
  self:set_header("HX-Trigger", EncodeJson({
    showToast = { message = "Datatype deleted successfully", type = "success" }
  }))
  self:html("")
end

-- JSON Schema editor
function CrudsController:schema_editor()
  local slug = self.params.slug
  local datatype = Datatype.find_by_slug(slug)

  if not datatype then
    self:set_header("HX-Redirect", "/cruds")
    return self:html("")
  end

  self.current_page = "schema_editor"
  self.current_datatype = slug
  self:render("cruds/schema_editor", {
    title = "JSON Schema - " .. datatype.name .. " - CRUDs Builder",
    datatype = datatype
  })
end

-- Update JSON schema
function CrudsController:update_schema()
  local slug = self.params.slug
  local datatype = Datatype.find_by_slug(slug)

  if not datatype then
    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = "Datatype not found", type = "error" }
    }))
    return self:html("")
  end

  -- Parse new schema
  local schema_json = self.params.json_schema or "{}"
  local ok, schema = pcall(DecodeJson, schema_json)

  if not ok then
    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = "Invalid JSON schema", type = "error" }
    }))
    return self:html("")
  end

  datatype:update({ json_schema = schema })

  self:set_header("HX-Trigger", EncodeJson({
    showToast = { message = "JSON schema updated successfully", type = "success" }
  }))
  self:html("")
end

--------------------------------------------------------------------------------
-- DATA CRUD Actions
--------------------------------------------------------------------------------

-- List records for a datatype
function CrudsController:data_index()
  local slug = self.params.datatype_slug
  local datatype = Datatype.find_by_slug(slug)

  if not datatype then
    self:set_header("HX-Redirect", "/cruds")
    return self:html("")
  end

  local page = tonumber(self.params.page) or 1
  local per_page = tonumber(self.params.per_page) or 30

  local records = datatype:get_records({ page = page, per_page = per_page })
  local total = datatype:records_count()
  local total_pages = math.ceil(total / per_page)

  self.current_datatype = slug
  self:render("cruds/data_index", {
    title = datatype.name .. " - CRUDs Builder",
    datatype = datatype,
    records = records,
    page = page,
    per_page = per_page,
    total = total,
    total_pages = total_pages
  })
end

-- New record form
function CrudsController:data_new()
  local slug = self.params.datatype_slug
  local datatype = Datatype.find_by_slug(slug)

  if not datatype then
    self:set_header("HX-Redirect", "/cruds")
    return self:html("")
  end

  self.current_datatype = slug
  self:render("cruds/data_new", {
    title = "New " .. datatype.name .. " - CRUDs Builder",
    datatype = datatype,
    record = {}
  })
end

-- Create record
function CrudsController:data_create()
  local slug = self.params.datatype_slug
  local datatype = Datatype.find_by_slug(slug)

  if not datatype then
    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = "Datatype not found", type = "error" }
    }))
    return self:html("")
  end

  -- Collect field data
  local data = {}
  local fields = datatype.fields or datatype.data.fields or {}

  for _, field in ipairs(fields) do
    local value = self.params["data_" .. field.name]

    -- Handle different field types
    if field.type == "tags" or field.type == "multilist" then
      -- Parse as array (comma-separated or JSON)
      if value and value ~= "" then
        local ok, arr = pcall(DecodeJson, value)
        if ok and type(arr) == "table" then
          data[field.name] = arr
        else
          -- Try comma-separated
          local items = {}
          for item in value:gmatch("[^,]+") do
            table.insert(items, item:match("^%s*(.-)%s*$"))
          end
          data[field.name] = items
        end
      else
        data[field.name] = {}
      end
    elseif field.type == "gps" then
      -- Parse lat/lng
      local lat = tonumber(self.params["data_" .. field.name .. "_lat"])
      local lng = tonumber(self.params["data_" .. field.name .. "_lng"])
      if lat and lng then
        data[field.name] = { lat = lat, lng = lng }
      end
    elseif field.type == "images" or field.type == "files" then
      -- Parse as JSON array of file keys
      if value and value ~= "" then
        local ok, arr = pcall(DecodeJson, value)
        if ok and type(arr) == "table" then
          data[field.name] = arr
        else
          data[field.name] = {}
        end
      else
        data[field.name] = {}
      end
    else
      data[field.name] = value
    end
  end

  -- Handle embedded relations
  local relations = datatype.relations or datatype.data.relations or {}
  for _, rel in ipairs(relations) do
    if rel.storage == "embedded" then
      local rel_data = self.params["rel_" .. rel.name]
      if rel_data and rel_data ~= "" then
        local ok, arr = pcall(DecodeJson, rel_data)
        if ok and type(arr) == "table" then
          data[rel.name] = arr
        end
      end
    end
  end

  local record = datatype:create_record(data)

  if record and not record.errors then
    self:set_header("HX-Redirect", "/cruds/data/" .. slug)
    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = "Record created successfully", type = "success" }
    }))
    self:html("")
  else
    local err_msg = "Error creating record"
    if record and record.errors then
       err_msg = table.concat(record.errors, ", ")
    end

    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = err_msg, type = "error" }
    }))
    self.current_datatype = slug
    self:render("cruds/data_new", {
      title = "New " .. datatype.name .. " - CRUDs Builder",
      datatype = datatype,
      record = data,
      errors = record and record.errors
    })
  end
end

-- Show record
function CrudsController:data_show()
  local slug = self.params.datatype_slug
  local key = self.params.key
  local datatype = Datatype.find_by_slug(slug)

  if not datatype then
    self:set_header("HX-Redirect", "/cruds")
    return self:html("")
  end

  local record = datatype:find_record(key)

  if not record then
    self:set_header("HX-Redirect", "/cruds/data/" .. slug)
    return self:html("")
  end

  self.current_datatype = slug
  self:render("cruds/data_show", {
    title = record._key .. " - " .. datatype.name .. " - CRUDs Builder",
    datatype = datatype,
    record = record
  })
end

-- Edit record form
function CrudsController:data_edit()
  local slug = self.params.datatype_slug
  local key = self.params.key
  local datatype = Datatype.find_by_slug(slug)

  if not datatype then
    self:set_header("HX-Redirect", "/cruds")
    return self:html("")
  end

  local record = datatype:find_record(key)

  if not record then
    self:set_header("HX-Redirect", "/cruds/data/" .. slug)
    return self:html("")
  end

  self.current_datatype = slug
  self:render("cruds/data_edit", {
    title = "Edit " .. record._key .. " - " .. datatype.name .. " - CRUDs Builder",
    datatype = datatype,
    record = record
  })
end

-- Update record
function CrudsController:data_update()
  local slug = self.params.datatype_slug
  local key = self.params.key
  local datatype = Datatype.find_by_slug(slug)

  if not datatype then
    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = "Datatype not found", type = "error" }
    }))
    return self:html("")
  end

  -- Collect field data (same as create)
  local data = {}
  local fields = datatype.fields or datatype.data.fields or {}

  for _, field in ipairs(fields) do
    local value = self.params["data_" .. field.name]

    if field.type == "tags" or field.type == "multilist" then
      if value and value ~= "" then
        local ok, arr = pcall(DecodeJson, value)
        if ok and type(arr) == "table" then
          data[field.name] = arr
        else
          local items = {}
          for item in value:gmatch("[^,]+") do
            table.insert(items, item:match("^%s*(.-)%s*$"))
          end
          data[field.name] = items
        end
      else
        data[field.name] = {}
      end
    elseif field.type == "gps" then
      local lat = tonumber(self.params["data_" .. field.name .. "_lat"])
      local lng = tonumber(self.params["data_" .. field.name .. "_lng"])
      if lat and lng then
        data[field.name] = { lat = lat, lng = lng }
      end
    elseif field.type == "images" or field.type == "files" then
      if value and value ~= "" then
        local ok, arr = pcall(DecodeJson, value)
        if ok and type(arr) == "table" then
          data[field.name] = arr
        else
          data[field.name] = {}
        end
      else
        data[field.name] = {}
      end
    else
      data[field.name] = value
    end
  end

  -- Handle embedded relations
  local relations = datatype.relations or datatype.data.relations or {}
  for _, rel in ipairs(relations) do
    if rel.storage == "embedded" then
      local rel_data = self.params["rel_" .. rel.name]
      if rel_data and rel_data ~= "" then
        local ok, arr = pcall(DecodeJson, rel_data)
        if ok and type(arr) == "table" then
          data[rel.name] = arr
        end
      end
    end
  end

  local record = datatype:update_record(key, data)

  if record and not record.errors then
    -- Skip redirect for auto-save requests (embedded relations changes)
    if not self.params.autosave then
      self:set_header("HX-Redirect", "/cruds/data/" .. slug)
    end
    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = "Record updated successfully", type = "success" }
    }))
    return self:html("")
  else
    local err_msg = "Error updating record"
    if record and record.errors then
       err_msg = table.concat(record.errors, ", ")
    end

    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = err_msg, type = "error" }
    }))
    local existing = datatype:find_record(key)
    self.current_datatype = slug
    self:render("cruds/data_edit", {
      title = "Edit " .. key .. " - " .. datatype.name .. " - CRUDs Builder",
      datatype = datatype,
      record = existing or data,
      errors = record and record.errors
    })
  end
end

-- Delete record
function CrudsController:data_delete()
  local slug = self.params.datatype_slug
  local key = self.params.key
  local datatype = Datatype.find_by_slug(slug)

  if not datatype then
    self:set_header("HX-Trigger", EncodeJson({
      showToast = { message = "Datatype not found", type = "error" }
    }))
    return self:html("")
  end

  datatype:delete_record(key)

  self:set_header("HX-Trigger", EncodeJson({
    showToast = { message = "Record deleted successfully", type = "success" }
  }))
  self:html("")
end

-- Get upload config for JavaScript
function CrudsController:upload_config()
  local db_name = Sdb._db_config and Sdb._db_config.db_name or "_system"
  local api_server = Sdb._db_config and Sdb._db_config.url or "http://localhost:6745"

  self:json({
    db_name = db_name,
    api_server = api_server,
    collection = "_uploads"
  })
end

-- File proxy for viewing uploaded files
function CrudsController:file_proxy()
  local key = self.params.key
  local db_name = Sdb._db_config and Sdb._db_config.db_name or "_system"
  local db_url = Sdb._db_config and Sdb._db_config.url or "http://localhost:6745"

  -- Get token for blob access
  local token = Sdb:LiveQueryToken()

  -- Redirect to blob URL
  local blob_url = db_url .. "/_api/blob/" .. db_name .. "/_uploads/" .. key
  return self:redirect(blob_url .. "?token=" .. token)
end

return CrudsController
