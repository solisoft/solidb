# Home controller - server dashboard + app health endpoint

class HomeController < Controller
  static {
    this.layout = "application"
  }

  # GET / - server health, cluster info, database count
  def index
    @title = "Dashboard"
    health_result = SolidbClient.get_api(SolidbEndpoints.health())
    @solidb_down = health_result["status"] == 0
    @server_ok = health_result["ok"]
    info_result = SolidbClient.get_api(SolidbEndpoints.cluster_info())
    @cluster = info_result["data"] ?? {}
    @database_names = AdminContext.database_names()
    @databases = @database_names
    @flash_error = ""
    @flash_notice = ""
  end

  # GET /health - app-level health check (used by the proxy)
  def health
    render_json({ "status": "ok" })
  end
end
