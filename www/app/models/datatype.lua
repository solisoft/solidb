local Model = require("model")

local Datatype = Model.create("datatypes", {
  permitted_fields = {
    "name", "slug", "description", "collection_name",
    "fields", "relations", "json_schema"
  },
  validations = {
    name = { presence = true, length = { between = {1, 100} } },
    slug = { presence = true, length = { between = {1, 50} }, format = "^[a-z0-9_%-]+$" }
  }
})

-- Field type definitions with their default properties
Datatype.FIELD_TYPES = {
  "string",     -- Single line text
  "text",       -- Multi-line textarea
  "wysiwyg",    -- Rich text editor
  "gps",        -- GPS coordinates (lat/lng)
  "images",     -- Image upload(s)
  "files",      -- File upload(s)
  "tags",       -- Tag input (array of strings)
  "list",       -- Single select dropdown
  "multilist"   -- Multi select
}

-- Find by slug
function Datatype.find_by_slug(slug)
  return Datatype:new():where({ slug = slug }):first()
end

-- Get target collection (custom or 'datasets')
function Datatype:target_collection()
  local coll = self.collection_name or (self.data and self.data.collection_name)
  if coll and coll ~= "" then
    return coll
  end
  return "datasets"
end

-- Generate slug from name
function Datatype.slugify(name)
  if not name then return "" end
  return name:lower():gsub("[^a-z0-9]+", "_"):gsub("^_+", ""):gsub("_+$", "")
end

-- Auto-generate JSON schema from fields
function Datatype:generate_json_schema()
  local fields = self.fields or (self.data and self.data.fields) or {}

  local schema = {
    type = "object",
    properties = {},
    required = {}
  }

  for _, field in ipairs(fields) do
    local prop = { type = "string" }

    if field.type == "gps" then
      prop = {
        type = "object",
        properties = {
          lat = { type = "number" },
          lng = { type = "number" }
        }
      }
    elseif field.type == "images" or field.type == "files" or
           field.type == "tags" or field.type == "multilist" then
      prop = { type = "array", items = { type = "string" } }
    elseif field.type == "list" then
      prop = { type = "string" }
      if field.options and #field.options > 0 then
        prop.enum = field.options
      end
    end

    -- Add validation constraints
    if field.validation then
      if field.validation.min_length then
        prop.minLength = field.validation.min_length
      end
      if field.validation.max_length then
        prop.maxLength = field.validation.max_length
      end
    end

    schema.properties[field.name] = prop

    if field.required then
      table.insert(schema.required, field.name)
    end
  end

  return schema
end

-- Get record count for this datatype
function Datatype:records_count()
  local collection = self:target_collection()
  local slug = self.slug or (self.data and self.data.slug)

  if collection == "datasets" then
    local result = Sdb:Sdbql(
      "FOR doc IN datasets FILTER doc._type == @type COLLECT WITH COUNT INTO c RETURN c",
      { type = slug }
    )
    if result and result.result and result.result[1] then
      return result.result[1]
    end
    return 0
  else
    local result = Sdb:Sdbql(
      "RETURN COLLECTION_COUNT(`" .. collection .. "`)"
    )
    if result and result.result and result.result[1] then
      return result.result[1]
    end
    return 0
  end
end

-- Get records for this datatype with pagination
function Datatype:get_records(options)
  options = options or {}
  options.per_page = options.per_page or 30
  options.page = options.page or 1
  local offset = options.per_page * (options.page - 1)

  local collection = self:target_collection()
  local slug = self.slug or (self.data and self.data.slug)

  local query
  local params = { offset = offset, per_page = options.per_page }

  if collection == "datasets" then
    query = [[
      FOR doc IN datasets
        FILTER doc._type == @type
        SORT doc.created_at DESC
        LIMIT @offset, @per_page
        RETURN doc
    ]]
    params.type = slug
  else
    query = string.format([[
      FOR doc IN `%s`
        SORT doc.created_at DESC
        LIMIT @offset, @per_page
        RETURN doc
    ]], collection)
  end

  local result = Sdb:Sdbql(query, params)
  return (result and result.result) or {}
