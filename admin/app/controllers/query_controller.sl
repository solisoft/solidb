# Query console - run SDBQL queries and explain plans against a database.
# Also exposes the slow query log: SoliDB records every query slower than
# 100ms into the per-db _slow_queries system collection.

class QueryController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases/:db/query
  def show
    this._ctx()
    @title = "Query · " + @db
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
    collections_result = SolidbClient.get_api(SolidbEndpoints.collections(@db))
    raw_collections = (collections_result["data"] ?? {})["collections"] ?? []
    @collection_names = raw_collections.map do |coll| coll["name"] ?? "" end
  end

  # POST /databases/:db/query/run - returns the results fragment (HTMX)
  def run
    this._ctx()
    payload = this._build_payload()
    if payload.nil?
      return this._render_results({ "ok": false, "error": "bind vars must be a JSON object" })
    end
    result = SolidbClient.post_api(SolidbEndpoints.cursor(@db), payload)
    return this._render_results(result)
  end

  # POST /databases/:db/query/explain - returns the explain fragment (HTMX)
  def explain
    this._ctx()
    payload = this._build_payload()
    if payload.nil?
      return this._render_explain({ "ok": false, "error": "bind vars must be a JSON object" })
    end
    result = SolidbClient.post_api(SolidbEndpoints.explain(@db), payload)
    return this._render_explain(result)
  end

  # GET /databases/:db/query/slow
  def slow
    this._ctx()
    @title = "Slow queries · " + @db
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
    this._load_slow()
  end

  # GET /databases/:db/query/slow/count - sidebar badge fragment (HTMX)
  def slow_count
    this._ctx()
    @slow_count = this._count_slow()
    return render("query/_slow_badge", { "layout": false })
  end

  # PUT /databases/:db/query/slow/clear
  def clear_slow
    this._ctx()
    @title = "Slow queries · " + @db
    result = SolidbClient.put_api(SolidbEndpoints.collection_truncate(@db, "_slow_queries"))
    @flash_error = result["ok"] ? "" : (result["error"] ?? "request failed")
    @flash_notice = result["ok"] ? "slow query log cleared" : ""
    @solidb_down = (result["status"] ?? -1) == 0
    this._load_slow()
    return render("query/slow", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("query/slow")
  end

  # Route context: set explicitly per action (before_action hooks are wired
  # by a startup-time scan and are unreliable under dev hot-reload).
  def _ctx
    @db = params["db"] ?? ""
    @databases = AdminContext.database_names()
  end

  # nil when the bind-vars textarea holds invalid JSON.
  def _build_payload
    bind_text = (params["bind_vars"] ?? "").trim()
    bind_vars = {}
    if !bind_text.blank?
      bind_vars = JSON.parse(bind_text) rescue nil
      return nil if bind_vars.nil?
    end
    # cache: false -- the console is for inspecting live data and measuring
    # real execution time; a cached result would silently lie about both.
    return { "query": params["query"] ?? "", "bindVars": bind_vars, "batchSize": 200, "cache": false }
  end

  def _render_results(result)
    @query_error = result["ok"] ? "" : (result["error"] ?? "request failed")
    data = result["data"] ?? {}
    @rows = data["result"] ?? []
    @meta = data
    return render("query/_results", { "layout": false })
  end

  def _render_explain(result)
    @explain_error = result["ok"] ? "" : (result["error"] ?? "request failed")
    @plan = result["data"] ?? {}
    return render("query/_explain", { "layout": false })
  end

  def _load_slow
    statement = "FOR q IN _slow_queries SORT q.timestamp DESC LIMIT 200 RETURN q"
    result = SolidbClient.post_api(SolidbEndpoints.cursor(@db), { "query": statement })
    @slow_queries = (result["data"] ?? {})["result"] ?? []
    if !result["ok"]
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
  end

  # 0 when the _slow_queries collection is missing or the query fails --
  # the badge just stays hidden. (COLLECT WITH COUNT INTO parses in the AST
  # but the SDBQL parser rejects it at runtime; counting rows client-side
  # with a cap is the portable form.)
  def _count_slow
    statement = "FOR q IN _slow_queries LIMIT 999 RETURN 1"
    result = SolidbClient.post_api(SolidbEndpoints.cursor(@db), { "query": statement })
    return 0 if !result["ok"]
    rows = (result["data"] ?? {})["result"] ?? []
    return rows.length()
  end
end
