# Cluster monitoring - node health, peers, replication sync log.
# The page polls /cluster/panel (HTMX, every 3s) so the numbers stay live
# without a manual refresh.

class ClusterController < Controller
  static {
    this.layout = "application"
  }

  # GET /cluster
  def show
    this._ctx()
    @title = "Cluster"
    this._reset_banners()
    this._load()
  end

  # GET /cluster/panel - polled fragment (HTMX every 3s)
  def panel
    this._ctx()
    this._reset_banners()
    this._load()
    return render("cluster/panel", { "layout": false })
  end

  # POST /cluster/sync-log/prune - prune everything older than the current
  # sequence (same boundary the auto-pruner uses).
  def prune_sync_log
    this._ctx()
    @title = "Cluster"
    this._reset_banners()
    stats_result = SolidbClient.get_api(SolidbEndpoints.sync_log_stats())
    current_sequence = (stats_result["data"] ?? {})["current_sequence"] ?? 0
    result = SolidbClient.post_api(SolidbEndpoints.sync_log_prune(),
                                   { "before_sequence": current_sequence })
    if result["ok"]
      removed = (result["data"] ?? {})["removed"] ?? 0
      @flash_notice = "sync log pruned (" + str(removed) + " entries removed)"
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    this._load()
    return render("cluster/show", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("cluster/show")
  end

  def _ctx
    @db = ""
    @databases = AdminContext.database_names()
  end

  def _load
    result = SolidbClient.get_api(SolidbEndpoints.cluster_status())
    @cluster = result["data"] ?? {}
    @node_stats = @cluster["stats"] ?? {}
    @peers = @cluster["peers"] ?? []
    if !result["ok"]
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    sync_result = SolidbClient.get_api(SolidbEndpoints.sync_log_stats())
    @sync_log = sync_result["data"] ?? {}
  end

  def _reset_banners
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
  end
end
