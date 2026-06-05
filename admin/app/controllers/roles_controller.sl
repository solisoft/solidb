# Roles - RBAC role management (builtin roles are read-only).

class RolesController < Controller
  static {
    this.layout = "application"
  }

  # GET /roles
  def index
    @title = "Roles"
    this._reset_banners()
    this._load()
  end

  # GET /roles/:name
  def show
    @title = "Role · " + (params["name"] ?? "")
    this._reset_banners()
    result = SolidbClient.get_api(SolidbEndpoints.role(params["name"] ?? ""))
    @role = result["data"] ?? {}
    if !result["ok"]
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
  end

  # POST /roles
  def create
    name = (params["name"] ?? "").trim()
    payload = this._build_payload(name)
    if payload.nil?
      return this._respond({ "ok": false, "status": 422, "error": "permissions must be a JSON array" }, "")
    end
    if name.blank?
      return this._respond({ "ok": false, "status": 422, "error": "role name is required" }, "")
    end
    result = SolidbClient.post_api(SolidbEndpoints.roles(), payload)
    return this._respond(result, "role " + name + " created")
  end

  # PUT /roles/:name
  def update
    name = params["name"] ?? ""
    payload = this._build_payload(name)
    if payload.nil?
      return this._respond({ "ok": false, "status": 422, "error": "permissions must be a JSON array" }, "")
    end
    result = SolidbClient.put_api(SolidbEndpoints.role(name), payload)
    return this._respond(result, "role " + name + " updated")
  end

  # DELETE /roles/:name
  def delete
    name = params["name"] ?? ""
    result = SolidbClient.delete_api(SolidbEndpoints.role(name))
    return this._respond(result, "role " + name + " deleted")
  end

  # nil when the permissions textarea holds invalid JSON.
  def _build_payload(name)
    permissions = []
    permissions_text = (params["permissions"] ?? "").trim()
    if !permissions_text.blank?
      permissions = JSON.parse(permissions_text) rescue nil
      return nil if permissions.nil?
    end
    return {
      "name": name,
      "description": params["description"] ?? "",
      "permissions": permissions
    }
  end

  def _load
    result = SolidbClient.get_api(SolidbEndpoints.roles())
    @roles = result["data"] ?? []
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
    @title = "Roles"
    this._reset_banners()
    if result["ok"]
      @flash_notice = notice
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    this._load()
    return render("roles/index", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("roles/index")
  end
end
