-- Dashboard Monitoring Controller
-- Handles system monitoring and stats
local DashboardBaseController = require("dashboard.base_controller")
local MonitoringController = DashboardBaseController:extend()

-- Helper to format bytes
local function format_bytes(bytes)
  if not bytes or bytes == 0 then return "0 B" end
  if bytes >= 1073741824 then
    return string.format("%.2f GB", bytes / 1073741824)
  elseif bytes >= 1048576 then
    return string.format("%.2f MB", bytes / 1048576)
  elseif bytes >= 1024 then
    return string.format("%.2f KB", bytes / 1024)
  else
    return bytes .. " B"
  end
end

-- Helper to format uptime
local function format_uptime(secs)
  if not secs or secs == 0 then return "-" end
  if secs < 60 then return secs .. "s" end
  if secs < 3600 then return math.floor(secs / 60) .. "m" end
  if secs < 86400 then
    return math.floor(secs / 3600) .. "h " .. math.floor((secs % 3600) / 60) .. "m"
  end
  return math.floor(secs / 86400) .. "d " .. math.floor((secs % 86400) / 3600) .. "h"
end

-- Helper to format large numbers
local function format_number(num)
  if not num then return "-" end
  if num >= 1000000 then
    return string.format("%.1fM", num / 1000000)
  elseif num >= 1000 then
    return string.format("%.1fK", num / 1000)
  end
  return tostring(num)
end

-- System monitoring page
function MonitoringController:index()
  self.layout = "dashboard"
  self:render("dashboard/monitoring", {
    title = "System Monitor - SoliDB",
    db = "_system",
    current_page = "monitoring"
  })
end

-- Fetch cluster status from API
function MonitoringController:get_cluster_status()
  local status, _, body = self:fetch_api("/_api/cluster/status")
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      return data
    end
  end
  return nil
end

-- Monitoring Metrics partial (CPU, Memory, Disk, Requests)
function MonitoringController:metrics()
  local cluster_status = self:get_cluster_status()
  local stats = cluster_status and cluster_status.stats or {}

  self:render_partial("dashboard/_monitoring_metrics", {
    cpu = stats.cpu_usage_percent or 0,
    memory_used = stats.memory_used_mb or 0,
    memory_total = stats.memory_total_mb or 0,
    storage_bytes = stats.storage_bytes or 0,
    request_count = stats.request_count or 0,
    format_bytes = format_bytes,
    format_number = format_number
  })
end

-- Extended Stats partial
function MonitoringController:extended_stats()
  local cluster_status = self:get_cluster_status()
  local stats = cluster_status and cluster_status.stats or {}

  self:render_partial("dashboard/_monitoring_extended_stats", {
    database_count = stats.database_count or 0,
    collection_count = stats.collection_count or 0,
    document_count = stats.document_count or 0,
    uptime_secs = stats.uptime_secs or 0,
    format_number = format_number,
    format_uptime = format_uptime
  })
end

-- Storage Stats partial
function MonitoringController:storage_stats()
  local cluster_status = self:get_cluster_status()
  local stats = cluster_status and cluster_status.stats or {}

  self:render_partial("dashboard/_monitoring_storage_stats", {
    total_sst_size = stats.total_sst_size or 0,
    total_memtable_size = stats.total_memtable_size or 0,
    total_live_size = stats.total_live_size or 0,
    total_file_count = stats.total_file_count or 0,
    total_chunk_count = stats.total_chunk_count or 0,
    network_rx_bytes = stats.network_rx_bytes or 0,
    network_tx_bytes = stats.network_tx_bytes or 0,
    system_load_avg = stats.system_load_avg or 0,
    format_bytes = format_bytes,
    format_number = format_number
  })
end

-- Monitoring Operations partial
function MonitoringController:operations()
  local cluster_status = self:get_cluster_status()
  local stats = cluster_status and cluster_status.stats or {}

  self:render_partial("dashboard/_monitoring_operations", {
    request_count = stats.request_count or 0,
    format_number = format_number
  })
end

-- Slow Queries page
function MonitoringController:slow_queries_page()
  self.layout = "dashboard"
  self:render("dashboard/slow_queries", {
    title = "Slow Queries - " .. self:get_db(),
    db = self:get_db(),
    current_page = "slow_queries"
  })
end

-- Monitoring Slow Queries partial (for HTMX)
function MonitoringController:slow_queries()
  local db = self:get_db()
  local limit = tonumber(self.params.limit) or 50

  -- Query the _slow_queries collection
  local query = string.format([[
    FOR sq IN _slow_queries
      SORT sq.timestamp DESC
      LIMIT %d
      RETURN sq
  ]], limit)

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/cursor", {
    method = "POST",
    body = EncodeJson({ query = query })
  })

  local slow_queries = {}
  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data and data.result then
      slow_queries = data.result
    end
  end

  self:render_partial("dashboard/_slow_queries_table", {
    slow_queries = slow_queries,
    db = db,
    format_time = function(ms)
      if not ms then return "-" end
      if ms < 1 then
        return string.format("%.0fµs", ms * 1000)
      elseif ms < 1000 then
        return string.format("%.2fms", ms)
      else
        return string.format("%.2fs", ms / 1000)
      end
    end
  })
end

-- Clear slow queries
function MonitoringController:clear_slow_queries()
  local db = self:get_db()

  -- Truncate the _slow_queries collection
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collection/_slow_queries/truncate", {
    method = "PUT"
  })

  if status == 200 then
    SetHeader("HX-Trigger", '{"showToast": {"message": "Slow queries cleared", "type": "success"}}')
  else
    SetHeader("HX-Trigger", '{"showToast": {"message": "Failed to clear slow queries", "type": "error"}}')
  end

  -- Return empty table
  self:render_partial("dashboard/_slow_queries_table", {
    slow_queries = {},
    db = db,
    format_time = function(ms) return "-" end
  })
end

-- Stats: Collections Count
function MonitoringController:stats_collections()
  local db = self:get_db()

  local status, headers, body = self:fetch_api("/_api/database/" .. db .. "/collection")
  if status == 200 then
    local ok, collections = pcall(DecodeJson, body)
    if ok and collections then
      self:html(tostring(#collections))
    else
      self:html("-")
    end
  else
    self:html("Err")
  end
end

-- Stats: Documents Count (Approximation or Sum)
function MonitoringController:stats_documents()
  local cluster_status = self:get_cluster_status()
  if cluster_status and cluster_status.stats then
    self:html(format_number(cluster_status.stats.document_count or 0))
  else
    self:html("-")
  end
end

-- Stats: Indexes Count
function MonitoringController:stats_indexes()
  self:html("-")
end

-- Stats: DB Size
function MonitoringController:stats_size()
  local cluster_status = self:get_cluster_status()
  if cluster_status and cluster_status.stats then
    self:html(format_bytes(cluster_status.stats.storage_bytes or 0))
  else
    self:html("-")
  end
end

return MonitoringController
