-- Docs Controller for SoliDB Documentation
local Controller = require("controller")
local DocsController = Controller:extend()

-- Page title mappings
local PAGE_TITLES = {
  ["getting-started"] = "Getting Started",
  ["sdbql"] = "SDBQL Reference",
  ["sql"] = "SQL Compatibility",
  ["api"] = "API Reference",
  ["driver"] = "Native Driver",
  ["architecture"] = "Architecture",
  ["cluster"] = "Clustering",
  ["sharding"] = "Sharding",
  ["replication"] = "Replication",
  ["transactions"] = "ACID Transactions",
  ["indexes"] = "Indexes",
  ["graphs"] = "Graphs & Edges",
  ["blobs"] = "Blob Storage",
  ["queues"] = "Queues & Jobs",
  ["scripting"] = "Lua Scripting",
  ["live-queries"] = "Live Queries",
  ["changefeeds"] = "Changefeeds",
  ["documents"] = "Documents Storage",
  ["timeseries"] = "Time Series",
  ["columnar"] = "Columnar Storage",
  ["clients"] = "Official Clients",
  ["comparison"] = "Database Comparison",
  ["security"] = "Security",
  ["tooling"] = "Command-Line Tools",
  ["ai_features"] = "AI Engine",
  ["ai_agents"] = "Build AI Agents"
}

-- Documentation landing page
function DocsController:index()
  self.layout = "docs"
  self:render("docs/index", {
    title = "SoliDB Documentation",
    current_page = "index"
  })
end

-- Individual documentation pages
function DocsController:show()
  local page = self.params.page or "index"

  -- Security: prevent path traversal
  if page:match("%.%.") or page:match("/") then
    return self:render("errors/404", {}, { layout = false, status = 404 })
  end

  -- Build page title
  local page_title = PAGE_TITLES[page]
  if not page_title then
    -- Convert hyphens/underscores to title case
    page_title = page:gsub("[-_]", " "):gsub("(%a)([%w]*)", function(first, rest)
      return first:upper() .. rest
    end)
  end

  self.layout = "docs"
  self:render("docs/" .. page, {
    title = page_title .. " - SoliDB Documentation",
    current_page = page
  })
end

-- Slides/Presentation mode
function DocsController:slides()
  self.layout = "slides"
  self:render("docs/slides", {
    title = "SoliDB Presentation",
    current_page = "slides"
  })
end

return DocsController
