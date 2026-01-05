-- Dashboard Admin Controller
-- Handles users, API keys, databases, sharding, env, and queues
local DashboardBaseController = require("dashboard.base_controller")
local AdminController = DashboardBaseController:extend()

-- Databases page (system only)
function AdminController:databases()
  self.layout = "dashboard"
  self:render("dashboard/databases", {
    title = "Databases - SoliDB",
    db = "_system",
    current_page = "databases"
  })
end

-- Databases list (HTMX partial)
function AdminController:databases_list()
  local status, _, body = self:fetch_api("/_api/databases")
  local databases = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      databases = data.databases or data or {}
    end
  end

  -- Fetch users count
  local users_count = 0
  local users_status, _, users_body = self:fetch_api("/_api/auth/users")
  if users_status == 200 then
    local ok, users_data = pcall(DecodeJson, users_body)
    if ok and users_data then
      local users = users_data.users or users_data or {}
      users_count = #users
    end
  end

  self:render_partial("dashboard/_databases_list", {
    databases = databases,
    users_count = users_count
  })
end

-- Create database modal
function AdminController:databases_modal_create()
  self:render_partial("dashboard/_modal_create_database", {})
end

-- Create database action
function AdminController:create_database()
  local name = self.params.name

  if not name or name == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Database name is required", "type": "error"}}')
    return self:databases_list()
  end

  local status, _, body = self:fetch_api("/_api/database", {
    method = "POST",
    body = EncodeJson({ name = name })
  })

  if status == 200 or status == 201 then
    -- Create _luaonbeans system collection in the new database
    self:fetch_api("/_api/database/" .. name .. "/collection", {
      method = "POST",
      body = EncodeJson({ name = "_luaonbeans", type = "document" })
    })
    SetHeader("HX-Trigger", '{"showToast": {"message": "Database created successfully", "type": "success"}}')
  else
    local err_msg = "Failed to create database"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:databases_list()
end

-- Delete database action
function AdminController:delete_database()
  local name = self.params.name

  if name == "_system" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Cannot delete system database", "type": "error"}}')
    return self:html("")
  end

  local status, _, body = self:fetch_api("/_api/database/" .. name, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Database deleted successfully", "type": "success"}}')
    self:html("")  -- Remove the card
  else
    SetHeader("HX-Trigger", '{"showToast": {"message": "Failed to delete database", "type": "error"}}')
    self:html("")
  end
end

-- Users page
function AdminController:users()
  self.layout = "dashboard"
  self:render("dashboard/users", {
    title = "Users & Roles - SoliDB",
    db = "_system",
    current_page = "users"
  })
end

-- Users table
function AdminController:users_table()
  local status, _, body = self:fetch_api("/_api/auth/users")
  local users = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      users = data.users or data or {}
    end
  end

  -- Fetch roles for each user
  for _, user in ipairs(users) do
    local roles_status, _, roles_body = self:fetch_api("/_api/auth/users/" .. user.username .. "/roles")
    if roles_status == 200 then
      local ok, roles_data = pcall(DecodeJson, roles_body)
      if ok and roles_data then
        user.roles = roles_data.roles or roles_data or {}
      end
    end
    user.roles = user.roles or {}
  end

  self:render_partial("dashboard/_users_table", { users = users })
end

-- Create user modal
function AdminController:users_modal_create()
  -- Fetch available roles for the dropdown
  local status, _, body = self:fetch_api("/_api/auth/roles")
  local roles = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      roles = data.roles or data or {}
    end
  end

  self:render_partial("dashboard/_modal_create_user", { roles = roles })
end

-- Create user action
function AdminController:create_user()
  local username = self.params.username
  local password = self.params.password
  local roles = self.params.roles or {}

  if not username or username == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Username is required", "type": "error"}}')
    return self:users_table()
  end

  if not password or password == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Password is required", "type": "error"}}')
    return self:users_table()
  end

  -- Create the user
  local status, _, body = self:fetch_api("/_api/auth/users", {
    method = "POST",
    body = EncodeJson({ username = username, password = password })
  })

  if status == 200 or status == 201 then
    -- Assign roles if any
    if type(roles) == "string" then
      roles = { roles }
    end
    for _, role in ipairs(roles) do
      self:fetch_api("/_api/auth/users/" .. username .. "/roles", {
        method = "POST",
        body = EncodeJson({ role = role })
      })
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "User created successfully", "type": "success"}}')
  else
    local err_msg = "Failed to create user"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:users_table()
