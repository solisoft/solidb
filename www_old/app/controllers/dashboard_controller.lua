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

  live_query = function(self)
    Page("dashboard/live_query", "app")
  end,

  cluster = function()
    Page("dashboard/cluster", "app")
  end,

  scripts = function()
    Page("dashboard/scripts", "app")
  end,

  repl = function()
    Page("dashboard/repl", "app")
  end,

  queues = function()
    Page("dashboard/queues", "app")
  end,

  sharding = function()
    Page("dashboard/sharding", "app")
  end,

  apikeys = function()
    Page("dashboard/apikeys", "app")
  end,

  monitoring = function()
    Page("dashboard/monitoring", "app")
  end,

  users = function()
    Page("dashboard/users", "app")
  end,

  env = function()
    Page("dashboard/env", "app")
  end,

  columnar = function()
    Page("dashboard/columnar", "app")
  end,

  ai_contributions = function()
    Page("dashboard/ai/contributions", "app")
  end,

  ai_tasks = function()
    Page("dashboard/ai/tasks", "app")
  end,

  ai_agents = function()
    Page("dashboard/ai/agents", "app")
  end,

  create_ai_task = function()
    local payload = GetBodyParams()
    local db_name = Params['db']

    -- Validation
    if not payload.task_type or payload.task_type == "" then
      SetStatus(400)
      return WriteJSON({ error = "task_type is required" })
    end

    local db = SoliDB[db_name]
    if not db then
       SetStatus(404)
       return WriteJSON({ error = "Database not found" })
    end
    
    -- Ensure collection exists
    if not db:HasCollection("_ai_tasks") then
        db:Run("CREATE COLLECTION _ai_tasks")
    end

    local task = {
       task_type = payload.task_type,
       status = "pending",
       priority = tonumber(payload.priority) or 0,
       payload = payload.payload or {},
       created_at = os.date("!%Y-%m-%dT%H:%M:%SZ"),
       updated_at = os.date("!%Y-%m-%dT%H:%M:%SZ")
    }

    local doc_key = db:ValidateAndInsert("_ai_tasks", task)
    
    if doc_key then
       task._key = doc_key
       WriteJSON(task)
    else
       SetStatus(500)
       WriteJSON({ error = "Failed to create task" })
    end
  end,

  create_ai_agent = function()
    local payload = GetBodyParams()
    local db_name = Params['db']

    -- Validation
    if not payload.name or payload.name == "" then
      SetStatus(400)
      return WriteJSON({ error = "name is required" })
    end

    local db = SoliDB[db_name]
    if not db then
       SetStatus(404)
       return WriteJSON({ error = "Database not found" })
    end
    
    -- Ensure collection exists
    if not db:HasCollection("_ai_agents") then
        db:Run("CREATE COLLECTION _ai_agents")
    end

    local agent = {
       name = payload.name,
       agent_type = payload.agent_type or "generic",
       status = payload.status or "idle",
       capabilities = payload.capabilities or {},
       last_heartbeat = os.date("!%Y-%m-%dT%H:%M:%SZ"),
       tasks_completed = 0,
       tasks_failed = 0,
       created_at = os.date("!%Y-%m-%dT%H:%M:%SZ"),
       updated_at = os.date("!%Y-%m-%dT%H:%M:%SZ")
    }

    local doc_key = db:ValidateAndInsert("_ai_agents", agent)
    
    if doc_key then
       agent._key = doc_key
       WriteJSON(agent)
    else
       SetStatus(500)
       WriteJSON({ error = "Failed to create agent" })
    end
  end
}

return BeansEnv == "development" and HandleController(app) or app
