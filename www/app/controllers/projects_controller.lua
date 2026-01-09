local Controller = require("controller")
local ProjectsController = Controller:extend()
local AuthHelper = require("helpers.auth_helper")
local App = require("models.app")
local Feature = require("models.feature")
local Task = require("models.task")

-- Get current user (middleware ensures user is authenticated)
local function get_current_user()
  return AuthHelper.get_current_user()
end

-- Get all users (for assignee dropdowns)
local function get_all_users()
  local result = Sdb:Sdbql("FOR u IN users RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname }")
  return (result and result.result) or {}
end

--------------------------------------------------------------------------------
-- APPS
--------------------------------------------------------------------------------

-- List all apps
function ProjectsController:index()
  local current_user = get_current_user()
  local apps = App:new():order("doc.position ASC, doc.created_at DESC"):all()

  -- Enrich with feature counts
  for _, app in ipairs(apps) do
    app.data.feature_count = app:features_count()
  end

  self.layout = "talks"
  self:render("projects/index", {
    current_user = current_user,
    apps = apps,
    db_name = Sdb._db_config and Sdb._db_config.db_name or "_system"
  })
end

-- Create app modal
function ProjectsController:app_modal_create()
  local current_user = get_current_user()

  self.layout = false
  self:render("projects/_app_modal", { current_user = current_user })
end

-- Create app
function ProjectsController:create_app()
  local current_user = get_current_user()
  local name = self.params.name

  if not name or name == "" then
    return self:html('<div class="text-red-400 text-sm">Name is required</div>')
  end

  App:create({
    name = name,
    description = self.params.description or "",
    color = self.params.color or "primary",
    status = "active",
    position = os.time(),
    created_by = current_user._key,
    created_at = os.time(),
    updated_at = os.time()
  })

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "appCreated")
    return self:html("")
  end
  return self:redirect("/projects")
end

-- Edit app modal
function ProjectsController:app_edit()
  local current_user = get_current_user()
  local app = App:find(self.params.key)

  if not app then
    return self:html('<div class="p-6 text-center text-text-dim">App not found</div>')
  end

  self.layout = false
  self:render("projects/_app_modal", {
    app = app,
    current_user = current_user
  })
end

-- Update app
function ProjectsController:update_app()
  local app = App:find(self.params.key)
  if not app then return self:html("") end

  local updates = { updated_at = os.time() }
  if self.params.name then updates.name = self.params.name end
  if self.params.description then updates.description = self.params.description end
  if self.params.color then updates.color = self.params.color end
  if self.params.status then updates.status = self.params.status end

  app:update(updates)

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "appUpdated")
    return self:html("")
  end
  return self:redirect("/projects")
end

-- Delete app
function ProjectsController:delete_app()
  local key = self.params.key

  -- Delete all tasks in features of this app
  Sdb:Sdbql("FOR f IN features FILTER f.app_id == @aid FOR t IN tasks FILTER t.feature_id == f._key REMOVE t IN tasks", { aid = key })
  -- Delete all features in this app
  Sdb:Sdbql("FOR f IN features FILTER f.app_id == @aid REMOVE f IN features", { aid = key })
  -- Delete the app
  local app = App:find(key)
  if app then app:destroy() end

  if self:is_htmx_request() then
    self:set_header("HX-Redirect", "/projects")
    return self:html("")
  end
  return self:redirect("/projects")
end

--------------------------------------------------------------------------------
-- FEATURES (within an App)
--------------------------------------------------------------------------------

-- List features for an app
function ProjectsController:features()
  local current_user = get_current_user()
  local app = App:find(self.params.app_key)

  if not app then
    return self:redirect("/projects")
  end

  local features = Feature:new():where({ app_id = self.params.app_key }):order("doc.position ASC, doc.created_at DESC"):all()

  -- Enrich with task counts
  for _, feature in ipairs(features) do
    feature.data.task_count = feature:tasks_count()
    feature.data.done_count = feature:done_tasks_count()
  end

  self.layout = "talks"
  self:render("projects/features", {
    current_user = current_user,
    app = app,
    features = features,
    db_name = Sdb._db_config and Sdb._db_config.db_name or "_system"
  })
end

-- Create feature modal
function ProjectsController:feature_modal_create()
  local current_user = get_current_user()

  self.layout = false
  self:render("projects/_feature_modal", {
    app_key = self.params.app_key,
    current_user = current_user
  })
end