end

-- Delete user action
function AdminController:delete_user()
  local username = self.params.username

  if username == "root" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Cannot delete root user", "type": "error"}}')
    return self:html("")
  end

  local status, _, body = self:fetch_api("/_api/auth/users/" .. username, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "User deleted successfully", "type": "success"}}')
    self:html("")
  else
    SetHeader("HX-Trigger", '{"showToast": {"message": "Failed to delete user", "type": "error"}}')
    self:html("")
  end
end

-- Assign role to user
function AdminController:assign_role()
  local username = self.params.username
  local role = self.params.role

  local status, _, body = self:fetch_api("/_api/auth/users/" .. username .. "/roles", {
    method = "POST",
    body = EncodeJson({ role = role })
  })

  if status == 200 or status == 201 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Role assigned successfully", "type": "success"}}')
  else
    SetHeader("HX-Trigger", '{"showToast": {"message": "Failed to assign role", "type": "error"}}')
  end

  self:users_table()
end

-- Revoke role from user
function AdminController:revoke_role()
  local username = self.params.username
  local role = self.params.role

  local status, _, body = self:fetch_api("/_api/auth/users/" .. username .. "/roles/" .. role, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Role revoked successfully", "type": "success"}}')
  else
    SetHeader("HX-Trigger", '{"showToast": {"message": "Failed to revoke role", "type": "error"}}')
  end

  self:users_table()
end

-- Roles table
function AdminController:roles_table()
  local status, _, body = self:fetch_api("/_api/auth/roles")
  local roles = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      roles = data.roles or data or {}
    end
  end

  self:render_partial("dashboard/_roles_table", { roles = roles })
end

-- Create role modal
function AdminController:roles_modal_create()
  self:render_partial("dashboard/_modal_create_role", {})
end

-- Create role action
function AdminController:create_role()
  local name = self.params.name
  local permissions = self.params.permissions or {}

  if not name or name == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Role name is required", "type": "error"}}')
    return self:roles_table()
  end

  local status, _, body = self:fetch_api("/_api/auth/roles", {
    method = "POST",
    body = EncodeJson({ name = name, permissions = permissions })
  })

  if status == 200 or status == 201 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Role created successfully", "type": "success"}}')
  else
    local err_msg = "Failed to create role"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:roles_table()
end

-- Delete role action
function AdminController:delete_role()
  local name = self.params.name

  local status, _, body = self:fetch_api("/_api/auth/roles/" .. name, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Role deleted successfully", "type": "success"}}')
    self:html("")
  else
    local err_msg = "Failed to delete role"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
    self:html("")
  end
end

-- API Keys page
function AdminController:apikeys()
  self.layout = "dashboard"
  self:render("dashboard/apikeys", {
    title = "API Keys - SoliDB",
    db = "_system",
    current_page = "apikeys"
  })
end

-- API Keys table
function AdminController:apikeys_table()
  local status, _, body = self:fetch_api("/_api/auth/api-keys")
  local keys = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      keys = data.keys or data or {}
    end
  end

  self:render_partial("dashboard/_apikeys_table", { keys = keys })
end

-- Create API key modal
function AdminController:apikeys_modal_create()
  -- Fetch available roles for the dropdown
  local status, _, body = self:fetch_api("/_api/auth/roles")
  local roles = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      roles = data.roles or data or {}
    end
  end

  -- Fetch databases for scoping
  local db_status, _, db_body = self:fetch_api("/_api/databases")
  local databases = {}

  if db_status == 200 then
    local ok, data = pcall(DecodeJson, db_body)
    if ok and data then
      databases = data.databases or data or {}
    end
  end

  self:render_partial("dashboard/_modal_create_apikey", { roles = roles, databases = databases })
end

-- Create API key action
function AdminController:create_apikey()
  local name = self.params.name
  local roles = self.params.roles or {}
  local scoped_databases = self.params.scoped_databases

  if not name or name == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Key name is required", "type": "error"}}')
    return self:apikeys_table()
  end

  -- Normalize roles to array
  if type(roles) == "string" then
    roles = { roles }
  end

  -- Normalize scoped_databases to array or nil
  if scoped_databases and scoped_databases ~= "" then
    if type(scoped_databases) == "string" then
      scoped_databases = { scoped_databases }
    end
  else
    scoped_databases = nil
  end

  local request_body = { name = name, roles = roles }
  if scoped_databases then
    request_body.scoped_databases = scoped_databases
  end

  local status, _, body = self:fetch_api("/_api/auth/api-keys", {
    method = "POST",
    body = EncodeJson(request_body)
  })

  if status == 200 or status == 201 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data and data.key then
      -- Return a special response showing the key (only shown once!)
      self:render_partial("dashboard/_apikey_created", { apikey = data })
      return
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "API key created successfully", "type": "success"}}')
  else
    local err_msg = "Failed to create API key"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:apikeys_table()