end

-- Find a single record by key
function Datatype:find_record(key)
  local collection = self:target_collection()
  local slug = self.slug or (self.data and self.data.slug)

  if collection == "datasets" then
    local result = Sdb:Sdbql(
      "FOR doc IN datasets FILTER doc._key == @key AND doc._type == @type LIMIT 1 RETURN doc",
      { key = key, type = slug }
    )
    if result and result.result and result.result[1] then
      return result.result[1]
    end
  else
    local doc = Sdb:GetDocument(collection .. "/" .. key)
    return doc
  end
  return nil
end

local JsonSchemaValidator = require("helpers.json_schema_validator")

-- Create a new record
function Datatype:create_record(data)
  local collection = self:target_collection()
  local slug = self.slug or (self.data and self.data.slug)
  local schema = self.json_schema or (self.data and self.data.json_schema)

  -- Validate against JSON Schema if present
  if schema and next(schema) then
    local ok, errors = JsonSchemaValidator.validate(schema, data)
    if not ok then
      return { errors = errors }
    end
  end

  -- Add metadata
  data.created_at = os.time()
  data.updated_at = os.time()

  if collection == "datasets" then
    data._type = slug
  end

  local result = Sdb:CreateDocument(collection, data)
  if result and result.new then
    return result.new
  end
  return result
end

-- Update a record
function Datatype:update_record(key, data)
  local collection = self:target_collection()
  local schema = self.json_schema or (self.data and self.data.json_schema)

  -- Validate against JSON Schema if present
  -- Note: partial updates might need partial schema validation, but for now we validate the passed data
  -- Ideally we should merge with existing data before validation if it's a patch, 
  -- but here 'data' contains fields to update. 
  -- Simplification: we only validate the fields being updated directly if possible, 
  -- OR strictly we might need to fetch -> merge -> validate -> save.
  -- For now, let's just validate the fields present if strict, or maybe skip required check?
  -- Let's stick to validating what we have, but be aware 'required' check might fail on partial updates.
  -- Actually, CrudsController sends all fields in edit form usually.
  
  if schema and next(schema) then
      local ok, errors = JsonSchemaValidator.validate(schema, data)
      if not ok then
        return { errors = errors }
      end
  end

  data.updated_at = os.time()

  local result = Sdb:UpdateDocument(collection .. "/" .. key, data)
  if result and result.new then
    return result.new
  end
  return result
end

-- Delete a record
function Datatype:delete_record(key)
  local collection = self:target_collection()
  return Sdb:DeleteDocument(collection .. "/" .. key)
end

-- Get related datatype for a relation
function Datatype:get_related_datatype(relation_name)
  local relations = self.relations or (self.data and self.data.relations) or {}

  for _, rel in ipairs(relations) do
    if rel.name == relation_name then
      return Datatype.find_by_slug(rel.target_datatype)
    end
  end
  return nil
end

-- Get embedded relation data
function Datatype:get_embedded_relation(record, relation_name)
  if record and record[relation_name] then
    return record[relation_name]
  end
  return {}
end

-- Get referenced relation data
function Datatype:get_referenced_relation(record_key, relation)
  local target = Datatype.find_by_slug(relation.target_datatype)
  if not target then return {} end

  local collection = target:target_collection()
  local fk = relation.foreign_key or (self.slug .. "_id")

  local query
  local params = { key = record_key }

  if collection == "datasets" then
    query = [[
      FOR doc IN datasets
        FILTER doc._type == @type AND doc[@fk] == @key
        SORT doc.created_at DESC
        RETURN doc
    ]]
    params.type = relation.target_datatype
    params.fk = fk
  else
    query = string.format([[
      FOR doc IN `%s`
        FILTER doc[@fk] == @key
        SORT doc.created_at DESC
        RETURN doc
    ]], collection)
    params.fk = fk
  end

  local result = Sdb:Sdbql(query, params)
  return (result and result.result) or {}
end

return Datatype
