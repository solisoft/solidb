# Users - manage SoliDB users and their role assignments.

class UsersController < Controller
  static {
    this.layout = "application"
  }

  # GET /users
  def index
    @title = "Users"
    this._reset_banners()
    this._load()
  end

  # POST /users
  def create
    username = (params["username"] ?? "").trim()
    password = params["password"] ?? ""
    if username.blank? || password.blank?
      return this._respond({ "ok": false, "status": 422, "error": "username and password are required" }, "")
    end
    payload = { "username": username, "password": password }
    initial_role = (params["initial_role"] ?? "").trim()
    payload["initial_role"] = initial_role unless initial_role.blank?
    result = SolidbClient.post_api(SolidbEndpoints.users(), payload)
    return this._respond(result, "user " + username + " created")
  end

  # DELETE /users/:username
  def delete
    username = params["username"] ?? ""
    result = SolidbClient.delete_api(SolidbEndpoints.user(username))
    return this._respond(result, "user " + username + " deleted")
  end

  # POST /users/:username/roles
  def add_role
    username = params["username"] ?? ""
    role = (params["role"] ?? "").trim()
    if role.blank?
      return this._respond({ "ok": false, "status": 422, "error": "role is required" }, "")
    end
    payload = { "role": role }
    database = (params["database"] ?? "").trim()
    payload["database"] = database unless database.blank?
    result = SolidbClient.post_api(SolidbEndpoints.user_roles(username), payload)
    return this._respond(result, "role " + role + " granted to " + username)
  end

  # DELETE /users/:username/roles/:role
  def remove_role
    username = params["username"] ?? ""
    role = params["role"] ?? ""
    result = SolidbClient.delete_api(SolidbEndpoints.user_role(username, role))
    return this._respond(result, "role " + role + " revoked from " + username)
  end

  def _load
    users_result = SolidbClient.get_api(SolidbEndpoints.users())
    @users = (users_result["data"] ?? {})["users"] ?? []
    roles_result = SolidbClient.get_api(SolidbEndpoints.roles())
    @role_names = (roles_result["data"] ?? []).map do |role| role["name"] end
    if !users_result["ok"]
      @flash_error = users_result["error"] ?? "request failed"
      @solidb_down = (users_result["status"] ?? -1) == 0
    end
  end

  def _reset_banners
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
  end

  def _respond(result, notice)
    @title = "Users"
    this._reset_banners()
    if result["ok"]
      @flash_notice = notice
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    this._load()
    return render("users/index", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("users/index")
  end
end
