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
  ["vector-search"] = "Vector Search",
  ["hybrid-search"] = "Hybrid Search",
  ["graphs"] = "Graphs & Edges",
  ["blobs"] = "Blob Storage",
  ["queues"] = "Queues & Jobs",
  ["streams"] = "Stream Processing",
  ["scripting"] = "Lua Scripting",
  ["scripting-management"] = "Lua: Management API",
  ["scripting-core"] = "Lua: Core API",
  ["scripting-ws"] = "Lua: WebSockets",
  ["scripting-database"] = "Lua: Database Access",
  ["scripting-validation"] = "Lua: Validation",
  ["scripting-files"] = "Lua: File & Media",
  ["scripting-utils"] = "Lua: Utilities",
  ["scripting-streams"] = "Lua: Streams",
  ["scripting-development"] = "Lua: Development Tools",
  ["live-queries"] = "Live Queries",
  ["changefeeds"] = "Changefeeds",
  ["documents"] = "Documents Storage",
  ["timeseries"] = "Time Series",
  ["columnar"] = "Columnar Storage",
  ["clients"] = "Official Clients",
  ["clients-go"] = "Go Client",
  ["clients-python"] = "Python Client",
  ["clients-nodejs"] = "Node.js / Bun Client",
  ["clients-php"] = "PHP Client",
  ["clients-ruby"] = "Ruby Client",
  ["clients-elixir"] = "Elixir Client",
  ["comparison"] = "Database Comparison",
  ["security"] = "Security",
  ["tooling"] = "Command-Line Tools"
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
