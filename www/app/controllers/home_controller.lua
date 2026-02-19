local Controller = require("controller")
local HomeController = Controller:extend()

local function get_version()
  local handle = io.popen("grep -m1 '^version' ../Cargo.toml 2>/dev/null | sed 's/.*version.*=.*\"\\(.*\\)\".*/\\1/'")
  if handle then
    local result = handle:read("*a")
    handle:close()
    if result and result ~= "" then
      return result:gsub("%s+", "")
    end
  end
  return "0.11.0"
end

function HomeController:index()
  -- Using "app" layout by default or custom one if needed
  -- The original landing controller in www used:
  -- Params.no_padding = true
  -- Params.hide_header = true
  -- Page("landing/index", "app")
  
  self.layout = "application" -- using www2 default layout name if it exists, likely "application" or "default"
  -- Based on investigation, www2 has layouts. Let's assume 'application' or check later.
  
  -- But wait, www2 controllers use :render().
  -- Porting logic to www2 style:
  
  self:render("home/index", {
    no_padding = true,
    hide_header = true,
    version = get_version()
  })
end

function HomeController:up()
  self:text("UP")
end

function HomeController:about()
    self:render("home/about")
end

return HomeController