-- Create feature
function ProjectsController:create_feature()
  local current_user = get_current_user()
  local app_key = self.params.app_key
  local name = self.params.name

  if not name or name == "" then
    return self:html('<div class="text-red-400 text-sm">Name is required</div>')
  end

  -- Handle tags (comma-separated string)
  local tags = {}
  if self.params.tags and self.params.tags ~= "" then
    for tag in string.gmatch(self.params.tags, "[^,]+") do
      table.insert(tags, tag)
    end
  end

  Feature:create({
    app_id = app_key,
    name = name,
    description = self.params.description or "",
    color = self.params.color or "primary",
    tags = tags,
    status = "active",
    position = os.time(),
    created_by = current_user._key,
    created_at = os.time(),
    updated_at = os.time(),
    columns = Feature.DEFAULT_COLUMNS
  })

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "featureCreated")
    return self:html("")
  end
  return self:redirect("/projects/app/" .. app_key)
end

-- Edit feature modal
function ProjectsController:feature_edit()
  local current_user = get_current_user()
  local feature = Feature:find(self.params.key)

  if not feature then
    return self:html('<div class="p-6 text-center text-text-dim">Feature not found</div>')
  end

  self.layout = false
  self:render("projects/_feature_modal", {
    app_key = self.params.app_key,
    feature = feature,
    current_user = current_user
  })
end

-- Update feature
function ProjectsController:update_feature()
  local app_key = self.params.app_key
  local feature = Feature:find(self.params.key)
  if not feature then return self:html("") end

  local updates = { updated_at = os.time() }
  if self.params.name then updates.name = self.params.name end
  if self.params.description then updates.description = self.params.description end
  if self.params.color then updates.color = self.params.color end
  if self.params.status then updates.status = self.params.status end

  -- Handle tags (comma-separated string)
  if self.params.tags ~= nil then
    local tags = {}
    if self.params.tags and self.params.tags ~= "" then
      for tag in string.gmatch(self.params.tags, "[^,]+") do
        table.insert(tags, tag)
      end
    end
    updates.tags = tags
  end

  feature:update(updates)

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "featureUpdated")
    return self:html("")
  end
  return self:redirect("/projects/app/" .. app_key)
end

-- Delete feature
function ProjectsController:delete_feature()
  local app_key = self.params.app_key
  local key = self.params.key

  -- Delete all tasks in this feature
  Sdb:Sdbql("FOR t IN tasks FILTER t.feature_id == @fid REMOVE t IN tasks", { fid = key })
  -- Delete the feature
  local feature = Feature:find(key)
  if feature then feature:destroy() end

  if self:is_htmx_request() then
    self:set_header("HX-Redirect", "/projects/app/" .. app_key)
    return self:html("")
  end
  return self:redirect("/projects/app/" .. app_key)
end

--------------------------------------------------------------------------------
-- FEATURE BOARD (Tasks Kanban)
--------------------------------------------------------------------------------

-- Feature board view
function ProjectsController:board()
  local current_user = get_current_user()
  local app_key = self.params.app_key
  local feature_key = self.params.key

  local app = App:find(app_key)
  if not app then
    return self:redirect("/projects")
  end

  local feature = Feature:find(feature_key)
  if not feature then
    return self:redirect("/projects/app/" .. app_key)
  end

  -- Initialize default columns if not present
  feature:ensure_columns()
  local columns = feature:sorted_columns()

  -- Fetch tasks for this feature
  local tasks = feature:tasks()

  -- Enrich with assignee info
  for _, task in ipairs(tasks) do
    task.data.assignee = task:assignee_info()
  end

  -- Group tasks by column status
  local tasks_by_column = {}
  for _, col in ipairs(columns) do
    tasks_by_column[col.id] = {}
  end

  for _, task in ipairs(tasks) do
    local status = task.status or task.data.status or "todo"
    if tasks_by_column[status] then
      table.insert(tasks_by_column[status], task)
    else
      -- Task has unknown status, put in first column
      local first_col = columns[1]
      if first_col then
        table.insert(tasks_by_column[first_col.id], task)
      end
    end
  end

  self.layout = "talks"
  self:render("projects/board", {
    current_user = current_user,
    app = app,
    feature = feature,
    columns = columns,
    tasks_by_column = tasks_by_column,
    users = get_all_users(),
    db_name = Sdb._db_config and Sdb._db_config.db_name or "_system"
  })
end

