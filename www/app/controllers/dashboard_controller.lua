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

  indexes = function()
    Page("dashboard/indexes", "app")
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

  scripts = function()
    Page("dashboard/scripts", "app")
  end,

  sharding = function()
    Page("dashboard/sharding", "app")
  end,

  apikeys = function()
    Page("dashboard/apikeys", "app")
  end
}

return BeansEnv == "development" and HandleController(app) or app