end

-- Delete API key action
function AdminController:delete_apikey()
  local key_id = self.params.key_id

  local status, _, body = self:fetch_api("/_api/auth/api-keys/" .. key_id, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "API key deleted successfully", "type": "success"}}')
    self:html("")
  else
    local err_msg = "Failed to delete API key"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
    self:html("")
  end
end

-- Sharding dashboard
function AdminController:sharding()
  self.layout = "dashboard"
  self:render("dashboard/sharding", {
    title = "Sharding - SoliDB",
    db = "_system",
    current_page = "sharding"
  })
end

-- Sharding distribution (HTMX partial)
function AdminController:sharding_distribution()
  -- Fetch all collections first
  local status, _, body = self:fetch_api("/_api/database/_system/collection")
  local collections = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      collections = data.collections or data or {}
    end
  end

  -- Aggregate node data across all sharded collections
  local node_distribution = {}  -- node_id -> { docs, size, primary_shards, replica_shards }
  local total_shards = 0
  local total_documents = 0
  local total_size = 0
  local replication_factor = 1

  for _, coll in ipairs(collections) do
    if coll.name and coll.name:sub(1, 1) ~= "_" then
      -- Fetch sharding details for each collection
      local shard_status, _, shard_body = self:fetch_api("/_api/database/_system/collection/" .. coll.name .. "/sharding")
      if shard_status == 200 then
        local ok, shard_data = pcall(DecodeJson, shard_body)
        if ok and shard_data and shard_data.sharded then
          -- This collection is sharded
          total_shards = total_shards + (shard_data.config and shard_data.config.num_shards or 0)
          total_documents = total_documents + (shard_data.total_documents or 0)
          total_size = total_size + (shard_data.total_size or 0)

          if shard_data.config and shard_data.config.replication_factor then
            replication_factor = math.max(replication_factor, shard_data.config.replication_factor)
          end

          -- Aggregate node stats
          if shard_data.nodes then
            for _, node in ipairs(shard_data.nodes) do
              local node_id = node.node_id or "unknown"
              if not node_distribution[node_id] then
                node_distribution[node_id] = {
                  node_id = node_id,
                  address = node.address or node_id,
                  status = node.status or "unknown",
                  healthy = node.healthy or false,
                  document_count = 0,
                  disk_size = 0,
                  primary_shards = 0,
                  replica_shards = 0
                }
              end
              local n = node_distribution[node_id]
              n.document_count = n.document_count + (node.document_count or 0)
              n.disk_size = n.disk_size + (node.disk_size or 0)
              n.primary_shards = n.primary_shards + (node.primary_shards or 0)
              n.replica_shards = n.replica_shards + (node.replica_shards or 0)
              -- Update status to worst status seen
              if node.status == "dead" then n.status = "dead"
              elseif node.status == "syncing" and n.status ~= "dead" then n.status = "syncing"
              elseif n.status == "unknown" then n.status = node.status or "unknown"
              end
              n.healthy = n.healthy or node.healthy
            end
          end
        end
      end
    end
  end

  -- Convert to array
  local nodes = {}
  for _, node in pairs(node_distribution) do
    table.insert(nodes, node)
  end

  -- Sort by address
  table.sort(nodes, function(a, b) return (a.address or "") < (b.address or "") end)

  -- Calculate balance status
  local balance_status = "Balanced"
  if #nodes > 0 then
    local min_shards, max_shards = math.huge, 0
    for _, node in ipairs(nodes) do
      local total = node.primary_shards + node.replica_shards
      min_shards = math.min(min_shards, total)
      max_shards = math.max(max_shards, total)
    end
    if max_shards - min_shards > 2 then
      balance_status = "Unbalanced"
    elseif max_shards - min_shards > 0 then
      balance_status = "Slightly Unbalanced"
    end
  end

  self:render_partial("dashboard/_sharding_distribution", {
    nodes = nodes,
    total_shards = total_shards,
    total_documents = total_documents,
    total_size = total_size,
    replication_factor = replication_factor,
    balance_status = balance_status
  })
