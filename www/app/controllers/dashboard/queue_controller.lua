-- Dashboard Queue Controller
-- Handles job queues and cron jobs management
local DashboardBaseController = require("dashboard.base_controller")
local QueueController = DashboardBaseController:extend()

-- Main queues page
function QueueController:index()
  self.layout = "dashboard"
  self:render("dashboard/queues", {
    title = "Queues - " .. self:get_db(),
    db = self:get_db(),
    current_page = "queues"
  })
end

-- Queue stats (HTMX partial)
function QueueController:stats()
  local db = self:get_db()
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/queues")

  local stats = {
    pending = 0,
    running = 0,
    completed = 0,
    failed = 0,
    queues = {}
  }

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      -- API returns array of QueueStats objects directly
      local queue_list = data
      
      -- Handle case where it might be wrapped (Defensive coding based on original code, 
      -- though implementation shows it returns Vec<QueueStats>)
      if type(data) == "table" and data.queues then
        queue_list = data.queues
      end

      if type(queue_list) == "table" then
        for _, q in ipairs(queue_list) do
          -- q is a QueueStats object: { name, pending, running, completed, failed, total }
          if type(q) == "table" then
            table.insert(stats.queues, q)
            stats.pending = stats.pending + (q.pending or 0)
            stats.running = stats.running + (q.running or 0)
            stats.completed = stats.completed + (q.completed or 0)
            stats.failed = stats.failed + (q.failed or 0)
          end
        end
      end
    end
  end

  self:render_partial("dashboard/_queues_stats", { stats = stats })
end

-- Jobs list by status (HTMX partial)
function QueueController:jobs()
  local db = self:get_db()
  local status_filter = self.params.status or GetParam("status")  -- nil means "all"

  Log(kLogInfo, "Fetching jobs for status: " .. tostring(status_filter or "all"))

  -- Fetch all queues first to know which ones to query
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/queues")
  local all_jobs = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      local queue_list = data
      if type(data) == "table" and data.queues then
        queue_list = data.queues
      end

      if type(queue_list) == "table" then
        for _, q in ipairs(queue_list) do
          -- q could be a string (if simple list) or object (QueueStats)
          local queue_name = q
          if type(q) == "table" and q.name then
            queue_name = q.name
          end

          if type(queue_name) == "string" then
            Log(kLogInfo, "Fetching jobs for queue: " .. queue_name)
            local api_path = "/_api/database/" .. db .. "/queues/" .. queue_name .. "/jobs"
            if status_filter and status_filter ~= "" then
              api_path = api_path .. "?status=" .. status_filter
            end
            local q_status, _, q_body = self:fetch_api(api_path)
            
            if q_status == 200 then
              local q_ok, jobs_data = pcall(DecodeJson, q_body)
              if q_ok and jobs_data then
                local jobs = jobs_data
                -- API returns { jobs: [...], total: N }
                if type(jobs_data) == "table" and jobs_data.jobs then
                  jobs = jobs_data.jobs
                end

                if type(jobs) == "table" then
                  Log(kLogInfo, "Found " .. #jobs .. " jobs for queue " .. queue_name)
                  for _, job in ipairs(jobs) do
                    -- Ensure queue name is attached if missing
                    if not job.queue then job.queue = queue_name end
                    table.insert(all_jobs, job)
                  end
                end
              end
            else
              Log(kLogWarn, "Failed to fetch jobs for queue " .. queue_name .. ": " .. tostring(q_status))
            end
          end
        end
      end
    else
        Log(kLogError, "Failed to decode queues list")
    end
  else
    Log(kLogError, "Failed to list queues: " .. tostring(status))
  end

  Log(kLogInfo, "Total jobs found: " .. #all_jobs)

  -- Sort by created_at descending
  table.sort(all_jobs, function(a, b)
    return (a.created_at or 0) > (b.created_at or 0)
  end)

  self:render_partial("dashboard/_queues_jobs", {
    jobs = all_jobs,
    status_filter = status_filter or "all",
    db = db
  })
end

-- Cron jobs list (HTMX partial)
function QueueController:cron()
  local db = self:get_db()
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/cron")

  local cron_jobs = {}
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      cron_jobs = data.cron_jobs or data or {}
    end
  end

  self:render_partial("dashboard/_queues_cron", {
    cron_jobs = cron_jobs,
    db = db
  })
end

-- Create job modal
function QueueController:modal_create_job()
  local db = self:get_db()

  -- Fetch available queues
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/queues")
  local queues = {}
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      -- Handle both array response and object with queues field
      if type(data) == "table" and data.queues then
        queues = data.queues
      else
        queues = data
      end
    end
  end

  -- Fetch available scripts
  local scripts = {}
  local s_status, _, s_body = self:fetch_api("/_api/database/" .. db .. "/scripts")
  if s_status == 200 then
    local ok, data = pcall(DecodeJson, s_body)
    if ok and data then
      scripts = data.scripts or data or {}
    end
  end

  self:render_partial("dashboard/_modal_create_job", {
    db = db,
    queues = queues,
    scripts = scripts
  })
end

-- Create job action
function QueueController:create_job()
  local db = self:get_db()
  local queue = self.params.queue or "default"
  local script = self.params.script
  local params_str = self.params.params or "{}"
  local priority = tonumber(self.params.priority) or 0
  local max_retries = tonumber(self.params.max_retries) or 20
  local run_at = self.params.run_at

  if not script or script == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Script path is required", "type": "error"}}')
    return self:jobs()
  end

  -- Parse params JSON
  local params = {}
  if params_str and params_str ~= "" then
    local ok, parsed = pcall(DecodeJson, params_str)
    if ok then
      params = parsed
    end
  end

  local request_body = {
    script = script,
    params = params,
    priority = priority,
    max_retries = max_retries
  }

  -- Convert run_at datetime to unix timestamp if provided
  if run_at and run_at ~= "" then
    -- Expecting format: YYYY-MM-DDTHH:MM (from datetime-local input)
    -- Convert to unix timestamp (this is simplified, may need timezone handling)
    local pattern = "(%d+)-(%d+)-(%d+)T(%d+):(%d+)"
    local year, month, day, hour, min = run_at:match(pattern)
    if year then
      local timestamp = os.time({
        year = tonumber(year),
        month = tonumber(month),
        day = tonumber(day),
        hour = tonumber(hour),
        min = tonumber(min),
        sec = 0
      })
      request_body.run_at = timestamp
    end
  end

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/queues/" .. queue .. "/enqueue", {
    method = "POST",
    body = EncodeJson(request_body)
  })

  if status == 200 or status == 201 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Job created successfully", "type": "success"}, "closeModal": true, "refreshStats": true}')
  else
    local err_msg = "Failed to create job"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:jobs()