-- Get tasks for a specific column (HTMX partial)
function ProjectsController:column()
  local current_user = get_current_user()
  local feature_key = self.params.feature_key
  local status = self.params.status or "todo"

  local tasks = Task:new():where({ feature_id = feature_key, status = status }):order("doc.position ASC, doc.created_at DESC"):all()

  for _, task in ipairs(tasks) do
    task.data.assignee = task:assignee_info()
  end

  self.layout = false
  self:render("projects/_column_tasks", {
    tasks = tasks,
    status = status,
    current_user = current_user
  })
end

--------------------------------------------------------------------------------
-- COLUMNS
--------------------------------------------------------------------------------

-- Columns management modal
function ProjectsController:columns_modal()
  local current_user = get_current_user()
  local feature = Feature:find(self.params.feature_key)

  if not feature then
    return self:html('<div class="p-6 text-center text-text-dim">Feature not found</div>')
  end

  feature:ensure_columns()
  local columns = feature:sorted_columns()

  self.layout = false
  self:render("projects/_columns_modal", {
    app_key = self.params.app_key,
    feature_key = self.params.feature_key,
    feature = feature,
    columns = columns,
    current_user = current_user
  })
end

-- Add column
function ProjectsController:add_column()
  local app_key = self.params.app_key
  local feature_key = self.params.feature_key
  local name = self.params.name
  local color = self.params.color or "text-dim"

  if not name or name == "" then
    return self:html('<div class="text-red-400 text-sm">Name is required</div>')
  end

  local feature = Feature:find(feature_key)
  if not feature then return self:html("") end

  local columns = feature.columns or feature.data.columns or {}
  local max_position = 0
  for _, col in ipairs(columns) do
    if (col.position or 0) > max_position then
      max_position = col.position
    end
  end

  -- Generate unique ID
  local id = string.lower(name):gsub("%s+", "_"):gsub("[^%w_]", "")
  if id == "" then id = "col_" .. os.time() end

  -- Check for duplicate ID
  for _, col in ipairs(columns) do
    if col.id == id then
      id = id .. "_" .. os.time()
      break
    end
  end

  table.insert(columns, {
    id = id,
    name = name,
    color = color,
    position = max_position + 1
  })

  feature:update({ columns = columns, updated_at = os.time() })

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "columnsUpdated")
    return self:html("")
  end
  return self:redirect("/projects/app/" .. app_key .. "/feature/" .. feature_key)
end

-- Update columns (reorder/rename/delete)
function ProjectsController:update_columns()
  local feature = Feature:find(self.params.feature_key)
  if not feature then
    return self:json({ error = "Feature not found" }, 404)
  end

  local columns_json = self.params.columns
  if not columns_json then
    return self:json({ error = "Missing columns" }, 400)
  end

  local columns = columns_json
  if type(columns_json) == "string" then
    local ok, parsed = pcall(function() return require("json").decode(columns_json) end)
    if ok then columns = parsed end
  end

  feature:update({ columns = columns, updated_at = os.time() })

  return self:json({ success = true })
end

-- Delete column
function ProjectsController:delete_column()
  local app_key = self.params.app_key
  local feature_key = self.params.feature_key
  local column_id = self.params.column_id
  local move_to = self.params.move_to or "todo"

  local feature = Feature:find(feature_key)
  if not feature or not (feature.columns or feature.data.columns) then return self:html("") end

  local columns = feature.columns or feature.data.columns

  -- Remove column
  local new_columns = {}
  for _, col in ipairs(columns) do
    if col.id ~= column_id then
      table.insert(new_columns, col)
    end
  end

  -- Move tasks from deleted column to target column
  Sdb:Sdbql("FOR t IN tasks FILTER t.feature_id == @fid AND t.status == @old_status UPDATE t WITH { status: @new_status } IN tasks", {
    fid = feature_key,
    old_status = column_id,
    new_status = move_to
  })

  feature:update({ columns = new_columns, updated_at = os.time() })

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "columnsUpdated")
    return self:html("")
  end
  return self:redirect("/projects/app/" .. app_key .. "/feature/" .. feature_key)
end

--------------------------------------------------------------------------------
-- TASKS
--------------------------------------------------------------------------------

-- Create task modal
function ProjectsController:task_modal_create()
  local current_user = get_current_user()

  self.layout = false
  self:render("projects/_task_modal", {
    app_key = self.params.app_key,
    feature_key = self.params.feature_key,
    users = get_all_users(),
    current_user = current_user
  })
end

