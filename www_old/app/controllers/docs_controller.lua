local app = {
  index = function()
    Page("docs/index", "app")
  end,

  show = function()
    local page = Params.page or "index"
    -- Basic security check
    if page:match("%.%.") then
      return { status = 403, json = { error = "Forbidden" } }
    end

    Page("docs/" .. page, "app")
  end,

  slides = function()
    Params.hide_header = true
    Params.full_height = true
    Params.no_padding = true
    Params['db'] = nil
    Page("docs/slides", "app")
  end
}

return BeansEnv == "development" and HandleController(app) or app