end

-- Cancel job action
function QueueController:cancel_job()
  local db = self:get_db()
  local job_id = self.params.job_id

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/queues/jobs/" .. job_id, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Job cancelled successfully", "type": "success"}, "refreshStats": true}')
    self:html("")
  else
    local err_msg = "Failed to cancel job"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
    self:html("")
  end
end

-- Create cron job modal
function QueueController:modal_create_cron()
  local db = self:get_db()

  -- Fetch available queues
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/queues")
  local queues = {}
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      queues = data
    end
  end

  -- Fetch available scripts
  local scripts = {}
  local s_status, _, s_body = self:fetch_api("/_api/database/" .. db .. "/scripts")
  if s_status == 200 then
    local ok, data = pcall(DecodeJson, s_body)
    if ok and data then
      scripts = data.scripts or data or {}
    end
  end

  self:render_partial("dashboard/_modal_create_cron", {
    db = db,
    queues = queues,
    scripts = scripts
  })
end

-- Create cron job action
function QueueController:create_cron()
  local db = self:get_db()
  local name = self.params.name
  local cron_expression = self.params.cron_expression
  local script = self.params.script
  local queue = self.params.queue or "default"
  local params_str = self.params.params or "{}"
  local priority = tonumber(self.params.priority) or 0
  local max_retries = tonumber(self.params.max_retries) or 3

  if not name or name == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Cron job name is required", "type": "error"}}')
    return self:cron()
  end

  if not cron_expression or cron_expression == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Cron expression is required", "type": "error"}}')
    return self:cron()
  end

  if not script or script == "" then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Script path is required", "type": "error"}}')
    return self:cron()
  end

  -- Parse params JSON
  local params = {}
  if params_str and params_str ~= "" then
    local ok, parsed = pcall(DecodeJson, params_str)
    if ok then
      params = parsed
    end
  end

  local request_body = {
    name = name,
    cron_expression = cron_expression,
    script = script,
    queue = queue,
    params = params,
    priority = priority,
    max_retries = max_retries
  }

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/cron", {
    method = "POST",
    body = EncodeJson(request_body)
  })

  if status == 200 or status == 201 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Cron job created successfully", "type": "success"}, "closeModal": true}')
  else
    local err_msg = "Failed to create cron job"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
  end

  self:cron()
end

-- Delete cron job action
function QueueController:delete_cron()
  local db = self:get_db()
  local cron_id = self.params.cron_id

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/cron/" .. cron_id, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Cron job deleted successfully", "type": "success"}}')
    self:html("")
  else
    local err_msg = "Failed to delete cron job"
    local ok, err_data = pcall(DecodeJson, body)
    if ok and err_data and err_data.error then
      err_msg = err_data.error
    end
    SetHeader("HX-Trigger", '{"showToast": {"message": "' .. err_msg:gsub('"', '\\"') .. '", "type": "error"}}')
    self:html("")
  end
end

return QueueController
