# Connection - point this browser session at any SoliDB server. Credentials
# are validated with a real /auth/login probe before being stored in the
# server-side session; the SOLIDB_* env vars stay the default for sessions
# without an override.

class ConnectionController < Controller
  static {
    this.layout = "application"
  }

  # GET /connection
  def show
    this._ctx()
  end

  # POST /connection - probe the target, then store it in the session.
  def create
    this._ctx()
    target_host = this._normalize_host(params["host"] ?? "")
    target_username = (params["username"] ?? "").trim()
    target_password = params["password"] ?? ""
    if target_host.blank?
      @connection_error = "server URL must start with http:// or https://"
      return render("connection/show")
    end
    if target_username.blank?
      @connection_error = "username is required"
      return render("connection/show")
    end
    probe = SolidbClient.probe_login(target_host, target_username, target_password)
    if !probe["ok"]
      @connection_error = "could not connect to " + target_host + " — " + (probe["error"] ?? "login failed")
      return render("connection/show")
    end
    # Credentials change -> new session id (fixation hygiene), then store the
    # override and seed the token cache so the next page needs no re-login.
    session_regenerate
    session_set("solidb_host", target_host)
    session_set("solidb_username", target_username)
    session_set("solidb_password", target_password)
    SolidbClient.cached_tokens[target_host + "|" + target_username] = probe["token"]
    return redirect(databases_path())
  end

  # POST /connection/reset - back to the SOLIDB_* env defaults.
  def reset
    session_delete("solidb_host")
    session_delete("solidb_username")
    session_delete("solidb_password")
    return redirect(connection_path())
  end

  # "host:port" is forgiven (http:// assumed); anything else must be a full
  # http(s) URL. Trailing slashes are stripped so path concatenation works.
  def _normalize_host(raw_host)
    candidate = raw_host.trim()
    return "" if candidate.blank?
    if !candidate.starts_with("http://") && !candidate.starts_with("https://")
      return "" if candidate.includes?("://")
      candidate = "http://" + candidate
    end
    candidate = candidate.substring(0, candidate.length() - 1) if candidate.ends_with("/")
    return candidate
  end

  def _ctx
    @title = "Connection"
    @databases = AdminContext.database_names()
    @db = ""
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
    @connection_error = ""
    # Form echo on validation failure (templates cannot read params).
    @form_host = (params["host"] ?? "").trim()
    @form_username = (params["username"] ?? "").trim()
    @current_host = SolidbClient.host()
    @current_username = SolidbClient.username()
    @overridden = SolidbClient.session_override()
    @env_host = getenv("SOLIDB_HOST") ?? "http://localhost:6745"
  end
end
