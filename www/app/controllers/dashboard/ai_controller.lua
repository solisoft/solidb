-- Dashboard AI Controller
-- Handles AI contributions, tasks, and agents
local DashboardBaseController = require("dashboard.base_controller")
local AIController = DashboardBaseController:extend()

--------------------------------------------------------------------------------
-- AI Contributions
--------------------------------------------------------------------------------

function AIController:contributions()
  self.layout = "dashboard"
  self:render("dashboard/ai/contributions", {
    title = "AI Contributions - " .. self:get_db(),
    db = self:get_db(),
    current_page = "ai_contributions"
  })
end

function AIController:contributions_table()
  local db = self:get_db()
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/ai/contributions")

  local contributions = {}
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      contributions = data.contributions or data or {}
    end
  end

  self:render_partial("dashboard/ai/_contributions_table", {
    contributions = contributions,
    db = db
  })
end

function AIController:contributions_stats()
  local db = self:get_db()
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/ai/contributions")

  -- Count contributions by status
  local stats = { submitted = 0, analyzing = 0, review = 0, merged = 0, total = 0 }
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      local contributions = data.contributions or data or {}
      stats.total = #contributions
      for _, c in ipairs(contributions) do
        local s = c.status or ""
        if s == "submitted" then
          stats.submitted = stats.submitted + 1
        elseif s == "analyzing" then
          stats.analyzing = stats.analyzing + 1
        elseif s == "review" then
          stats.review = stats.review + 1
        elseif s == "merged" or s == "approved" then
          stats.merged = stats.merged + 1
        end
      end
    end
  end

  self:render_partial("dashboard/ai/_contributions_stats", {
    stats = stats
  })
end

function AIController:contributions_modal_create()
  -- Fetch collections for the related_collections dropdown
  local db = self:get_db()
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collection")

  local collections = {}
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      collections = data or {}
    end
  end

  self:render_partial("dashboard/ai/_modal_create_contribution", {
    db = db,
    collections = collections
  })
end

function AIController:create_contribution()
  local db = self:get_db()
  local contribution_type = self.params.type or "feature"
  local description = self.params.description or ""
  local related_collections = self.params.related_collections or ""
  local priority = self.params.priority or "medium"

  if description == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Description is required", "type": "error"}}')
    return self:contributions_table()
  end

  -- Parse related_collections from comma-separated string to array
  local collections_array = {}
  if related_collections ~= "" then
    for coll in string.gmatch(related_collections, "[^,]+") do
      table.insert(collections_array, coll:match("^%s*(.-)%s*$"))  -- trim whitespace
    end
  end

  local request_body = {
    type = contribution_type,
    description = description,
    context = {
      related_collections = collections_array,
      priority = priority
    }
  }

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/ai/contributions", {
    method = "POST",
    body = EncodeJson(request_body)
  })

  if status == 200 or status == 201 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Contribution submitted successfully", "type": "success"}}')
  else
    local err_msg = "Failed to submit contribution"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:contributions_table()
end

function AIController:cancel_contribution()
  local db = self:get_db()
  local contribution_id = self.params.contribution_id

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/ai/contributions/" .. contribution_id .. "/cancel", {
    method = "POST"
  })

  if status == 200 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Contribution cancelled", "type": "success"}}')
    self:html("")
  else
    local err_msg = "Failed to cancel contribution"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
    self:html("")
  end
end

--------------------------------------------------------------------------------
-- AI Tasks
--------------------------------------------------------------------------------

function AIController:tasks()
  self.layout = "dashboard"
  self:render("dashboard/ai/tasks", {
    title = "AI Tasks - " .. self:get_db(),
    db = self:get_db(),
    current_page = "ai_tasks"
  })
end

function AIController:tasks_stats()
  local db = self:get_db()
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/ai/tasks/stats")

  local stats = { running = 0, queued = 0, completed = 0, failed = 0 }
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      stats = data
    end
  end

  self:render_partial("dashboard/ai/_tasks_stats", {
    stats = stats
  })
end

function AIController:tasks_table()
  local db = self:get_db()
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/ai/tasks")

  local tasks = {}
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      tasks = data.tasks or data or {}
    end
  end

  self:render_partial("dashboard/ai/_tasks_table", {
    tasks = tasks,
    db = db
  })
end

function AIController:cancel_task()
  local db = self:get_db()
  local task_id = self.params.task_id

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/ai/tasks/" .. task_id, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Task cancelled", "type": "success"}}')
    self:html("")
  else
    SetHeader("HX-Trigger", '{"showToast": {"message": "Failed to cancel task", "type": "error"}}')
    self:html("")
  end
end

--------------------------------------------------------------------------------
-- AI Agents
--------------------------------------------------------------------------------

function AIController:agents()
  self.layout = "dashboard"
  self:render("dashboard/ai/agents", {
    title = "AI Agents - " .. self:get_db(),
    db = self:get_db(),
    current_page = "ai_agents"
  })
end