end

-- Sharding collections (HTMX partial)
function AdminController:sharding_collections()
  -- Fetch all collections
  local status, _, body = self:fetch_api("/_api/database/_system/collection")
  local collections = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      collections = data.collections or data or {}
    end
  end

  local sharded_collections = {}

  for _, coll in ipairs(collections) do
    if coll.name and coll.name:sub(1, 1) ~= "_" then
      -- Fetch sharding details for each collection
      local shard_status, _, shard_body = self:fetch_api("/_api/database/_system/collection/" .. coll.name .. "/sharding")
      if shard_status == 200 then
        local ok, shard_data = pcall(DecodeJson, shard_body)
        if ok and shard_data and shard_data.sharded then
          table.insert(sharded_collections, {
            name = coll.name,
            type = shard_data.type or coll.type,
            num_shards = shard_data.config and shard_data.config.num_shards or 0,
            shard_key = shard_data.config and shard_data.config.shard_key or "_key",
            replication_factor = shard_data.config and shard_data.config.replication_factor or 1,
            total_documents = shard_data.total_documents or 0,
            total_size = shard_data.total_size or 0,
            total_size_formatted = shard_data.total_size_formatted or "0 B",
            cluster = shard_data.cluster or { total_nodes = 1, healthy_nodes = 1 },
            shards = shard_data.shards or {}
          })
        end
      end
    end
  end

  self:render_partial("dashboard/_sharding_collections", {
    collections = sharded_collections
  })
end

-- Sharding configure modal
function AdminController:sharding_modal_configure()
  -- Fetch all collections to allow setting sharding config
  local status, _, body = self:fetch_api("/_api/database/_system/collection")
  local collections = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      collections = data.collections or data or {}
    end
  end

  -- Filter out system collections
  local filtered = {}
  for _, coll in ipairs(collections) do
    if coll.name and coll.name:sub(1, 1) ~= "_" then
      table.insert(filtered, coll)
    end
  end

  self:render_partial("dashboard/_modal_sharding_configure", {
    collections = filtered
  })
end

-- Sharding rebalance action
function AdminController:sharding_rebalance()
  -- Call cluster reshard API
  local status, _, body = self:fetch_api("/_api/cluster/reshard", {
    method = "POST",
    body = EncodeJson({})
  })

  if status == 200 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Shard rebalancing initiated", "type": "success"}}')
  else
    local err_msg = "Failed to initiate rebalancing"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  -- Return refreshed distribution
  self:sharding_distribution()
end

