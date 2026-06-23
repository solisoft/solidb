# Materialized views - list / create / refresh / drop per database.
#
# SoliDB manages these through SDBQL statements (CREATE/REFRESH MATERIALIZED
# VIEW) run on the cursor API. Metadata lives in the per-db `_views` system
# collection; the server stores the view query as a serialized AST, so on
# create we annotate the metadata doc with the original SDBQL source
# (query_text) to keep definitions readable in the UI. There is no DROP
# statement: dropping = delete the metadata doc + drop the backing collection.

class MaterializedViewsController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases/:db/views
  def index
    this._ctx()
    @title = "Views · " + @db
    this._reset_banners()
    this._load()
  end

  # POST /databases/:db/views
  def create
    this._ctx()
    name = (params["name"] ?? "").trim()
    query_text = (params["query"] ?? "").trim()
    if !this._valid_view_name(name)
      return this._respond({ "ok": false, "status": 422,
                             "error": "view name must be an identifier (letters, digits, underscore)" }, "")
    end
    if query_text.blank?
      return this._respond({ "ok": false, "status": 422, "error": "view query is required" }, "")
    end
    statement = "CREATE MATERIALIZED VIEW " + name + " AS " + query_text
    result = SolidbClient.post_api(SolidbEndpoints.cursor(@db), { "query": statement })
    this._annotate_query_text(name, query_text) if result["ok"]
    return this._respond(result, "view " + name + " created")
  end

  # PUT /databases/:db/views/:name/refresh
  def refresh
    this._ctx()
    name = params["name"] ?? ""
    if !this._valid_view_name(name)
      return this._respond({ "ok": false, "status": 422, "error": "invalid view name" }, "")
    end
    statement = "REFRESH MATERIALIZED VIEW " + name
    result = SolidbClient.post_api(SolidbEndpoints.cursor(@db), { "query": statement })
    refreshed_count = (result["data"] ?? {})["inserted"] ?? 0
    return this._respond(result, "view " + name + " refreshed (" + str(refreshed_count) + " documents)")
  end

  # DELETE /databases/:db/views/:name
  def delete
    this._ctx()
    name = params["name"] ?? ""
    result = SolidbClient.delete_api(SolidbEndpoints.document(@db, "_views", name))
    if !result["ok"]
      return this._respond(result, "")
    end
    # Backing collection may already be gone - the metadata doc was the
    # source of truth, so a miss here is not an error worth surfacing.
    SolidbClient.delete_api(SolidbEndpoints.collection(@db, name))
    return this._respond(result, "view " + name + " dropped")
  end

  # View names are spliced into SDBQL statements - restrict to identifiers
  # so a crafted name can't smuggle extra clauses in.
  def _valid_view_name(name)
    return false if name.blank?
    cleaned = name.gsub("[^A-Za-z0-9_]", "")
    return false if cleaned != name
    first_char = name.substring(0, 1)
    return !"0123456789".includes?(first_char)
  end

  # Best-effort: PUT merges fields into the metadata doc, preserving the
  # stored AST. A failure here only loses the pretty definition display.
  def _annotate_query_text(name, query_text)
    SolidbClient.put_api(SolidbEndpoints.document(@db, "_views", name), { "query_text": query_text })
  end

  # Route context: set explicitly per action (before_action hooks are wired
  # by a startup-time scan and are unreliable under dev hot-reload).
  def _ctx
    @db = params["db"] ?? ""
    @databases = AdminContext.database_names()
  end

  def _load
    list_query = "FOR v IN _views FILTER v.type == \"materialized\" SORT v._key ASC " +
                 "RETURN { name: v._key, created_at: v.created_at, query_text: v.query_text }"
    result = SolidbClient.post_api(SolidbEndpoints.cursor(@db), { "query": list_query })
    # No _views collection yet just means no views were ever created.
    @views = result["ok"] ? ((result["data"] ?? {})["result"] ?? []) : []
    counts = {}
    names = []
    collections_result = SolidbClient.get_api(SolidbEndpoints.collections(@db))
    for coll in ((collections_result["data"] ?? {})["collections"] ?? [])
      coll_name = coll["name"] ?? ""
      counts[coll_name] = coll["count"] ?? 0
      names.push(coll_name) unless coll_name.blank?
    end
    @view_counts = counts
    @collection_names = names
  end

  def _reset_banners
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
  end

  def _respond(result, notice)
    @title = "Views · " + @db
    this._reset_banners()
    if result["ok"]
      @flash_notice = notice
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    this._load()
    return render("materialized_views/index", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("materialized_views/index")
  end
end
