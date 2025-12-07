local app = {
  index = function()
    Page("dashboard/index", "app")
  end,

  collections = function()
    -- In a real app, we would fetch collections here
    -- local collections = DB:query("FOR c IN collections RETURN c")
    Page("dashboard/collections", "app", { collections = {} })
  end,

  query = function()
    Page("dashboard/query", "app")
  end,

  databases = function()
    Page("dashboard/databases", "app")
  end,

  documents = function(self)
    Page("dashboard/documents", "app")
  end,

  live = function(self)
    Page("dashboard/live", "app")
  end,

  cluster = function()
    Page("dashboard/cluster", "app")
  end,

  apikeys = function()
    Page("dashboard/apikeys", "app")
  end
}

return BeansEnv == "development" and HandleController(app) or app
