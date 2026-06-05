# Env vars - per-database key/value settings exposed to Lua scripts.
# SoliDB's PUT /env/{key} is an upsert, so create and edit share one action.

class EnvVarsController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases/:db/env
  def index
    this._ctx()
    @title = "Env vars · " + @db
    this._reset_banners()
    this._load()
  end

  # POST /databases/:db/env
  def upsert
    this._ctx()
    env_key = (params["env_key"] ?? "").trim()
    env_value = params["env_value"] ?? ""
    if env_key.blank?
      return this._respond({ "ok": false, "status": 422, "error": "key is required" }, "")
    end
    result = SolidbClient.put_api(SolidbEndpoints.env_var(@db, env_key), { "value": env_value })
    return this._respond(result, "env var " + env_key + " saved")
  end

  # DELETE /databases/:db/env/:key
  def delete
    this._ctx()
    result = SolidbClient.delete_api(SolidbEndpoints.env_var(@db, params["key"] ?? ""))
    return this._respond(result, "env var deleted")
  end

  def _ctx
    @db = params["db"] ?? ""
    @databases = AdminContext.database_names()
  end

  # SoliDB returns a flat { key: value } hash -- flatten to a sorted array
  # of rows so the view stays a dumb loop.
  def _load
    result = SolidbClient.get_api(SolidbEndpoints.env_vars(@db))
    env_map = result["data"] ?? {}
    @env_vars = env_map.keys().sort().map do |env_key|
      { "key": env_key, "value": env_map[env_key] ?? "" }
    end
    if !result["ok"]
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
  end

  def _reset_banners
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
  end

  def _respond(result, notice)
    @title = "Env vars · " + @db
    this._reset_banners()
    if result["ok"]
      @flash_notice = notice
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    this._load()
    return render("env_vars/index", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("env_vars/index")
  end
end
