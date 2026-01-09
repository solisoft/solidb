local SoliDBModel = {}
SoliDBModel = setmetatable({}, { __index = SoliDBModel })

-- Save built-in functions before overriding
local builtin_table_concat = table.concat

-- Table utility functions
function table.merge(t1, t2)
  local result = {}
  for k, v in pairs(t1) do result[k] = v end
  for k, v in pairs(t2) do result[k] = v end
  return result
end

function table.append(t1, t2)
  local result = {}
  for i, v in ipairs(t1) do table.insert(result, v) end
  for i, v in ipairs(t2) do table.insert(result, v) end
  return result
end

-- Custom concat for table filter arrays (preserves separator support)
function table.concat(t, sep)
  -- If separator provided, use builtin for standard behavior
  if sep ~= nil then
    return builtin_table_concat(t, sep)
  end

  if not t or next(t) == nil then return "" end
  local result = {}
  for _, v in ipairs(t) do
    if type(v) == "table" then
      table.insert(result, v[1] or v.filter or tostring(v))
    else
      table.insert(result, tostring(v))
    end
  end
  return builtin_table_concat(result)
end


function table.keys(tbl)
  local result = {}
  for k, _ in pairs(tbl) do table.insert(result, k) end
  return result
end

function table.contains(tbl, value)
  for _, v in pairs(tbl) do
    if v == value then return true end
  end
  return false
end

function SoliDBModel.new(data)
  local self = setmetatable({}, SoliDBModel)
  self.filters = {}
  self.bindvars = {}
  self.sort = "doc._id ASC"
  self.data = data or {}
  self.var_index = 0
  self.global_callbacks = {
    before_create = { "run_before_create_callback" },
    after_create = {},
    before_update = { "run_before_update_callback"  },
    after_update = {}
  }
  self.callbacks = { before_create = {}, before_update = {}, after_create = {}, after_update = {} }
  self.errors = {}
  self.validations = {}
  return self
end

function SoliDBModel.run_before_create_callback(data)
  return data
end

function SoliDBModel.run_before_update_callback(data)
  return data
end

function SoliDBModel:first()
  local list = self:all({ per_page = 1 })
  if list and #list > 0 then
    return list[1]
  end
  return nil
end

function SoliDBModel:last()
  self.sort = "doc._id DESC"
  local list = self:all({ per_page = 1 })
  if list and #list > 0 then
    return list[1]
  end
  return nil
end

function SoliDBModel:any()
  return self:count() > 0
end

function SoliDBModel:all(options)
  options = options or {}
  options.per_page = options.per_page or 30
  options.page = options.page or 1
  options.collection = options.collection or self.COLLECTION

  local offset = options.per_page * (options.page - 1)

  -- Use defaults if not set (handles class-level calls)
  local sort = self.sort or "doc._id ASC"
  local filters = self.filters or {}
  local bindvars = self.bindvars or {}

  local filters_str = table.concat(filters)
  local request = Sdb:Sdbql(
    "FOR doc IN `" .. options.collection .. "`\n" .. filters_str .. "\n" ..
    "SORT " .. sort .. "\n" ..
    "LIMIT @offset, @per_page\n" ..
    "RETURN doc",
    table.merge({
      ["per_page"] = options.per_page,
      ["offset"] = offset
    }, bindvars)
  )


  local instances = {}
  for _, doc in ipairs(request.result) do
    table.insert(instances, self:new(doc))
  end

  -- For backward compatibility with tests that expect result.data
  setmetatable(instances, {
    __index = function(t, k)
      if k == "data" then return t end
      return nil
    end
  })

  return instances
end