-- Environment variables page
function AdminController:env()
  self.layout = "dashboard"
  self:render("dashboard/env", {
    title = "Environment - " .. self:get_db(),
    db = self:get_db(),
    current_page = "env"
  })
end

-- Env table
function AdminController:env_table()
  local db = self:get_db()
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/env")

  local env_vars = {}
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      env_vars = data
    end
  end

  self:render_partial("dashboard/_env_table", {
    env_vars = env_vars,
    db = db
  })
end

-- Env modal create
function AdminController:env_modal_create()
  self:render_partial("dashboard/_modal_env", {
    db = self:get_db(),
    env_key = "",
    env_value = ""
  })
end

-- Env modal edit
function AdminController:env_modal_edit()
  local db = self:get_db()
  local key = self.params.key

  -- Fetch current value
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/env")
  local value = ""
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data and data[key] then
      value = data[key]
    end
  end

  self:render_partial("dashboard/_modal_env", {
    db = db,
    env_key = key,
    env_value = value
  })
end

-- Env set (PUT)
function AdminController:env_set()
  local db = self:get_db()
  local key = self.params.key
  local value = self.params.value

  if not key or key == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Variable name is required", "type": "error"}}')
    return self:env_table()
  end

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/env/" .. key, {
    method = "PUT",
    body = EncodeJson({ value = value or "" })
  })

  if status == 200 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Environment variable saved", "type": "success"}}')
  else
    local err_msg = "Failed to save variable"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:env_table()
end

-- Env delete (DELETE)
function AdminController:env_delete()
  local db = self:get_db()
  local key = self.params.key

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/env/" .. key, {
    method = "DELETE"
  })

  if status == 200 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Environment variable deleted", "type": "success"}}')
  else
    SetHeader("HX-Trigger", '{"showToast": {"message": "Failed to delete variable", "type": "error"}}')
  end

  self:env_table()
end

-- Queues manager
function AdminController:queues()
  self.layout = "dashboard"
  self:render("dashboard/queues", {
    title = "Queues - " .. self:get_db(),
    db = self:get_db(),
    current_page = "queues"
  })
end

-- Queues Status
function AdminController:queues_status()
  local db = self:get_db()
  local status_filter = self.params.status -- pending, running, completed, failed

  -- 1. List all queues
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/queues")
  if status ~= 200 then
    self:render_partial("dashboard/_empty_state", { message = "Failed to load queues" })
    return
  end

  local ok, queues = pcall(DecodeJson, body)
  if not ok or not queues then
    self:render_partial("dashboard/_empty_state", { message = "No queues found" })
    return
  end

  local all_jobs = {}

  -- 2. For each queue, fetch jobs with status (limit to 10 total for demo)
  for _, queue in ipairs(queues) do
     -- Note: Assuming API supports status filtering. If not, we filter client side.
     -- Using list_jobs_handler: /_api/database/{db}/queues/{name}/jobs
     local q_status, _, q_body = self:fetch_api("/_api/database/" .. db .. "/queues/" .. queue .. "/jobs")
     if q_status == 200 then
       local q_ok, jobs = pcall(DecodeJson, q_body)
       if q_ok and jobs then
         -- Filter by status if API doesn't support it (naive impl)
         for _, job in ipairs(jobs) do
           if job.status == status_filter then
             table.insert(all_jobs, job)
           end
         end
       end
     end
  end

  -- Render rows
  if #all_jobs == 0 then
    self:html('<div class="p-4 text-center text-text-muted">No ' .. status_filter .. ' jobs found</div>')
  else
    -- Simple list rendering for now
    local html = '<div class="space-y-2">'
    for _, job in ipairs(all_jobs) do
      html = html .. '<div class="p-3 bg-bg-card rounded border border-border flex justify-between">'
      html = html .. '<span>' .. (job.id or "Job") .. '</span>'
      html = html .. '<span class="text-xs px-2 py-1 rounded bg-secondary">' .. (job.status or "") .. '</span>'
      html = html .. '</div>'
    end
    html = html .. '</div>'
    self:html(html)
  end
end

return AdminController
