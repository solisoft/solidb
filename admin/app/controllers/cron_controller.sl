# Cron - scheduled jobs (cron expression -> script on a queue).

class CronController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases/:db/cron
  def index
    this._ctx()
    @title = "Cron · " + @db
    this._reset_banners()
    this._load()
  end

  # POST /databases/:db/cron
  def create
    this._ctx()
    payload = this._build_payload()
    if payload.nil?
      return this._respond({ "ok": false, "status": 422, "error": "params must be a JSON object" }, "")
    end
    cron_name = payload["name"] ?? ""
    has_target = (payload["script"] ?? "") != "" || (payload["webhook_url"] ?? "") != ""
    if cron_name.blank? || (payload["cron_expression"] ?? "") == "" || !has_target
      return this._respond({ "ok": false, "status": 422,
                             "error": "name, cron expression and a script or webhook target are required" }, "")
    end
    result = SolidbClient.post_api(SolidbEndpoints.cron_jobs(@db), payload)
    return this._respond(result, "cron job " + cron_name + " created")
  end

  # PUT /databases/:db/cron/:id
  def update
    this._ctx()
    payload = this._build_payload()
    if payload.nil?
      return this._respond({ "ok": false, "status": 422, "error": "params must be a JSON object" }, "")
    end
    result = SolidbClient.put_api(SolidbEndpoints.cron_job(@db, params["id"] ?? ""), payload)
    return this._respond(result, "cron job " + (payload["name"] ?? "") + " updated")
  end

  # DELETE /databases/:db/cron/:id
  def delete
    this._ctx()
    result = SolidbClient.delete_api(SolidbEndpoints.cron_job(@db, params["id"] ?? ""))
    return this._respond(result, "cron job deleted")
  end

  def _ctx
    @db = params["db"] ?? ""
    @databases = AdminContext.database_names()
  end

  # nil when the params textarea holds invalid JSON.
  def _build_payload
    job_params = {}
    params_text = (params["job_params"] ?? "").trim()
    if !params_text.blank?
      job_params = JSON.parse(params_text) rescue nil
      return nil if job_params.nil?
    end
    payload = {
      "name": (params["cron_name"] ?? "").trim(),
      "cron_expression": (params["cron_expression"] ?? "").trim(),
      "script": (params["script"] ?? "").trim(),
      "params": job_params
    }
    # SoliDB rejects a job with BOTH a script and a webhook -- only forward the
    # webhook fields when a url was actually given (the form disables the
    # inactive target so its inputs never submit).
    webhook_url = (params["webhook_url"] ?? "").trim()
    payload["webhook_url"] = webhook_url unless webhook_url.blank?
    webhook_secret = (params["webhook_secret"] ?? "").trim()
    payload["webhook_secret"] = webhook_secret unless webhook_secret.blank?
    queue = (params["queue"] ?? "").trim()
    payload["queue"] = queue unless queue.blank?
    priority = (params["priority"] ?? "").trim()
    payload["priority"] = priority.to_int() unless priority.blank?
    max_retries = (params["max_retries"] ?? "").trim()
    payload["max_retries"] = max_retries.to_int() unless max_retries.blank?
    return payload
  end

  def _load
    result = SolidbClient.get_api(SolidbEndpoints.cron_jobs(@db))
    # Normalize the API shape (older servers return arrays, not objects) so the
    # view can index cron["_key"] without a 500. See ApiList.
    @cron_jobs = ApiList.normalize(result["data"] ?? [])
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
    @title = "Cron · " + @db
    this._reset_banners()
    if result["ok"]
      @flash_notice = notice
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    this._load()
    return render("cron/index", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("cron/index")
  end
end
