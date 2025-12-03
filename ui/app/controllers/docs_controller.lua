local app = {
  index = function()
    Page("docs/index", "app")
  end,

  show = function()
    local page = Params.page or "index"
    -- Basic security check to prevent directory traversal
    if page:match("%.%.") then
      return { status = 403, json = { error = "Forbidden" } }
    end
    
    -- Check if the view exists (this is a bit tricky in luaonbeans without a direct file check helper exposed easily, 
    -- but Page() will error if not found. For now we assume valid links.)
    -- A better approach might be to have a whitelist of pages or check file existence if possible.
    
    Page("docs/" .. page, "app")
  end
}

return BeansEnv == "development" and HandleController(app) or app