function AIController:agents_grid()
  local db = self:get_db()
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/ai/agents")

  local agents = {}
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      agents = data.agents or data or {}
      -- Normalize agents
      for _, agent in ipairs(agents) do
        agent.id = agent._key or agent.id
        if agent.config then
          agent.model = agent.model or agent.config.model
          agent.system_prompt = agent.system_prompt or agent.config.system_prompt
        end
      end
    end
  end

  self:render_partial("dashboard/ai/_agents_grid", {
    agents = agents,
    db = db
  })
end

function AIController:agents_modal_create()
  self:render_partial("dashboard/ai/_modal_agent", {
    agent = nil,
    db = self:get_db(),
    is_edit = false
  })
end

function AIController:agents_modal_edit()
  local db = self:get_db()
  local agent_id = self.params.agent_id

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/ai/agents/" .. agent_id)
  local agent = nil
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      agent = data.agent or data
      -- Normalize agent data for view
      if agent then
        agent.id = agent._key or agent.id
        if agent.config then
          agent.model = agent.model or agent.config.model
          agent.system_prompt = agent.system_prompt or agent.config.system_prompt
        end
      end
    end
  end

  self:render_partial("dashboard/ai/_modal_agent", {
    agent = agent,
    db = db,
    is_edit = true
  })
end

function AIController:create_agent()
  local db = self:get_db()
  local name = self.params.name or ""
  local model = self.params.model or "claude-3-5-haiku"
  local system_prompt = self.params.system_prompt or ""
  ngx.req.read_body()
  local post_args = ngx.req.get_post_args()
  local raw_capabilities = post_args["capabilities[]"] or self.params.capabilities
  print("DEBUG: create_agent params:", EncodeJson(self.params))
  print("DEBUG: raw_capabilities type:", type(raw_capabilities))
  if type(raw_capabilities) == "table" then
    print("DEBUG: raw_capabilities content:", EncodeJson(raw_capabilities))
  else
    print("DEBUG: raw_capabilities value:", tostring(raw_capabilities))
  end
  local api_url = self.params.api_url or ""
  local api_key = self.params.api_key or ""

  if name == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Agent name is required", "type": "error"}}')
    return self:agents_grid()
  end

  -- Robust capabilities parsing
  local capabilities = {}
  if type(raw_capabilities) == "table" then
    -- Handle array/map from Lapis
    for _, v in pairs(raw_capabilities) do
      if type(v) == "string" and v ~= "" then
        table.insert(capabilities, v)
      end
    end
  elseif type(raw_capabilities) == "string" and raw_capabilities ~= "" then
    table.insert(capabilities, raw_capabilities)
  end

  -- Build config object
  local config = {}
  if api_url ~= "" then config.api_url = api_url end
  if api_key ~= "" then config.api_key = api_key end

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/ai/agents", {
    method = "POST",
    body = EncodeJson({
      name = name,
      model = model,
      system_prompt = system_prompt,
      capabilities = capabilities,
      config = config
    })
  })

  if status == 200 or status == 201 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Agent created successfully", "type": "success"}, "agentUpdated": true}')
  else
    local err_msg = "Failed to create agent"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:agents_grid()
end

function AIController:update_agent()
  local db = self:get_db()
  local agent_id = self.params.agent_id
  local name = self.params.name or ""
  local model = self.params.model or "claude-3-5-haiku"
  local system_prompt = self.params.system_prompt or ""
  ngx.req.read_body()
  local post_args = ngx.req.get_post_args()
  local raw_capabilities = post_args["capabilities[]"] or self.params.capabilities
  print("DEBUG: update_agent params:", EncodeJson(self.params))
  print("DEBUG: raw_capabilities type:", type(raw_capabilities))
  if type(raw_capabilities) == "table" then
    print("DEBUG: raw_capabilities content:", EncodeJson(raw_capabilities))
  else
    print("DEBUG: raw_capabilities value:", tostring(raw_capabilities))
  end
  local api_url = self.params.api_url or ""
  local api_key = self.params.api_key or ""

  -- Robust capabilities parsing
  local capabilities = {}
  if type(raw_capabilities) == "table" then
    -- Handle array/map from Lapis
    for _, v in pairs(raw_capabilities) do
      if type(v) == "string" and v ~= "" then
        table.insert(capabilities, v)
      end
    end
  elseif type(raw_capabilities) == "string" and raw_capabilities ~= "" then
    table.insert(capabilities, raw_capabilities)
  end

  -- Build config object
  local config = {}
  if api_url ~= "" then config.api_url = api_url end
  if api_key ~= "" then config.api_key = api_key end

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/ai/agents/" .. agent_id, {
    method = "PUT",
    body = EncodeJson({
      name = name,
      model = model,
      system_prompt = system_prompt,
      capabilities = capabilities,
      config = config
    })
  })
  print("DEBUG: update_agent backend status:", status)
  print("DEBUG: update_agent backend body:", body)

  if status == 200 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Agent updated", "type": "success"}, "agentUpdated": true}')
  else
    local err_msg = "Failed to update agent"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:agents_grid()
end

function AIController:delete_agent()
  local db = self:get_db()
  local agent_id = self.params.agent_id

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/ai/agents/" .. agent_id, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Agent deleted", "type": "success"}}')
    self:html("")
  else
    SetHeader("HX-Trigger", '{"showToast": {"message": "Failed to delete agent", "type": "error"}}')
    self:html("")
  end
end

return AIController
