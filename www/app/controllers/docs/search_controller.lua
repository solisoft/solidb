local Controller = require("controller")
local SearchController = Controller:extend()

function SearchController:search()
  local query = self.params.q or ""
  query = query:lower():gsub("^%s+", ""):gsub("%s+$", "") -- trim and lowercase

  if #query < 1 then
    self:render_partial("docs/_search_results", { results = {}, query = query })
    return
  end

  -- Load search data (try multiple paths)
  local paths = {
    "public/data/sdbql-methods.json",
    "www/public/data/sdbql-methods.json",
    "/zip/public/data/sdbql-methods.json"
  }
  local file, content
  for _, path in ipairs(paths) do
    file = io.open(path, "r")
    if file then
      content = file:read("*a")
      file:close()
      break
    end
  end
  if not content then
    return self:render_partial("docs/_search_results", { results = {}, query = query, error = "Could not load search data" })
  end

  local data = DecodeJson(content)
  local results = {}

  -- Search functions
  for _, fn in ipairs(data.functions or {}) do
    local name_lower = fn.name:lower()
    local desc_lower = fn.description:lower()
    local score = 0

    if name_lower == query then
      score = 1000
    elseif name_lower:find("^" .. query:gsub("([%(%)%.%%%+%-%*%?%[%]%^%$])", "%%%1")) then
      score = 800
    elseif name_lower:find(query, 1, true) then
      score = 600
    elseif desc_lower:find(query, 1, true) then
      score = 200
    end

    if score > 0 then
      table.insert(results, {
        type = "function",
        name = fn.name,
        description = fn.description,
        category = fn.category,
        url = fn.url,
        score = score
      })
    end
  end

  -- Search operators
  for _, op in ipairs(data.operators or {}) do
    local name_lower = op.name:lower()
    local desc_lower = op.description:lower()
    local score = 0

    if name_lower == query then
      score = 1000
    elseif name_lower:find("^" .. query:gsub("([%(%)%.%%%+%-%*%?%[%]%^%$])", "%%%1")) then
      score = 800
    elseif name_lower:find(query, 1, true) then
      score = 600
    elseif desc_lower:find(query, 1, true) then
      score = 200
    end

    if score > 0 then
      table.insert(results, {
        type = "operator",
        name = op.name,
        description = op.description,
        category = op.category,
        url = op.url,
        score = score
      })
    end
  end

  -- Search keywords
  for _, kw in ipairs(data.keywords or {}) do
    local name_lower = kw.name:lower()
    local desc_lower = kw.description:lower()
    local score = 0

    if name_lower == query then
      score = 1000
    elseif name_lower:find("^" .. query:gsub("([%(%)%.%%%+%-%*%?%[%]%^%$])", "%%%1")) then
      score = 800
    elseif name_lower:find(query, 1, true) then
      score = 600
    elseif desc_lower:find(query, 1, true) then
      score = 200
    end

    if score > 0 then
      table.insert(results, {
        type = "keyword",
        name = kw.name,
        description = kw.description,
        category = kw.category,
        url = kw.url,
        score = score
      })
    end
  end

  -- Sort by score descending
  table.sort(results, function(a, b) return a.score > b.score end)

  -- Limit to 10 results
  local limited = {}
  for i = 1, math.min(10, #results) do
    table.insert(limited, results[i])
  end

  return self:render_partial("docs/_search_results", { results = limited, query = query })
end

return SearchController
