-- Dashboard Cluster Controller
-- Handles cluster management and monitoring
local DashboardBaseController = require("dashboard.base_controller")
local ClusterController = DashboardBaseController:extend()

-- Cluster page
function ClusterController:index()
  self.layout = "dashboard"
  self:render("dashboard/cluster", {
    title = "Cluster - SoliDB",
    db = "_system",
    current_page = "cluster"
  })
end

-- Cluster stats partial
function ClusterController:stats()

  -- Fetch cluster status
  local status, _, body = self:fetch_api("/_api/cluster/status")
  local cluster_data = {}

  if status == 200 then
    local ok, res = pcall(DecodeJson, body)
    if ok and res then
      cluster_data = res
    end
  end

  -- Fetch cluster info for additional details
  local info_status, _, info_body = self:fetch_api("/_api/cluster/info")
  local cluster_info = {}
  if info_status == 200 then
    local ok, res = pcall(DecodeJson, info_body)
    if ok and res then
      cluster_info = res
    end
  end

  -- Count active nodes
  local nodes = cluster_data.nodes or {}
  local active_count = 0
  for _, node in ipairs(nodes) do
    if node.status == "online" or node.status == "active" or node.healthy then
      active_count = active_count + 1
    end
  end

  self:render_partial("dashboard/_cluster_stats", {
    active_nodes = active_count,
    total_nodes = #nodes,
    cluster_id = cluster_info.cluster_id or cluster_data.cluster_id or "-",
    is_leader = cluster_info.is_leader or cluster_data.is_leader or false,
    sync_status = cluster_data.sync_status or "Unknown",
    started_at = cluster_info.started_at or cluster_data.started_at
  })
end

-- Cluster nodes partial
function ClusterController:nodes()

  -- Fetch cluster status
  local status, _, body = self:fetch_api("/_api/cluster/status")
  local nodes = {}

  if status == 200 then
    local ok, res = pcall(DecodeJson, body)
    if ok and res then
      nodes = res.nodes or res or {}
    end
  end

  self:render_partial("dashboard/_cluster_nodes", {
    nodes = nodes
  })
end

-- Replication log partial
function ClusterController:replication_log()

  -- Fetch cluster info for replication details
  local status, _, body = self:fetch_api("/_api/cluster/info")
  local replication = {}

  if status == 200 then
    local ok, res = pcall(DecodeJson, body)
    if ok and res then
      replication = res.replication or res
    end
  end

  self:render_partial("dashboard/_cluster_replication", {
    replication = replication
  })
end

return ClusterController
