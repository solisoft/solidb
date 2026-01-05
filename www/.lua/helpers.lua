-- Helper functions for luaonbeans MVC framework
-- Common utilities for views and controllers

local Helpers = {}

-- Cache for file hashes (to avoid recalculating on every request)
local hash_cache = {}

-- Read file contents for hashing
local function read_file_content(path)
  -- Try to load from zip first (redbean stores files in zip)
  if type(LoadAsset) == "function" then
    local content = LoadAsset("/" .. path)
    if content then
      return content
    end
  end

  -- Fallback to filesystem (for development mode with -D flag)
  local file = io.open(path, "rb")
  if not file then
    return nil
  end
  local content = file:read("*all")
  file:close()
  return content
end

-- Generate MD5 hash of a string (using redbean's built-in function or fallback)
local function md5_hash(content)
  -- Try redbean's built-in Md5 function
  if type(Md5) == "function" then
    return EncodeHex(Md5(content))
  end
  -- Fallback: use simple hash based on content length and sample bytes
  local hash = #content
  for i = 1, math.min(100, #content) do
    hash = (hash * 31 + content:byte(i)) % 0xFFFFFFFF
  end
  return string.format("%08x", hash)
end

-- Generate asset path with cache-busting hash
-- Usage: public_path("css/style.css") -> "/css/style.css?v=abc123"
function Helpers.public_path(filename)
  if not filename then
    return ""
  end

  -- Ensure filename starts with /
  local url_path = filename
  if url_path:sub(1, 1) ~= "/" then
    url_path = "/" .. url_path
  end

  -- Check cache first
  if hash_cache[filename] then
    return url_path .. "?v=" .. hash_cache[filename]
  end

  -- Build file path (public/ folder)
  local file_path = "public" .. url_path

  -- Read file and generate hash
  local content = read_file_content(file_path)
  if content then
    local hash = md5_hash(content):sub(1, 8)  -- Use first 8 chars
    hash_cache[filename] = hash
    return url_path .. "?v=" .. hash
  end

  -- File not found, return path without hash
  return url_path
end

-- Clear the hash cache (useful for development/reload)
function Helpers.clear_public_path_cache()
  hash_cache = {}
end

-- Generate a URL path (simple version, can be extended for named routes)
function Helpers.url_for(path, params)
  if not params then
    return path
  end

  -- Replace :param placeholders with actual values
  local url = path:gsub(":([%w_]+)", function(param)
    local value = params[param]
    if value then
      return EscapePath(tostring(value))
    end
    return ":" .. param
  end)

  return url
end

-- Generate an anchor tag
function Helpers.link_to(text, url, options)
  options = options or {}
  local attrs = {}

  for k, v in pairs(options) do
    table.insert(attrs, k .. '="' .. EscapeHtml(tostring(v)) .. '"')
  end

  local attr_str = #attrs > 0 and " " .. table.concat(attrs, " ") or ""
  return '<a href="' .. EscapeHtml(url) .. '"' .. attr_str .. '>' .. EscapeHtml(text) .. '</a>'
end

-- Generate a form tag
function Helpers.form_tag(action, options, content)
  options = options or {}
  local method = options.method or "POST"
  local attrs = { 'action="' .. EscapeHtml(action) .. '"' }

  -- HTML forms only support GET and POST
  local actual_method = method
  if method ~= "GET" and method ~= "POST" then
    actual_method = "POST"
    -- Add hidden field for method override
    content = '<input type="hidden" name="_method" value="' .. method .. '">' .. (content or "")
  end

  table.insert(attrs, 'method="' .. actual_method .. '"')

  for k, v in pairs(options) do
    if k ~= "method" then
      table.insert(attrs, k .. '="' .. EscapeHtml(tostring(v)) .. '"')
    end
  end

  local attr_str = table.concat(attrs, " ")
  return '<form ' .. attr_str .. '>' .. (content or "")
end

-- Close form tag
function Helpers.end_form()
  return '</form>'
end

-- Generate an input field
function Helpers.input_tag(name, value, options)
  options = options or {}
  local input_type = options.type or "text"
  local attrs = {
    'type="' .. EscapeHtml(input_type) .. '"',
    'name="' .. EscapeHtml(name) .. '"'
  }

  if value then
    table.insert(attrs, 'value="' .. EscapeHtml(tostring(value)) .. '"')
  end

  for k, v in pairs(options) do
    if k ~= "type" then
      table.insert(attrs, k .. '="' .. EscapeHtml(tostring(v)) .. '"')
    end
  end

  return '<input ' .. table.concat(attrs, " ") .. '>'
end

-- Generate a textarea
function Helpers.textarea_tag(name, content, options)
  options = options or {}
  local attrs = { 'name="' .. EscapeHtml(name) .. '"' }

  for k, v in pairs(options) do
    table.insert(attrs, k .. '="' .. EscapeHtml(tostring(v)) .. '"')
  end

  return '<textarea ' .. table.concat(attrs, " ") .. '>' .. EscapeHtml(content or "") .. '</textarea>'
end

-- Generate a select dropdown
function Helpers.select_tag(name, options_list, selected, attrs)
  attrs = attrs or {}
  local attr_list = { 'name="' .. EscapeHtml(name) .. '"' }

  for k, v in pairs(attrs) do
    table.insert(attr_list, k .. '="' .. EscapeHtml(tostring(v)) .. '"')
  end

  local html = '<select ' .. table.concat(attr_list, " ") .. '>'

  for _, opt in ipairs(options_list) do
    local value, label
    if type(opt) == "table" then
      value, label = opt[1], opt[2]
    else
      value, label = opt, opt
    end

    local selected_attr = ""
    if tostring(value) == tostring(selected) then
      selected_attr = ' selected'
    end

    html = html .. '<option value="' .. EscapeHtml(tostring(value)) .. '"' .. selected_attr .. '>' ..
           EscapeHtml(tostring(label)) .. '</option>'
  end

  html = html .. '</select>'
  return html
end

-- Format a date
function Helpers.format_date(timestamp, format)
  format = format or "%Y-%m-%d %H:%M:%S"
  if type(timestamp) == "number" then
    return os.date(format, timestamp)
  end
  return timestamp or ""
end

-- Parse ISO 8601 date string and return Unix timestamp
-- Handles formats like: "2025-12-31T15:49:33.421166+00:00" or "2025-12-31T15:49:33Z"
function Helpers.parse_iso8601(iso_string)
  if not iso_string or iso_string == "" then
    return nil
  end

  -- Extract date and time components
  local year, month, day, hour, min, sec = iso_string:match(
    "(%d+)-(%d+)-(%d+)T(%d+):(%d+):(%d+)"
  )

  if not year then
    return nil
  end

  return os.time({
    year = tonumber(year),
    month = tonumber(month),
    day = tonumber(day),
    hour = tonumber(hour),
    min = tonumber(min),
    sec = tonumber(sec)
  })
end

-- Format an ISO 8601 date string using I18n
-- Usage: format_datetime("2025-12-31T15:49:33+00:00") -> "Dec 31, 2025 15:49"
-- Usage: format_datetime("2025-12-31T15:49:33+00:00", "short") -> "Dec 31, 15:49"
function Helpers.format_datetime(iso_string, style)
  if not iso_string or iso_string == "" then
    return "-"
  end

  local timestamp = Helpers.parse_iso8601(iso_string)
  if not timestamp then
    return iso_string
  end

  -- Get format from I18n based on style
  local I18n = require("i18n")
  local format

  if style == "short" then
    format = I18n:t("datetime.formats.short")
    if format == "datetime.formats.short" then
      format = "%b %d, %H:%M"
    end
  elseif style == "long" then
    format = I18n:t("datetime.formats.long")
    if format == "datetime.formats.long" then
      format = "%B %d, %Y at %H:%M"
    end
  elseif style == "date" then
    format = I18n:t("date.formats.long")
    if format == "date.formats.long" then
      format = "%B %d, %Y"
    end
  else
    format = I18n:t("datetime.formats.default")
    if format == "datetime.formats.default" then
      format = "%Y-%m-%d %H:%M"
    end
  end

  return os.date(format, timestamp)
end

-- Format relative time (e.g., "5 minutes ago", "yesterday")
function Helpers.time_ago(iso_string)
  if not iso_string or iso_string == "" then
    return "-"
  end

  local timestamp = Helpers.parse_iso8601(iso_string)
  if not timestamp then
    return iso_string
  end

  local I18n = require("i18n")
  local now = os.time()
  local diff = now - timestamp

  if diff < 60 then
    return I18n:t("relative_time.now") or "just now"
  elseif diff < 120 then
    return I18n:t("relative_time.minute") or "1 minute ago"
  elseif diff < 3600 then
    local mins = math.floor(diff / 60)
    return string.format(I18n:t("relative_time.minutes") or "%d minutes ago", mins)
  elseif diff < 7200 then
    return I18n:t("relative_time.hour") or "1 hour ago"
  elseif diff < 86400 then
    local hours = math.floor(diff / 3600)
    return string.format(I18n:t("relative_time.hours") or "%d hours ago", hours)
  elseif diff < 172800 then
    return I18n:t("relative_time.day") or "yesterday"
  elseif diff < 604800 then
    local days = math.floor(diff / 86400)
    return string.format(I18n:t("relative_time.days") or "%d days ago", days)
  elseif diff < 1209600 then
    return I18n:t("relative_time.week") or "1 week ago"
  elseif diff < 2592000 then
    local weeks = math.floor(diff / 604800)
    return string.format(I18n:t("relative_time.weeks") or "%d weeks ago", weeks)
  elseif diff < 5184000 then
    return I18n:t("relative_time.month") or "1 month ago"
  elseif diff < 31536000 then
    local months = math.floor(diff / 2592000)
    return string.format(I18n:t("relative_time.months") or "%d months ago", months)
  elseif diff < 63072000 then
    return I18n:t("relative_time.year") or "1 year ago"
  else
    local years = math.floor(diff / 31536000)
    return string.format(I18n:t("relative_time.years") or "%d years ago", years)
  end
end

-- Truncate text
function Helpers.truncate(text, length, suffix)
  if not text then return "" end
  length = length or 100
  suffix = suffix or "..."

  if #text <= length then
    return text
  end

  return text:sub(1, length) .. suffix
end

-- Pluralize a word based on count
function Helpers.pluralize(count, singular, plural)
  plural = plural or (singular .. "s")
  if count == 1 then
    return tostring(count) .. " " .. singular
  else
    return tostring(count) .. " " .. plural
  end
end

-- Generate a simple pagination
function Helpers.paginate(current_page, total_pages, base_url)
  if total_pages <= 1 then
    return ""
  end

  local html = '<nav class="pagination">'

  -- Previous link
  if current_page > 1 then
    html = html .. '<a href="' .. base_url .. '?page=' .. (current_page - 1) .. '">&laquo; Prev</a>'
  else
    html = html .. '<span class="disabled">&laquo; Prev</span>'
  end

  -- Page numbers
  for i = 1, total_pages do
    if i == current_page then
      html = html .. '<span class="current">' .. i .. '</span>'
    else
      html = html .. '<a href="' .. base_url .. '?page=' .. i .. '">' .. i .. '</a>'
    end
  end

  -- Next link
  if current_page < total_pages then
    html = html .. '<a href="' .. base_url .. '?page=' .. (current_page + 1) .. '">Next &raquo;</a>'
  else
    html = html .. '<span class="disabled">Next &raquo;</span>'
  end

  html = html .. '</nav>'
  return html
end

-- ============================================================================
-- HTMX Helpers
-- ============================================================================

-- Generate an HTMX link that fetches content and swaps it
-- Usage: hx_link("Load More", "/posts/more", { target = "#posts-list", swap = "beforeend" })
function Helpers.hx_link(text, url, options)
  options = options or {}
  local attrs = {
    'href="' .. EscapeHtml(url) .. '"',
    'hx-get="' .. EscapeHtml(url) .. '"'
  }
  
  if options.target then
    table.insert(attrs, 'hx-target="' .. EscapeHtml(options.target) .. '"')
  end
  
  if options.swap then
    table.insert(attrs, 'hx-swap="' .. EscapeHtml(options.swap) .. '"')
  else
    table.insert(attrs, 'hx-swap="innerHTML"')
  end
  
  if options.trigger then
    table.insert(attrs, 'hx-trigger="' .. EscapeHtml(options.trigger) .. '"')
  end
  
  if options.confirm then
    table.insert(attrs, 'hx-confirm="' .. EscapeHtml(options.confirm) .. '"')
  end
  
  if options.indicator then
    table.insert(attrs, 'hx-indicator="' .. EscapeHtml(options.indicator) .. '"')
  end
  
  -- Pass through other attributes like class, id, etc.
  for k, v in pairs(options) do
    if k ~= "target" and k ~= "swap" and k ~= "trigger" and k ~= "confirm" and k ~= "indicator" then
      table.insert(attrs, k .. '="' .. EscapeHtml(tostring(v)) .. '"')
    end
  end
  
  return '<a ' .. table.concat(attrs, " ") .. '>' .. EscapeHtml(text) .. '</a>'
end

-- Generate an HTMX form that submits via AJAX
-- Usage: hx_form("/posts", { method = "POST", target = "#result" })
function Helpers.hx_form(action, options)
  options = options or {}
  local method = options.method or "POST"
  local hx_method = method:lower()
  
  local attrs = {
    'action="' .. EscapeHtml(action) .. '"',
    'hx-' .. hx_method .. '="' .. EscapeHtml(action) .. '"'
  }
  
  -- HTML forms only support GET and POST
  local actual_method = method
  if method ~= "GET" and method ~= "POST" then
    actual_method = "POST"
  end
  table.insert(attrs, 'method="' .. actual_method .. '"')
  
  if options.target then
    table.insert(attrs, 'hx-target="' .. EscapeHtml(options.target) .. '"')
  end
  
  if options.swap then
    table.insert(attrs, 'hx-swap="' .. EscapeHtml(options.swap) .. '"')
  end
  
  if options.confirm then
    table.insert(attrs, 'hx-confirm="' .. EscapeHtml(options.confirm) .. '"')
  end
  
  if options.indicator then
    table.insert(attrs, 'hx-indicator="' .. EscapeHtml(options.indicator) .. '"')
  end
  
  -- Pass through other attributes
  for k, v in pairs(options) do
    if k ~= "method" and k ~= "target" and k ~= "swap" and k ~= "confirm" and k ~= "indicator" then
      table.insert(attrs, k .. '="' .. EscapeHtml(tostring(v)) .. '"')
    end
  end
  
  local html = '<form ' .. table.concat(attrs, " ") .. '>'
  
  -- Add method override for DELETE, PUT, PATCH
  if method ~= "GET" and method ~= "POST" then
    html = html .. '<input type="hidden" name="_method" value="' .. method .. '">'
  end
  
  return html
end

-- HTMX delete button with confirmation
-- Usage: hx_delete_button("Delete", "/posts/1", { target = "closest tr", swap = "outerHTML swap:1s" })
function Helpers.hx_delete_button(text, url, options)
  options = options or {}
  local attrs = {
    'type="button"',
    'hx-delete="' .. EscapeHtml(url) .. '"'
  }
  
  if options.target then
    table.insert(attrs, 'hx-target="' .. EscapeHtml(options.target) .. '"')
  end
  
  if options.swap then
    table.insert(attrs, 'hx-swap="' .. EscapeHtml(options.swap) .. '"')
  else
    table.insert(attrs, 'hx-swap="outerHTML"')
  end
  
  if options.confirm then
    table.insert(attrs, 'hx-confirm="' .. EscapeHtml(options.confirm) .. '"')
  else
    table.insert(attrs, 'hx-confirm="Are you sure?"')
  end
  
  -- Pass through other attributes
  for k, v in pairs(options) do
    if k ~= "target" and k ~= "swap" and k ~= "confirm" then
      table.insert(attrs, k .. '="' .. EscapeHtml(tostring(v)) .. '"')
    end
  end
  
  return '<button ' .. table.concat(attrs, " ") .. '>' .. EscapeHtml(text) .. '</button>'
end

return Helpers
