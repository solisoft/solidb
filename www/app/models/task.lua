local Model = require("model")

local Task = Model.create("tasks", {
  permitted_fields = { "title", "description", "status", "priority", "tags", "assignee_id", "reporter_id", "position", "feature_id", "comments", "files", "sequence_id" },
  validations = {
    title = { presence = true, length = { between = {1, 200} } },
    feature_id = { presence = true }
  },
  before_create = { "generate_sequence_id" }
})

-- Callback to generate sequence_id
function Task.generate_sequence_id(data)
  -- Generate sequence ID
  local result = Sdb:Sdbql([[
    UPSERT { _key: "tasks" } 
    INSERT { _key: "tasks", seq: 1 } 
    UPDATE { seq: OLD.seq + 1 } 
    IN counters 
    RETURN NEW.seq
  ]])
  
  if result and result.result and result.result[1] then
    data.sequence_id = result.result[1]
  else
    -- Fallback
    data.sequence_id = os.time() 
  end
  return data
end

-- Get parent feature (use get_ prefix to avoid conflict with data.feature)
function Task:get_feature()
  local Feature = require("models.feature")
  local feature_id = self.feature_id or (self.data and self.data.feature_id)
  if not feature_id then return nil end
  return Feature:find(feature_id)
end

-- Get app through feature
function Task:get_app()
  local feature = self:get_feature()
  if feature then
    return feature:get_app()
  end
  return nil
end

-- Get assignee user model (use get_ prefix to avoid conflict with data.assignee)
function Task:get_assignee()
  local assignee_id = self.assignee_id or (self.data and self.data.assignee_id)
  if not assignee_id then return nil end
  local User = require("models.user")
  return User:find(assignee_id)
end

-- Get reporter user model (use get_ prefix to avoid conflict with data.reporter)
function Task:get_reporter()
  local reporter_id = self.reporter_id or (self.data and self.data.reporter_id)
  if not reporter_id then return nil end
  local User = require("models.user")
  return User:find(reporter_id)
end

-- Get assignee info (lightweight - just basic fields)
function Task:assignee_info()
  local assignee_id = self.assignee_id or (self.data and self.data.assignee_id)
  if not assignee_id then return nil end
  local result = Sdb:Sdbql(
    "FOR u IN users FILTER u._key == @key LIMIT 1 RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname }",
    { key = assignee_id }
  )
  if result and result.result and result.result[1] then
    return result.result[1]
  end
  return nil
end

-- Get reporter info (lightweight)
function Task:reporter_info()
  local reporter_id = self.reporter_id or (self.data and self.data.reporter_id)
  if not reporter_id then return nil end
  local result = Sdb:Sdbql(
    "FOR u IN users FILTER u._key == @key LIMIT 1 RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname }",
    { key = reporter_id }
  )
  if result and result.result and result.result[1] then
    return result.result[1]
  end
  return nil
end

-- Add a comment to the task
function Task:add_comment(user_id, text)
  local comments = self.comments or self.data.comments or {}
  table.insert(comments, {
    id = os.time() .. "-" .. math.random(10000),
    text = text,
    user_id = user_id,
    created_at = os.time()
  })
  self:update({ comments = comments, updated_at = os.time() })
end

-- Get comments with user info
function Task:comments_with_users()
  local comments = self.comments or self.data.comments or {}
  for _, comment in ipairs(comments) do
    if comment.user_id then
      local result = Sdb:Sdbql(
        "FOR u IN users FILTER u._key == @key LIMIT 1 RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname }",
        { key = comment.user_id }
      )
      comment.user = (result and result.result and result.result[1])
    end
  end
  return comments
end

-- Add file attachment
function Task:attach_file(file_info, user_id)
  local files = self.files or self.data.files or {}
  table.insert(files, {
    key = file_info.key,
    name = file_info.name or "file",
    size = file_info.size or 0,
    content_type = file_info.content_type or "",
    uploaded_at = os.time(),
    uploaded_by = user_id
  })
  self:update({ files = files, updated_at = os.time() })
end

-- Remove file attachment
function Task:remove_file(file_key)
  local files = self.files or self.data.files or {}
  local new_files = {}
  for _, f in ipairs(files) do
    if f.key ~= file_key then
      table.insert(new_files, f)
    end
  end
  self:update({ files = new_files, updated_at = os.time() })
end

-- Find tasks assigned to a user
function Task.by_assignee(user_key, options)
  options = options or {}
  local instance = Task:new()
  instance = instance:where({ assignee_id = user_key })
  if options.status then
    instance = instance:where({ status = options.status })
  end
  if options.exclude_status then
    instance = instance:where_not({ status = options.exclude_status })
  end
  return instance:order("doc.position ASC"):all(options)
end

-- Find in-progress tasks for user (not todo, not done, not backlog)
function Task.in_progress_for_user(user_key, limit)
  local result = Sdb:Sdbql([[
    FOR t IN tasks
      FILTER t.assignee_id == @user_key
      FILTER t.status != "todo" AND t.status != "done" AND t.status != "backlog"
      LET priority_order = t.priority == "high" ? 0 : (t.priority == "medium" ? 1 : 2)
      SORT priority_order ASC, t.updated_at DESC
      LIMIT @limit
      RETURN t
  ]], { user_key = user_key, limit = limit or 10 })

  local tasks = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(tasks, Task:new(doc))
    end
  end
  return tasks
end

-- Find high priority todo tasks for user
function Task.todo_for_user(user_key, limit)
  local result = Sdb:Sdbql([[
    FOR t IN tasks
      FILTER t.assignee_id == @user_key
      FILTER t.status == "todo" AND t.priority == "high"
      SORT t.updated_at DESC
      LIMIT @limit
      RETURN t
  ]], { user_key = user_key, limit = limit or 10 })

  local tasks = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(tasks, Task:new(doc))
    end
  end
  return tasks
end

return Task
