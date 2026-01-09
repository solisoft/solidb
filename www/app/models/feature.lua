local Model = require("model")

local Feature = Model.create("features", {
  permitted_fields = { "name", "description", "color", "tags", "status", "position", "columns", "app_id" },
  validations = {
    name = { presence = true, length = { between = {1, 100} } },
    app_id = { presence = true }
  }
})

-- Default columns for new features
Feature.DEFAULT_COLUMNS = {
  { id = "todo", name = "To Do", color = "text-dim", position = 1 },
  { id = "in_progress", name = "In Progress", color = "primary", position = 2 },
  { id = "done", name = "Done", color = "success", position = 3 }
}

-- Get parent app (use get_ prefix to avoid conflict with data.app)
function Feature:get_app()
  local App = require("models.app")
  local app_id = self.app_id or (self.data and self.data.app_id)
  if not app_id then return nil end
  return App:find(app_id)
end

-- Get tasks for this feature
function Feature:tasks()
  local Task = require("models.task")
  return Task:new():where({ feature_id = self._key }):order("doc.position ASC"):all()
end

-- Get tasks by status
function Feature:tasks_by_status(status)
  local Task = require("models.task")
  return Task:new():where({ feature_id = self._key, status = status }):order("doc.position ASC"):all()
end

-- Count tasks
function Feature:tasks_count()
  local result = Sdb:Sdbql(
    "FOR t IN tasks FILTER t.feature_id == @feature_id COLLECT WITH COUNT INTO c RETURN c",
    { feature_id = self._key }
  )
  if result and result.result and result.result[1] then
    return result.result[1]
  end
  return 0
end

-- Count done tasks
function Feature:done_tasks_count()
  local result = Sdb:Sdbql(
    "FOR t IN tasks FILTER t.feature_id == @feature_id AND t.status == 'done' COLLECT WITH COUNT INTO c RETURN c",
    { feature_id = self._key }
  )
  if result and result.result and result.result[1] then
    return result.result[1]
  end
  return 0
end

-- Initialize default columns if not present
function Feature:ensure_columns()
  if not self.columns or #self.columns == 0 then
    self:update({ columns = Feature.DEFAULT_COLUMNS })
    self.data.columns = Feature.DEFAULT_COLUMNS
  end
  return self.columns or self.data.columns
end

-- Get sorted columns
function Feature:sorted_columns()
  local cols = self.columns or self.data.columns or Feature.DEFAULT_COLUMNS
  table.sort(cols, function(a, b) return (a.position or 0) < (b.position or 0) end)
  return cols
end

return Feature
