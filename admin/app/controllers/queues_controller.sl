# Queues - background job queues: stats, job lists, enqueue, cancel, run-now.

class QueuesController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases/:db/queues
  def index
    this._ctx()
    @title = "Queues · " + @db
    this._reset_banners()
    this._load()
  end

  # GET /databases/:db/queues/:name/jobs - HTMX-loaded fragment
  def jobs
    this._ctx()
    @queue_name = params["name"] ?? ""
    result = SolidbClient.get_api(SolidbEndpoints.queue_jobs(@db, @queue_name))
    @jobs = (result["data"] ?? {})["jobs"] ?? []
    @jobs_error = result["ok"] ? "" : (result["error"] ?? "request failed")
    return render("queues/_jobs", { "layout": false })
  end

  # POST /databases/:db/queues/enqueue (queue name comes from the form)
  def enqueue
    this._ctx()
    queue_name = (params["queue"] ?? "").trim()
    queue_name = "default" if queue_name.blank?
    payload = this._build_enqueue_payload()
    if payload.nil?
      return this._respond({ "ok": false, "status": 422, "error": "params must be a JSON object" }, "")
    end
    job_script = payload["script"] ?? ""
    if job_script.blank?
      return this._respond({ "ok": false, "status": 422, "error": "script is required" }, "")
    end
    result = SolidbClient.post_api(SolidbEndpoints.queue_enqueue(@db, queue_name), payload)
    return this._respond(result, "job enqueued on " + queue_name)
  end

  # DELETE /databases/:db/queues/jobs/:id
  def cancel_job
    this._ctx()
    result = SolidbClient.delete_api(SolidbEndpoints.queue_job(@db, params["id"] ?? ""))
    return this._respond(result, "job cancelled")
  end

  # POST /databases/:db/queues/jobs/:id/run-now
  def run_now
    this._ctx()
    result = SolidbClient.post_api(SolidbEndpoints.queue_job_run_now(@db, params["id"] ?? ""))
    return this._respond(result, "job scheduled to run now")
  end

  def _ctx
    @db = params["db"] ?? ""
    @databases = AdminContext.database_names()
  end

  # nil when the params textarea holds invalid JSON.
  def _build_enqueue_payload
    job_params = {}
    params_text = (params["job_params"] ?? "").trim()
    if !params_text.blank?
      job_params = JSON.parse(params_text) rescue nil
      return nil if job_params.nil?
    end
    payload = { "script": (params["script"] ?? "").trim(), "params": job_params }
    priority = (params["priority"] ?? "").trim()
    payload["priority"] = priority.to_int() unless priority.blank?
    max_retries = (params["max_retries"] ?? "").trim()
    payload["max_retries"] = max_retries.to_int() unless max_retries.blank?
    run_at = (params["run_at"] ?? "").trim()
    payload["run_at"] = run_at unless run_at.blank?
    return payload
  end

  def _load
    result = SolidbClient.get_api(SolidbEndpoints.queues(@db))
    @queues = result["data"] ?? []
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
    @title = "Queues · " + @db
    this._reset_banners()
    if result["ok"]
      @flash_notice = notice
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    this._load()
    return render("queues/index", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("queues/index")
  end
end
