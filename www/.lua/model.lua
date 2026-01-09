-- Model base class for Luaonbeans MVC Framework
-- Wraps SoliDBModel for easy model creation

local SoliDBModel = require("solidb_model")

local Model = {}
Model.__index = Model

-- Create a new model class
function Model.create(collection_name, options)
  options = options or {}

  local ModelClass = {}
  ModelClass.__index = ModelClass
  setmetatable(ModelClass, { __index = Model })

  -- Store collection name
  ModelClass.COLLECTION = collection_name

  -- Store validations
  ModelClass._validations = options.validations or {}

  -- Store permitted fields for mass assignment protection
  ModelClass._permitted_fields = options.permitted_fields or nil

  -- Store callbacks
  ModelClass._callbacks = {
    before_create = options.before_create or {},
    after_create = options.after_create or {},
    before_update = options.before_update or {},
    after_update = options.after_update or {}
  }

  -- Filter data to only include permitted fields (mass assignment protection)
  function ModelClass.permit(s, d)
    local self, data = s, d
    if type(s) ~= "table" or not s._permitted_fields then
      self, data = ModelClass, s
    end

    if not self._permitted_fields or not data then
      return data
    end

    local filtered = {}
    for _, field in ipairs(self._permitted_fields) do
      if data[field] ~= nil then
        filtered[field] = data[field]
      end
    end
    return filtered
  end

  -- Helper to handle dot vs colon calls
  local function resolve(s, d)
    if s == ModelClass or (type(s) == "table" and s.data) then
      return s, d
    end
    return ModelClass, s
  end

  -- Create a new model instance
  function ModelClass.new(s, d)
    local self, data = resolve(s, d)
    local instance = SoliDBModel.new(data)
    instance.COLLECTION = self.COLLECTION
    instance.validations = self._validations
    instance.callbacks = self._callbacks
    setmetatable(instance, { __index = function(t, k)
      -- First check ModelClass for custom methods (favors subclass)
      if ModelClass[k] then return ModelClass[k] end
      -- Then check SoliDBModel for framework methods
      if SoliDBModel[k] then return SoliDBModel[k] end

      -- Check data attributes
      if t.data then
        if t.data[k] ~= nil then return t.data[k] end
        -- Alias id -> _key (common convention for URLs in SDB)
        if k == "id" then return t.data._key end
        if k == "key" then return t.data._key end
        if k == "uid" then return t.data._id end
      end

      return nil
    end })
    return instance
  end

  -- Query shortcuts that return new instances
  function ModelClass.find(s, id)
    local self, handler = resolve(s, id)
    if self.data then return SoliDBModel.find(self, handler) end
    return self:new():find(handler)
  end

  function ModelClass.find_by(criteria)
    local self, c = resolve(ModelClass, criteria)
    if self.data then return SoliDBModel.find_by(self, c) end
    local instance = self:new()
    return SoliDBModel.find_by(instance, c)
  end

  function ModelClass.where(s, criteria)
    local self, c = resolve(s, criteria)
    if self.data then return SoliDBModel.where(self, c) end
    local instance = self:new()
    return SoliDBModel.where(instance, c)
  end

  function ModelClass.all(s, options)
    local self, opts = resolve(s, options)
    if self.data then return SoliDBModel.all(self, opts) end
    return SoliDBModel.all(self, opts)
  end

  function ModelClass.first(s)
    local self = s
    if not self or self == ModelClass then
      self = ModelClass:new()
    end
    return SoliDBModel.first(self)
  end

  function ModelClass.last(s)
    local self = s
    if not self or self == ModelClass then
      self = ModelClass:new()
    end
    return SoliDBModel.last(self)
  end

  function ModelClass.any(s)
    local self = s
    if not self or self == ModelClass then
      self = ModelClass:new()
    end
    return SoliDBModel.any(self)
  end

  function ModelClass.create(s, d)
    local self, data = resolve(s, d)
    if self.data then return SoliDBModel.create(self, data) end
    local instance = self:new(data)
    instance:save()
    return instance
  end

  function ModelClass.count(s)
    local self = resolve(s)
    local collection = self.COLLECTION
    local result = Sdb:Sdbql(
      "RETURN COLLECTION_COUNT(`" .. collection .. "`)"
    )
    if result.result and result.result[1] then
      return result.result[1]
    end
    return 0
  end

  return ModelClass
end

return Model
