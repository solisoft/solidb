# Triggers - fire a script or webhook when documents change in a collection.

class TriggersController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases/:db/triggers
  def index
    this._ctx()
    @title = "Triggers · " + @db
    this._reset_banners()
    this._load()
  end

  # POST /databases/:db/triggers
  def create
    this._ctx()
    payload = this._build_payload()
    trigger_name = payload["name"] ?? ""
    events = payload["events"] ?? []
    script_target = payload["script_path"] ?? ""
    webhook_target = payload["webhook_url"] ?? ""
    has_target = script_target != "" || webhook_target != ""
    if trigger_name.blank? || (payload["collection"] ?? "") == "" || events.length() == 0 || !has_target
      validation_error = "name, collection, at least one event and a script or webhook target are required"
      return this._respond({ "ok": false, "status": 422, "error": validation_error }, "")
    end
    result = SolidbClient.post_api(SolidbEndpoints.triggers(@db), payload)
    return this._respond(result, "trigger " + trigger_name + " created")
  end

  # POST /databases/:db/triggers/:id/toggle
  def toggle
    this._ctx()
    result = SolidbClient.post_api(SolidbEndpoints.trigger_toggle(@db, params["id"] ?? ""))
    now_enabled = (result["data"] ?? {})["enabled"] ?? false
    return this._respond(result, now_enabled ? "trigger enabled" : "trigger disabled")
  end

  # DELETE /databases/:db/triggers/:id
  def delete
    this._ctx()
    result = SolidbClient.delete_api(SolidbEndpoints.trigger(@db, params["id"] ?? ""))
    return this._respond(result, "trigger deleted")
  end

  def _ctx
    @db = params["db"] ?? ""
    @databases = AdminContext.database_names()
  end

  def _build_payload
    # Checkboxes only submit when checked -- absent means off.
    insert_flag = params["event_insert"] ?? ""
    update_flag = params["event_update"] ?? ""
    delete_flag = params["event_delete"] ?? ""
    events = []
    events.push("insert") if insert_flag != ""
    events.push("update") if update_flag != ""
    events.push("delete") if delete_flag != ""
    payload = {
      "name": (params["trigger_name"] ?? "").trim(),
      "collection": (params["collection"] ?? "").trim(),
      "events": events
    }
    script_path = (params["script_path"] ?? "").trim()
    payload["script_path"] = script_path unless script_path.blank?
    webhook_url = (params["webhook_url"] ?? "").trim()
    payload["webhook_url"] = webhook_url unless webhook_url.blank?
    webhook_secret = (params["webhook_secret"] ?? "").trim()
    payload["webhook_secret"] = webhook_secret unless webhook_secret.blank?
    filter_expr = (params["filter"] ?? "").trim()
    payload["filter"] = filter_expr unless filter_expr.blank?
    queue = (params["queue"] ?? "").trim()
    payload["queue"] = queue unless queue.blank?
    priority = (params["priority"] ?? "").trim()
    payload["priority"] = priority.to_int() unless priority.blank?
    max_retries = (params["max_retries"] ?? "").trim()
    payload["max_retries"] = max_retries.to_int() unless max_retries.blank?
    return payload
  end

  def _load
    result = SolidbClient.get_api(SolidbEndpoints.triggers(@db))
    @triggers = (result["data"] ?? {})["triggers"] ?? []
    if !result["ok"]
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    # Collection names feed the target-collection dropdown in the create
    # modal. Internal collections (_triggers, _env, _columnar_*...) are not
    # valid trigger targets -- hide them.
    collections_result = SolidbClient.get_api(SolidbEndpoints.collections(@db))
    all_collections = (collections_result["data"] ?? {})["collections"] ?? []
    visible = all_collections.filter do |coll| !(coll["name"] ?? "").starts_with("_") end
    @collection_names = visible.map do |coll| coll["name"] ?? "" end
  end

  def _reset_banners
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
  end

  def _respond(result, notice)
    @title = "Triggers · " + @db
    this._reset_banners()
    if result["ok"]
      @flash_notice = notice
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    this._load()
    return render("triggers/index", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("triggers/index")
  end
end
