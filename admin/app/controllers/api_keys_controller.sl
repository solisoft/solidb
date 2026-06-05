# API keys - create / list / revoke server API keys. The raw key is only
# returned by SoliDB at creation time, so it is surfaced once in a banner.

class ApiKeysController < Controller
  static {
    this.layout = "application"
  }

  # GET /api-keys
  def index
    @title = "API Keys"
    this._reset_banners()
    this._load()
  end

  # POST /api-keys
  def create
    name = (params["name"] ?? "").trim()
    if name.blank?
      return this._respond({ "ok": false, "status": 422, "error": "key name is required" }, "")
    end
    payload = { "name": name }
    roles = this._split_csv(params["roles"] ?? "")
    payload["roles"] = roles if roles.length() > 0
    scoped = this._split_csv(params["scoped_databases"] ?? "")
    payload["scoped_databases"] = scoped if scoped.length() > 0
    result = SolidbClient.post_api(SolidbEndpoints.api_keys(), payload)
    raw_key = (result["data"] ?? {})["key"] ?? ""
    return this._respond(result, "api key " + name + " created", raw_key)
  end

  # DELETE /api-keys/:id
  def delete
    key_id = params["id"] ?? ""
    result = SolidbClient.delete_api(SolidbEndpoints.api_key(key_id))
    return this._respond(result, "api key revoked")
  end

  def _split_csv(text)
    return [] if text.trim().blank?
    parts = text.split(",").map do |part| part.trim() end
    return parts.filter do |part| !part.blank? end
  end

  def _load
    # Header db picker needs the database list on every render.
    @databases = AdminContext.database_names()
    result = SolidbClient.get_api(SolidbEndpoints.api_keys())
    @keys = (result["data"] ?? {})["keys"] ?? []
    if !result["ok"]
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
  end

  def _reset_banners
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
    @created_key = ""
  end

  def _respond(result, notice, created_key = "")
    @title = "API Keys"
    this._reset_banners()
    @created_key = created_key if result["ok"]
    if result["ok"]
      @flash_notice = notice
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    this._load()
    return render("api_keys/index", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("api_keys/index")
  end
end
