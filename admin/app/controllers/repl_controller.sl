# Lua REPL - interactive Lua execution against a database. SoliDB keeps
# session state server-side (variables persist across evals); the page holds
# the session id in a hidden field and round-trips it on every eval.

class ReplController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases/:db/repl
  def show
    this._ctx()
    @title = "Lua REPL · " + @db
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
  end

  # POST /databases/:db/repl/eval - returns one log entry fragment (HTMX)
  def eval
    this._ctx()
    code = (params["code"] ?? "").trim()
    @repl_code = code
    @repl_output = []
    @repl_result = nil
    @repl_lua_error = nil
    @repl_time = 0
    @repl_session = (params["session_id"] ?? "").trim()
    @repl_transport_error = ""
    if code.blank?
      @repl_transport_error = "nothing to run"
      return render("repl/_result", { "layout": false })
    end
    payload = { "code": code }
    payload["session_id"] = @repl_session unless @repl_session.blank?
    result = SolidbClient.post_api(SolidbEndpoints.repl(@db), payload)
    if !result["ok"]
      @repl_transport_error = result["error"] ?? "request failed"
      return render("repl/_result", { "layout": false })
    end
    data = result["data"] ?? {}
    @repl_result = data["result"]
    @repl_output = data["output"] ?? []
    @repl_lua_error = data["error"]
    @repl_time = data["execution_time_ms"] ?? 0
    @repl_session = data["session_id"] ?? @repl_session
    return render("repl/_result", { "layout": false })
  end

  # Route context: set explicitly per action (before_action hooks are wired
  # by a startup-time scan and are unreliable under dev hot-reload).
  def _ctx
    @db = params["db"] ?? ""
    @databases = AdminContext.database_names()
  end
end
