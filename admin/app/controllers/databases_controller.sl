# Databases - list / create / drop databases on the SoliDB server.

class DatabasesController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases
  def index
    @title = "Databases"
    this._reset_banners()
    this._load()
  end

  # POST /databases
  def create
    name = (params["name"] ?? "").trim()
    if name.blank?
      return this._respond({ "ok": false, "status": 422, "error": "database name is required" }, "")
    end
    result = SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": name })
    return this._respond(result, "database " + name + " created")
  end

  # DELETE /databases/:db
  # Requires confirm_name to match the database name (typed in the confirm modal).
  def delete
    db_name = params["db"] ?? ""
    confirm = (params["confirm_name"] ?? "").trim()
    if confirm.blank? || confirm != db_name
      return this._respond({
        "ok": false,
        "status": 422,
        "error": "type the database name '" + db_name + "' to confirm drop"
      }, "")
    end
    result = SolidbClient.delete_api(SolidbEndpoints.database(db_name))
    return this._respond(result, "database " + db_name + " dropped")
  end

  def _load
    @database_names = AdminContext.database_names()
    @databases = @database_names
  end

  # Reading an unset @field raises in Soli, so banner fields are always
  # assigned before the view renders.
  def _reset_banners
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
  end

  # Shared tail for mutations: set banners, reload the list, re-render the
  # section (fragment for HTMX swaps, full page otherwise).
  def _respond(result, notice)
    @title = "Databases"
    this._reset_banners()
    if result["ok"]
      @flash_notice = notice
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    this._load()
    return render("databases/index", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("databases/index")
  end
end
