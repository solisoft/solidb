-- I18n Module for Luaonbeans
-- Internationalization with Lua table-based translations

local I18n = {}
I18n.__index = I18n

-- Configuration
I18n.default_locale = "en"
I18n.current_locale = "en"
I18n.fallback_locale = "en"
I18n.translations = {}
I18n.loaded_locales = {}

-- Load a locale file
function I18n:load_locale(locale)
  if self.loaded_locales[locale] then
    return true
  end
  
  local ok, translations = pcall(require, "locales." .. locale)
  if ok and type(translations) == "table" then
    self.translations[locale] = translations
    self.loaded_locales[locale] = true
    return true
  end
  
  return false
end

-- Load all locales from config/locales/
function I18n:load_all()
  -- Try to load common locales
  local common_locales = {"en", "fr", "de", "es", "it", "pt", "ja", "zh", "ko", "ru"}
  for _, locale in ipairs(common_locales) do
    self:load_locale(locale)
  end
end

-- Set the current locale
function I18n:set_locale(locale)
  if self:load_locale(locale) then
    self.current_locale = locale
    return true
  end
  return false
end

-- Get the current locale
function I18n:get_locale()
  return self.current_locale
end

-- Get available locales
function I18n:available_locales()
  local locales = {}
  for locale, _ in pairs(self.loaded_locales) do
    table.insert(locales, locale)
  end
  return locales
end

-- Deep get a value from a nested table using dot notation
local function deep_get(tbl, key)
  if not tbl then return nil end
  
  local parts = {}
  for part in string.gmatch(key, "[^%.]+") do
    table.insert(parts, part)
  end
  
  local current = tbl
  for _, part in ipairs(parts) do
    if type(current) ~= "table" then
      return nil
    end
    current = current[part]
  end
  
  return current
end

-- Translate a key with optional interpolation
-- Usage: I18n:t("hello") or I18n:t("welcome", "John") or I18n:t("items", {count = 5})
function I18n:t(key, ...)
  if key == nil then return nil end
  if key == "" then return "" end
  
  local args = {...}
  local locale = self.current_locale
  
  -- Try current locale
  local translation = deep_get(self.translations[locale], key)
  
  -- Fallback to default locale
  if translation == nil and locale ~= self.fallback_locale then
    translation = deep_get(self.translations[self.fallback_locale], key)
  end
  
  -- Return key if not found
  if translation == nil then
    return key
  end
  
  -- Handle functions (for complex pluralization)
  if type(translation) == "function" then
    return translation(table.unpack(args))
  end
  
  -- Handle string interpolation
  if type(translation) == "string" and #args > 0 then
    -- If first arg is a table, use named interpolation
    if type(args[1]) == "table" then
      local vars = args[1]
      translation = translation:gsub("%%{(%w+)}", function(name)
        return tostring(vars[name] or "%{" .. name .. "}")
      end)
    else
      -- Use positional interpolation (sprintf style)
      local ok, result = pcall(string.format, translation, table.unpack(args))
      if ok then
        translation = result
      end
    end
  end
  
  return translation
end

-- Alias for t()
function I18n:translate(key, ...)
  return self:t(key, ...)
end

-- Check if a translation exists
function I18n:exists(key, locale)
  locale = locale or self.current_locale
  return deep_get(self.translations[locale], key) ~= nil
end

-- Pluralization helper
-- Usage: I18n:p(count, "item", "items") or I18n:p(count, {one = "1 item", other = "%d items"})
function I18n:p(count, singular_or_table, plural)
  if type(singular_or_table) == "table" then
    local forms = singular_or_table
    if count == 0 and forms.zero then
      return forms.zero:gsub("%%d", tostring(count))
    elseif count == 1 and forms.one then
      return forms.one:gsub("%%d", tostring(count))
    else
      return (forms.other or forms[2] or ""):gsub("%%d", tostring(count))
    end
  else
    if count == 1 then
      return singular_or_table
    else
      return plural or (singular_or_table .. "s")
    end
  end
end

-- Localize a number
function I18n:number(num, options)
  options = options or {}
  local precision = options.precision or 0
  local delimiter = options.delimiter or ","
  local separator = options.separator or "."
  
  local formatted = string.format("%." .. precision .. "f", num)
  local int_part, dec_part = formatted:match("([^.]+)%.?(.*)")
  
  -- Add thousand separators
  int_part = int_part:reverse():gsub("(%d%d%d)", "%1" .. delimiter):reverse()
  int_part = int_part:gsub("^" .. delimiter, "")
  
  if precision > 0 and dec_part ~= "" then
    return int_part .. separator .. dec_part
  end
  return int_part
end

-- Localize a date
function I18n:date(timestamp, format)
  local t_format = self:t("date.formats.default")
  if not format and t_format and t_format ~= "date.formats.default" then
    format = t_format
  end
  format = format or "%Y-%m-%d"
  return os.date(format, timestamp)
end

-- Localize a time
function I18n:time(timestamp, format)
  local t_format = self:t("time.formats.default")
  if not format and t_format and t_format ~= "time.formats.default" then
    format = t_format
  end
  format = format or "%H:%M:%S"
  return os.date(format, timestamp)
end

-- Localize a datetime
function I18n:datetime(timestamp, format)
  local t_format = self:t("datetime.formats.default")
  if not format and t_format and t_format ~= "datetime.formats.default" then
    format = t_format
  end
  format = format or "%Y-%m-%d %H:%M:%S"
  return os.date(format, timestamp)
end

-- Detect locale from Accept-Language header
function I18n:detect_locale(accept_language)
  if not accept_language then return self.default_locale end
  
  -- Parse Accept-Language header (e.g., "en-US,en;q=0.9,fr;q=0.8")
  local locales = {}
  for part in accept_language:gmatch("[^,]+") do
    local locale, q = part:match("([^;]+);?q?=?([%d%.]*)")
    locale = locale:match("^%s*(.-)%s*$") -- trim
    q = tonumber(q) or 1.0
    table.insert(locales, {locale = locale, q = q})
  end
  
  -- Sort by quality
  table.sort(locales, function(a, b) return a.q > b.q end)
  
  -- Find first available locale
  for _, item in ipairs(locales) do
    local locale = item.locale:match("^(%w+)")
    if self.loaded_locales[locale] then
      return locale
    end
  end
  
  return self.default_locale
end

-- Create a global t() function for convenience
function I18n:make_global()
  _G.t = function(key, ...)
    return I18n:t(key, ...)
  end
  _G.I18n = I18n
end

return I18n