-- Create task
function ProjectsController:create_task()
  local current_user = get_current_user()
  local app_key = self.params.app_key
  local feature_key = self.params.feature_key
  local title = self.params.title

  if not title or title == "" then
    return self:html('<div class="text-red-400 text-sm">Title is required</div>')
  end

  -- Handle tags
  local tags = {}
  if self.params.tags and self.params.tags ~= "" then
    for tag in string.gmatch(self.params.tags, "[^,]+") do
      table.insert(tags, tag)
    end
  end

  Task:create({
    feature_id = feature_key,
    title = title,
    description = self.params.description or "",
    status = "todo",
    priority = self.params.priority or "medium",
    tags = tags,
    assignee_id = (self.params.assignee_id and self.params.assignee_id ~= "") and self.params.assignee_id or nil,
    reporter_id = current_user._key,
    position = os.time(),
    created_at = os.time(),
    updated_at = os.time(),
    comments = {}
  })

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "taskCreated")
    return self:html("")
  end

  return self:redirect("/projects/app/" .. app_key .. "/feature/" .. feature_key)
end

-- Assignee dropdown
function ProjectsController:assignee_dropdown()
  local current_user = get_current_user()
  local task = Task:find(self.params.key)

  self.layout = false
  self:render("projects/_assignee_dropdown", {
    task_key = self.params.key,
    users = get_all_users(),
    current_assignee = task and (task.assignee_id or task.data.assignee_id) or nil,
    current_user = current_user
  })
end

-- Get task card HTML (for real-time updates)
function ProjectsController:task_card()
  local task = Task:find(self.params.key)
  if not task then return self:html("") end

  task.data.assignee = task:assignee_info()

  self.layout = false
  self:render("projects/_task_card", { task = task })
end

-- Get task row HTML (for real-time updates)
function ProjectsController:task_row()
  local task = Task:find(self.params.key)
  if not task then return self:html("") end

  task.data.assignee = task:assignee_info()

  -- Fetch columns for the dropdown order
  local columns = {}
  local feature_id = task.feature_id or task.data.feature_id
  if feature_id then
    local feature = Feature:find(feature_id)
    if feature then
      feature:ensure_columns()
      columns = feature:sorted_columns()
    end
  end

  self.layout = false
  self:render("projects/_task_row", { task = task, columns = columns })
end

-- Assign task to user
function ProjectsController:assign_task()
  local task = Task:find(self.params.key)
  if not task then
    return self:json({ error = "Task not found" }, 404)
  end

  local assignee_id = self.params.assignee_id
  task:update({
    assignee_id = (assignee_id and assignee_id ~= "") and assignee_id or nil,
    updated_at = os.time()
  })

  task.data.assignee = task:assignee_info()

  self.layout = false
  self:render("projects/_task_card", { task = task })
end

-- Update task status (drag & drop)
function ProjectsController:update_status()
  local key = self.params.key
  local status = self.params.status
  local position = self.params.position

  if not key or not status then
    return self:json({ error = "Missing parameters" }, 400)
  end

  local task = Task:find(key)
  if not task then
    return self:json({ error = "Task not found" }, 404)
  end

  local updates = {
    status = status,
    updated_at = os.time()
  }

  if position then
    updates.position = tonumber(position)
  end

  task:update(updates)

  self:json({ success = true })
end

-- Update task priority
function ProjectsController:update_priority()
  local key = self.params.key
  local priority = self.params.priority

  if not key or not priority then
    return self:json({ error = "Missing parameters" }, 400)
  end

  local task = Task:find(key)
  if not task then
    return self:json({ error = "Task not found" }, 404)
  end

  task:update({
    priority = priority,
    updated_at = os.time()
  })

  self:json({ success = true })
end

-- Show task details
function ProjectsController:show()
  local current_user = get_current_user()
  local task = Task:find(self.params.key)

  if not task then
    return self:html('<div class="p-6 text-center text-text-dim">Task not found</div>')
  end

  -- Get feature and app for breadcrumb
  local feature_id = task.feature_id or task.data.feature_id
  if feature_id then
    task.data.feature = Feature:find(feature_id)
    if task.data.feature then
      local app_id = task.data.feature.app_id or task.data.feature.data.app_id
      if app_id then
        task.data.app = App:find(app_id)
      end
    end
  end

  -- Enrich assignee and reporter
  task.data.assignee = task:assignee_info()
  task.data.reporter = task:reporter_info()

  -- Enrich comments with user info
  task.data.comments = task:comments_with_users()

  self.layout = false
  self:render("projects/_task_details", {
    task = task,
    users = get_all_users(),
    current_user = current_user
  })
end

