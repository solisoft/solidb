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

  # GET /databases/:db/queues/:name/jobs - HTMX-loaded fragment.
  # Accepts ?status=&limit=&offset= for filtering and pagination; the API
  # returns the full filtered total alongside the page so we can page on it.
  def jobs
    this._ctx()
    return this._render_jobs_fragment("", "")
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
  # Re-renders only the queue's jobs fragment so the expanded panel stays open
  # instead of reloading the whole page (which collapses it). The button sends
  # ?queue=&limit=&status=&offset= so we refresh the exact view the user is on.
  def cancel_job
    this._ctx()
    result = SolidbClient.delete_api(SolidbEndpoints.queue_job(@db, params["id"] ?? ""))
    notice = result["ok"] ? "job cancelled" : ""
    action_error = result["ok"] ? "" : (result["error"] ?? "request failed")
    return this._render_jobs_fragment(notice, action_error)
  end

  # POST /databases/:db/queues/jobs/:id/run-now
  # Same in-place refresh as cancel_job — see its note.
  def run_now
    this._ctx()
    result = SolidbClient.post_api(SolidbEndpoints.queue_job_run_now(@db, params["id"] ?? ""))
    notice = result["ok"] ? "job scheduled to run now" : ""
    action_error = result["ok"] ? "" : (result["error"] ?? "request failed")
    return this._render_jobs_fragment(notice, action_error)
  end

  # Renders the HTMX jobs fragment for a single queue, honoring the
  # filter/pagination state on the request. Shared by the jobs listing and the
  # cancel / run-now actions so those stay in-place instead of swapping the
  # whole #content (which would collapse the open queue panel). `notice` shows a
  # success line; `action_error` (a failed cancel/run-now) wins over any
  # downstream fetch error so the user sees the real cause.
  def _render_jobs_fragment(notice, action_error)
    # The jobs route carries the queue name in the path (:name); cancel/run-now
    # send it as ?queue= because their path slot holds the job id instead.
    @queue_name = params["name"] ?? (params["queue"] ?? "")
    @status_filter = this._status_filter()
    @jobs_limit = this._jobs_limit()
    offset = (params["offset"] ?? "0").to_int()
    offset = 0 if offset < 0
    @jobs_offset = offset
    endpoint = SolidbEndpoints.queue_jobs(@db, @queue_name) + this._jobs_query()
    result = SolidbClient.get_api(endpoint)
    data = result["data"] ?? {}
    @jobs = data["jobs"] ?? []
    @jobs_total = data["total"] ?? 0
    @jobs_notice = notice
    if !action_error.blank?
      @jobs_error = action_error
    else
      @jobs_error = result["ok"] ? "" : (result["error"] ?? "request failed")
    end
    return render("queues/_jobs", { "layout": false })
  end

  def _ctx
    @db = params["db"] ?? ""
    @databases = AdminContext.database_names()
  end

  # Whitelisted job status; "" means no filter (all statuses).
  def _status_filter
    status = (params["status"] ?? "").trim().downcase()
    return "" unless ["pending", "running", "completed", "failed"].includes?(status)
    return status
  end

  def _jobs_limit
    limit = (params["limit"] ?? "50").to_int()
    return 50 unless [25, 50, 100].includes?(limit)
    return limit
  end

  # Query string for the jobs API: limit/offset always sent, status only when set.
  def _jobs_query
    query = "?limit=" + str(@jobs_limit) + "&offset=" + str(@jobs_offset)
    # status is a whitelisted enum word (see _status_filter), so it needs no encoding.
    query = query + "&status=" + @status_filter unless @status_filter.blank?
    return query
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
    # Normalize the API shape (older SoliDB returns arrays, not stat objects)
    # so the view can always index queue["name"] without a 500. See QueuesView.
    @queues = QueuesView.normalize(result["data"] ?? [])
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
