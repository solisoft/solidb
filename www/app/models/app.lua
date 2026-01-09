local Model = require("model")

local App = Model.create("apps", {
  permitted_fields = { "name", "description", "color", "status", "position" },
  validations = {
    name = { presence = true, length = { between = {1, 100} } }
  }
})

-- Get features for this app
function App:features()
  local Feature = require("models.feature")
  return Feature:new():where({ app_id = self._key }):order("doc.position ASC"):all()
end

-- Count features
function App:features_count()
  local result = Sdb:Sdbql(
    "FOR f IN features FILTER f.app_id == @app_id COLLECT WITH COUNT INTO c RETURN c",
    { app_id = self._key }
  )
  if result and result.result and result.result[1] then
    return result.result[1]
  end
  return 0
end

return App
