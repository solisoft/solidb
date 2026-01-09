local JsonSchemaValidator = {}

function JsonSchemaValidator.validate(schema, instance)
  local errors = {}

  if not schema then return true, errors end

  -- Validate type
  if schema.type then
    local valid_type = false
    local instance_type = type(instance)
    
    -- Lua type mapping
    if schema.type == "object" and instance_type == "table" and not (instance[1]) then
      valid_type = true
    elseif schema.type == "array" and instance_type == "table" and (instance[1] or next(instance) == nil) then
      valid_type = true
    elseif schema.type == "string" and instance_type == "string" then
      valid_type = true
    elseif schema.type == "number" and instance_type == "number" then
      valid_type = true
    elseif schema.type == "integer" and instance_type == "number" and math.floor(instance) == instance then
      valid_type = true
    elseif schema.type == "boolean" and instance_type == "boolean" then
      valid_type = true
    elseif schema.type == "null" and instance == nil then
      valid_type = true
    end

    if not valid_type then
      table.insert(errors, "Expected type " .. schema.type .. " but got " .. instance_type)
      return false, errors
    end
  end

  -- Validate properties (for objects)
  if schema.type == "object" and schema.properties then
    if type(instance) == "table" then
      for key, prop_schema in pairs(schema.properties) do
        local value = instance[key]
        -- Recursive validation
        if value ~= nil then 
          local ok, field_errors = JsonSchemaValidator.validate(prop_schema, value)
          if not ok then
            for _, err in ipairs(field_errors) do
              table.insert(errors, key .. ": " .. err)
            end
          end
        end
      end
    end
  end

  -- Validate required (for objects)
  if schema.required then
    for _, req_field in ipairs(schema.required) do
      if instance[req_field] == nil or instance[req_field] == "" then
        table.insert(errors, "Missing required field: " .. req_field)
      end
    end
  end

  -- Validate string constraints
  if schema.type == "string" then
    if schema.minLength and #instance < schema.minLength then
      table.insert(errors, "String length must be at least " .. schema.minLength)
    end
    if schema.maxLength and #instance > schema.maxLength then
      table.insert(errors, "String length must be at most " .. schema.maxLength)
    end
    if schema.pattern then
      if not string.match(instance, schema.pattern) then
        table.insert(errors, "String does not match pattern: " .. schema.pattern)
      end
    end
    if schema.format then
       if schema.format == "email" then
         if not string.match(instance, "[%w%.%_%-%+]+@[%w%.%_%-]+%.%w+") then
           table.insert(errors, "Invalid email format")
         end
       elseif schema.format == "date" then
          -- YYYY-MM-DD simple check
          if not string.match(instance, "^%d%d%d%d%-%d%d%-%d%d$") then
             table.insert(errors, "Invalid date format (expected YYYY-MM-DD)")
          end
       end
    end
    if schema.enum then
       local found = false
       for _, v in ipairs(schema.enum) do
          if v == instance then found = true break end
       end
       if not found then
          table.insert(errors, "Value not in allowed options")
       end
    end
  end

  return #errors == 0, errors
end

return JsonSchemaValidator
