# Lua scripts - CRUD for the database's custom Lua endpoints.

class ScriptsController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases/:db/scripts
  def index
    this._ctx()
    @title = "Scripts · " + @db
    this._reset_banners()
    this._load()
  end

  # GET /databases/:db/scripts/new
  def new
    this._ctx()
    @title = "New script · " + @db
    this._reset_banners()
    @script = {}
  end

  # POST /databases/:db/scripts
  def create
    this._ctx()
    payload = this._build_payload()
    script_name = payload["name"] ?? ""
    script_path = payload["path"] ?? ""
    if script_name.blank? || script_path.blank?
      return this._respond({ "ok": false, "status": 422, "error": "name and path are required" }, "")
    end
    result = SolidbClient.post_api(SolidbEndpoints.scripts(@db), payload)
    return this._respond(result, "script " + payload["name"] + " created")
  end

  # GET /databases/:db/scripts/:id
  def show
    this._ctx()
    this._reset_banners()
    this._load_script()
    @title = "Script · " + (@script["name"] ?? "")
  end

  # GET /databases/:db/scripts/:id/edit
  def edit
    this._ctx()
    this._reset_banners()
    this._load_script()
    @title = "Edit script · " + (@script["name"] ?? "")
  end

  # PUT /databases/:db/scripts/:id
  def update
    this._ctx()
    payload = this._build_payload()
    result = SolidbClient.put_api(SolidbEndpoints.script(@db, params["id"] ?? ""), payload)
    return this._respond(result, "script " + (payload["name"] ?? "") + " updated")
  end

  # DELETE /databases/:db/scripts/:id
  def delete
    this._ctx()
    result = SolidbClient.delete_api(SolidbEndpoints.script(@db, params["id"] ?? ""))
    return this._respond(result, "script deleted")
  end

  def _ctx
    @db = params["db"] ?? ""
    @databases = AdminContext.database_names()
  end

  def _build_payload
    methods = []
    methods.push("GET") if params["method_get"] == "on"
    methods.push("POST") if params["method_post"] == "on"
    methods.push("PUT") if params["method_put"] == "on"
    methods.push("DELETE") if params["method_delete"] == "on"
    methods = ["GET"] if methods.length() == 0
    return {
      "name": (params["name"] ?? "").trim(),
      "path": (params["path"] ?? "").trim(),
      "methods": methods,
      "code": params["code"] ?? "",
      "description": params["description"] ?? "",
      "service": (params["service"] ?? "").trim()
    }
  end

  def _load
    result = SolidbClient.get_api(SolidbEndpoints.scripts(@db))
    @scripts = (result["data"] ?? {})["scripts"] ?? []
    if !result["ok"]
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
  end

  def _load_script
    result = SolidbClient.get_api(SolidbEndpoints.script(@db, params["id"] ?? ""))
    @script = result["data"] ?? {}
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
    @title = "Scripts · " + @db
    this._reset_banners()
    if result["ok"]
      @flash_notice = notice
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    this._load()
    return render("scripts/index", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("scripts/index")
  end
end