function SoliDBModel:find(handler)
  assert(not self.data or next(self.data) == nil, "find not allowed here")

  if not handler then return nil end

  local prefix = self.COLLECTION .. "/"
  if string.sub(handler, 1, #prefix) ~= prefix then
    handler = prefix .. handler
  end

  local doc = Sdb:GetDocument(handler)
  if not doc then return nil end
  self.data = doc
  return self
end

function SoliDBModel:find_by(criteria)
  return self:where(criteria):first()
end

-- filtering

function SoliDBModel:where(criteria)
  assert(criteria, "you must specify criteria")
  assert(type(criteria) == "table", "criteria must be a table")

  for k, v in pairs(criteria) do
    self.var_index = self.var_index + 1
    local var_name = string.format("@data_%s", self.var_index)
    local bindvar_name = string.format("data_%s", self.var_index)

    local filter = ""
    filter = string.format(" FILTER doc.%s == %s", k, var_name)
    self.bindvars = table.merge(self.bindvars, { [bindvar_name] = v })

    if type(v) == "table" then
      filter = string.format(" FILTER doc.%s IN %s", k, var_name)
      self.bindvars = table.merge(self.bindvars, { [bindvar_name] = v })
    end

    self.filters = table.append(self.filters, { filter })
  end

  return self
end

function SoliDBModel:where_not(criteria)
  assert(criteria, "you must specify criteria")
  assert(type(criteria) == "table", "criteria must be a table")

  for k, v in pairs(criteria) do
    self.var_index = self.var_index + 1
    local var_name = string.format("@data_%s", self.var_index)
    local bindvar_name = string.format("data_%s", self.var_index)

    local filter = ""
    filter = string.format(" FILTER doc.%s != %s", k, var_name)
    self.bindvars = table.merge(self.bindvars, { [bindvar_name] = v })

    if type(v) == "table" then
      filter = string.format(" FILTER doc.%s NOT IN %s", k, var_name)
      self.bindvars = table.merge(self.bindvars, { [bindvar_name] = v })
    end

    self.filters = table.append(self.filters, { filter })
  end

  return self
end

function SoliDBModel:filter_by(criteria, sign)
  assert(criteria, "you must specify criteria")
  assert(type(criteria) == "table", "criteria must be a table")

  for k, v in pairs(criteria) do
    self.var_index = self.var_index + 1
    local var_name = string.format("@data_%s", self.var_index)
    local bindvar_name = string.format("data_%s", self.var_index)

    local filter = ""
    filter = string.format(" FILTER doc.%s %s %s", k, sign, var_name)
    self.bindvars = table.merge(self.bindvars, { [bindvar_name] = v })
    self.filters = table.append(self.filters, { filter })
  end

  return self
end

function SoliDBModel:gt(criteria)
  return self:filter_by(criteria, ">")
end

function SoliDBModel:lt(criteria)
  return self:filter_by(criteria, "<")
end

function SoliDBModel:lte(criteria)
  return self:filter_by(criteria, "<=")
end

function SoliDBModel:gte(criteria)
  return self:filter_by(criteria, ">=")
end

-- sorting

function SoliDBModel:order(sort)
  self.sort = sort
  return self
end

-- validations

function SoliDBModel:validates_each(data)
  self.errors = {}
  for field, validations in pairs(self.validations) do
    local value = data[field]
    for k, v in pairs(validations) do
      if k == "presence" then
        local default_error = I18n:t("models.errors.presence")
        if v == true then v = { message = default_error } end
        if v.message == nil then v.message = default_error end
        if value == nil or value == "" then
          self.errors = table.append(self.errors, {{ field = field, message = v.message }})
        end
      end

      if k == "numericality" then
        local default_error = I18n:t("models.errors.numericality.valid_number")
        if type(value) ~= "number" then
          if type(v) == "number" then v = { message = default_error } end
          if v.message == nil then v = { message = "must be a valid number" } end
          self.errors = table.append(self.errors, {{ field = field, message = v.message }})
        end

        if type(v) == "table" and v.only_integer ~= nil then
          if v.message == nil then v = { message = I18n:t("models.errors.numericality.valid_integer") } end
          if math.type(value) ~= "integer" then
            self.errors = table.append(self.errors, {{ field = field, message = v.message }})
          end
        end
      end

      if k == "length" then
        -- Skip length validation if value is nil (let presence handle it)
        if value == nil or value == "" then
          -- Don't validate length on nil values
        else
        local default_error = I18n:t("models.errors.length.eq")
        if type(v) == "number" then v = { eq = v, message = string.format(default_error, v) } end

        if table.contains(table.keys(v), "eq") then
          if v.message == nil then v.message = string.format(default_error, v) end
          if #value ~= v.eq then
            if v.message == nil then v.message = string.format(default_error, v.eq) end
            self.errors = table.append(self.errors, {{ field = field, message = v.message }})
          end
        end

        if table.contains(table.keys(v), "between") then
          assert(type(v["between"]) == "table", "'between' argument must be a table of 2 arguments")
          assert(#v["between"] == 2, "'between' argument must be a table of 2 arguments")
          assert(type(v["between"][1]) == "number" and type(v["between"][2]) == "number", "'between' arguments must be numbers")

          if #value < v["between"][1] or #value > v["between"][2] then
            if v.message == nil then v.message = I18n:t("models.errors.length.between", v["between"][1], v["between"][2]) end
            self.errors = table.append(self.errors, {{ field = field, message = v.message }})
          end
        end

        if table.contains(table.keys(v), "minimum") then
          assert(type(v.minimum) == "number", "'minimum' argument must be a number")
          if #value < v.minimum then
            if v.message == nil then v.message = I18n:t("models.errors.length.minimum", v.minimum)  end
            self.errors = table.append(self.errors, {{ field = field, message = v.message }})
          end
        end

        if table.contains(table.keys(v), "maximum") then
          assert(type(v.maximum) == "number", "'maximum' argument must be a number")
          if #value > v.maximum then
            if v.message == nil then v.message = I18n:t("models.errors.length.maximum", v.maximum) end
            self.errors = table.append(self.errors, {{ field = field, message = v.message }})
          end
        end
        end -- end nil guard
      end

      if k == "format" then
        local default_error = I18n:t("models.errors.format")
        if type(v) == "string" then v = { re = v } end
        if v.message == nil then v.message = default_error end
        if re and re.compile then
          local regex = assert(re.compile(v.re))
          local match = regex:search(value or "")
          if match == nil then
            self.errors = table.append(self.errors, {{ field = field, message = default_error }})
          end
        end
      end

      if k == "comparaison" then
        local default_error = I18n:t("models.errors.comparaison")
        if type(v) == "string" then v = { eq = v } end
        if v.message == nil then v.message = default_error end

        if v.eq then
          if self.data then
            if not table.contains(table.keys(self.data), v.eq) and table.contains(table.keys(data), v.eq) then
              self.data[v.eq] = data[v.eq]
            else
              if not table.contains(table.keys(self.data), v.eq) then self.data[v.eq] = nil end
            end
          end
          local against_data = self.data and self.data[v.eq] or data[v.eq]

          if data[field] ~= against_data then
            self.errors = table.append(self.errors, {{ field = field, message = default_error }})
          end
        end

        if v.gt then
          if self.data then
            if not table.contains(table.keys(self.data), v.gt) and table.contains(table.keys(data), v.gt) then
              self.data[v.gt] = data[v.gt]
            else
              if not table.contains(table.keys(self.data), v.gt) then self.data[v.gt] = nil end
            end
          end
          local against_data = self.data and self.data[v.gt] or data[v.gt]

          if data[field] <= against_data then
            self.errors = table.append(self.errors, {{ field = field, message = default_error }})
          end
        end

        if v.gte then
          if self.data then
            if not table.contains(table.keys(self.data), v.gte) and table.contains(table.keys(data), v.gte) then
              self.data[v.gte] = data[v.gte]
            else
              if not table.contains(table.keys(self.data), v.gte) then self.data[v.gte] = nil end
            end
          end
          local against_data = self.data and self.data[v.gte] or data[v.gte]

          if data[field] < against_data then
            self.errors = table.append(self.errors, {{ field = field, message = default_error }})
          end
        end

        if v.lt then
          if self.data then
            if not table.contains(table.keys(self.data), v.lt) and table.contains(table.keys(data), v.lt) then
              self.data[v.lt] = data[v.lt]
            else
              if not table.contains(table.keys(self.data), v.lt) then self.data[v.lt] = nil end
            end
          end
          local against_data = self.data and self.data[v.lt] or data[v.lt]

          if data[field] >= against_data then
            self.errors = table.append(self.errors, {{ field = field, message = default_error }})
          end
        end

        if v.lte then
          if self.data then
            if not table.contains(table.keys(self.data), v.lte) and table.contains(table.keys(data), v.lte) then
              self.data[v.lte] = data[v.lte]
            else
              if not table.contains(table.keys(self.data), v.lte) then self.data[v.lte] = nil end
            end
          end
          local against_data = self.data and self.data[v.lte] or data[v.lte]

          if data[field] > against_data then
            self.errors = table.append(self.errors, {{ field = field, message = default_error }})
          end
        end

        if v.other_than then
          if self.data then
            if not table.contains(table.keys(self.data), v.other_than) and table.contains(table.keys(data), v.other_than) then
              self.data[v.other_than] = data[v.other_than]
            else
              if not table.contains(table.keys(self.data), v.other_than) then self.data[v.other_than] = nil end
            end
          end
          local against_data = self.data and self.data[v.other_than] or data[v.other_than]

          if data[field] == against_data then
            self.errors = table.append(self.errors, {{ field = field, message = default_error }})
          end
        end
      end

      if k == "acceptance" then
        local default_error = I18n:t("models.errors.acceptance")
        if(type(v) ~= "table") then v = { } end
        if v.message == nil then v.message = default_error end
        if value ~= true then
          self.errors = table.append(self.errors, {{ field = field, message = default_error }})
        end
      end

      if k == "inclusion" then
        local default_error = I18n:t("models.errors.inclusion")
        if v.message == nil then v.message = default_error end
        if table.contains(v.values, value) ~= true then
          self.errors = table.append(self.errors, {{ field = field, message = default_error }})
        end
      end

      if k == "exclusion" then
        local default_error = I18n:t("models.errors.exclusion")
        if v.message == nil then v.message = default_error end
        if table.contains(v.values, value) ~= false then
          self.errors = table.append(self.errors, {{ field = field, message = default_error }})
        end
      end
    end
  end
end

-- collection

function SoliDBModel:create(data)
  assert(self.data._id == nil, "create not allowed here")
  local callbacks = table.append(self.global_callbacks.before_create, self.callbacks.before_create)
  for _, methodName in pairs(callbacks) do data = self[methodName](data) end

  self:validates_each(data)

  if #self.errors == 0 then
    local result = Sdb:CreateDocument(self.COLLECTION, data)
    if result and result.new then
      self.data = result.new
    else
      self.data = result
    end
    callbacks = table.append(self.global_callbacks.after_create, self.callbacks.after_create)
    for _, methodName in pairs(callbacks) do self[methodName](self)  end
  else
    self.data = data
  end
  return self
end

function SoliDBModel:update(data)
  assert(self.data, "udpate not allowed here")
  local callbacks = table.append(self.global_callbacks.before_update, self.callbacks.before_update)
  for _, methodName in pairs(callbacks) do data = self[methodName](data) end

  -- Merge data for validation
  local merged = {}
  if self.data then
    for k, v in pairs(self.data) do merged[k] = v end
  end
  for k, v in pairs(data) do merged[k] = v end

  self:validates_each(merged)

  if #self.errors == 0 then
    local result = Sdb:UpdateDocument(self.COLLECTION .. "/" .. self.data["_key"], data)
    if result and result.new then
      self.data = result.new
    else
      self.data = result
    end
    callbacks = table.append(self.global_callbacks.after_update, self.callbacks.after_update)
    for _, methodName in pairs(callbacks) do self[methodName](self)  end
    return true
  end
  return false
end

-- Save (Create or Update based on ID)
function SoliDBModel:save(data)
  local payload = data or self.data
  if self.data and self.data._id then
    return self:update(payload)
  else
    self:create(payload)
    return #self.errors == 0
  end
end

function SoliDBModel:delete()
  assert(self.data, "delete not allowed here")
  local result = Sdb:DeleteDocument(self.COLLECTION .. "/" .. self.data["_key"])
  self.data = nil
  return result
end

-- Alias for delete
function SoliDBModel:destroy()
  return self:delete()
end

return SoliDBModel
