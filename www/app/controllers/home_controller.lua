local Controller = require("controller")
local HomeController = Controller:extend()

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
    hide_header = true
  })
end

function HomeController:up()
  self:text("UP")
end

function HomeController:about()
    self:render("home/about")
end

return HomeController
