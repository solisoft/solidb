local app = {
  index = function()
    Params.no_padding = true
    Params.hide_header = true
    Page("landing/index", "app")
  end
}

return BeansEnv == "development" and HandleController(app) or app