-- Add comment
function ProjectsController:add_comment()
  local current_user = get_current_user()
  local key = self.params.key
  local text = self.params.text

  if not key or not text or text == "" then
    return self:show()
  end

  local task = Task:find(key)
  if task then
    task:add_comment(current_user._key, text)
  end

  return self:show()
end

-- Update task
function ProjectsController:update()
  local task = Task:find(self.params.key)
  if not task then return self:show() end

  local updates = { updated_at = os.time() }

  if self.params.title then updates.title = self.params.title end
  if self.params.description then updates.description = self.params.description end
  if self.params.priority then updates.priority = self.params.priority end
  if self.params.assignee_id then
    updates.assignee_id = self.params.assignee_id ~= "" and self.params.assignee_id or nil
  end

  task:update(updates)

  return self:show()
end

-- Update task tags
function ProjectsController:update_tags()
  local task = Task:find(self.params.key)
  if not task then
    return self:json({ error = "Task not found" }, 404)
  end

  -- Handle tags (comma-separated string)
  local tags = {}
  if self.params.tags and self.params.tags ~= "" then
    for tag in string.gmatch(self.params.tags, "[^,]+") do
      table.insert(tags, tag)
    end
  end

  task:update({ tags = tags, updated_at = os.time() })

  task.data.assignee = task:assignee_info()

  self.layout = false
  self:render("projects/_task_card", { task = task })
end

-- Add file to task
function ProjectsController:attach_file()
  local current_user = get_current_user()
  local task = Task:find(self.params.key)

  if not task then
    return self:json({ error = "Task not found" }, 404)
  end

  task:attach_file({
    key = self.params.file_key or self.params["key"],
    name = self.params.name or "file",
    size = tonumber(self.params.size) or 0,
    content_type = self.params.content_type or ""
  }, current_user._key)

  return self:json({ success = true })
end

-- Remove file from task
function ProjectsController:remove_file()
  local task = Task:find(self.params.key)
  if not task then
    return self:json({ error = "Task not found" }, 404)
  end

  task:remove_file(self.params.file_key)

  return self:json({ success = true })
end

-- Delete task
function ProjectsController:delete()
  local task = Task:find(self.params.key)
  local redirect_url = "/projects"

  if task then
    local feature_id = task.feature_id or task.data.feature_id
    if feature_id then
      local feature = Feature:find(feature_id)
      if feature then
        local app_id = feature.app_id or feature.data.app_id
        if app_id then
          redirect_url = "/projects/app/" .. app_id .. "/feature/" .. feature_id
        end
      end
    end
    task:destroy()
  end

  if self:is_htmx_request() then
    self:set_header("HX-Redirect", redirect_url)
    return self:html("")
  end

  return self:redirect(redirect_url)
end

-- Sidebar: My in-progress tasks
function ProjectsController:sidebar_in_progress()
  local current_user = get_current_user()
  local tasks = Task.in_progress_for_user(current_user._key, 10)

  -- Enrich with feature and app info
  for _, task in ipairs(tasks) do
    local feature_id = task.feature_id or task.data.feature_id
    if feature_id then
      task.data.feature = Feature:find(feature_id)
      if task.data.feature then
        local app_id = task.data.feature.app_id or task.data.feature.data.app_id
        if app_id then
          task.data.app = App:find(app_id)
        end
      end
    end
  end

  self.layout = false
  return self:render("projects/_sidebar_tasks", {
    tasks = tasks,
    section_title = "In Progress",
    empty_message = "No tasks in progress"
  })
end

-- Sidebar: Latest tasks assigned to me
function ProjectsController:sidebar_my_tasks()
  local current_user = get_current_user()
  local tasks = Task.todo_for_user(current_user._key, 10)

  -- Enrich with feature and app info
  for _, task in ipairs(tasks) do
    local feature_id = task.feature_id or task.data.feature_id
    if feature_id then
      task.data.feature = Feature:find(feature_id)
      if task.data.feature then
        local app_id = task.data.feature.app_id or task.data.feature.data.app_id
        if app_id then
          task.data.app = App:find(app_id)
        end
      end
    end
  end

  self.layout = false
  return self:render("projects/_sidebar_tasks", {
    tasks = tasks,
    section_title = "To Do",
    empty_message = "No tasks to do"
  })
end

-- Sidebar: Apps list
function ProjectsController:sidebar_apps()
  local apps = App:new():order("doc.position ASC, doc.created_at DESC"):all()

  self.layout = false
  return self:render("projects/_sidebar_apps", { apps = apps })
end

return ProjectsController
