# app/services/solidb_client.sl
#
# Single gateway to the SoliDB HTTP API. Controllers never call HTTP.request
# directly -- they go through SolidbClient.get_api/post_api/put_api/delete_api,
# which return a uniform { "ok", "status", "data", "error" } hash.
#
# Connection: the browser session can override the target server (set from
# the /connection page - host/username/password live in the server-side
# session store); otherwise the SOLIDB_* env vars apply. session_get is
# rescue-guarded because specs call this client outside a request context.
#
# Auth: POST /auth/login mints a JWT (~24h exp), cached process-wide PER
# "host|username" connection so different sessions/servers never clobber
# each other's token. On any 401 the entry is cleared and the request
# retried once with a fresh login, so expiry is self-healing.
#
# NOTE: HTTP.request()'s SSRF guard blocks loopback hosts unless
# SOLI_DEV_ALLOW_SSRF=1 is set (dev only -- see .env).

class SolidbClient
  # Process-wide JWT cache, keyed by "host|username".
  static cached_tokens: Hash = {}

  static def host()
    session_host = session_get("solidb_host") rescue nil
    return session_host unless session_host.blank?
    return getenv("SOLIDB_HOST") ?? "http://localhost:6745"
  end

  static def username()
    session_username = session_get("solidb_username") rescue nil
    return session_username unless session_username.blank?
    return getenv("SOLIDB_USERNAME") ?? ""
  end

  static def password()
    session_password = session_get("solidb_password") rescue nil
    return session_password unless session_password.blank?
    return getenv("SOLIDB_PASSWORD") ?? ""
  end

  # True when this browser session targets a server other than the env one.
  static def session_override()
    session_host = session_get("solidb_host") rescue nil
    return !session_host.blank?
  end

  # Browser-reachable WebSocket base for the changefeed page. A session
  # connection derives from its own host (SOLIDB_PUBLIC_WS_URL only describes
  # the env server); otherwise prefer the explicit env override (prod, behind
  # a proxy) and finally derive from SOLIDB_HOST by swapping the scheme.
  static def public_ws_url()
    return SolidbClient.derive_ws_url(SolidbClient.host()) if SolidbClient.session_override()
    explicit = getenv("SOLIDB_PUBLIC_WS_URL") ?? ""
    return explicit unless explicit.blank?
    return SolidbClient.derive_ws_url(SolidbClient.host())
  end

  # Scheme-swap derivation, split out so every host shape is unit-testable.
  static def derive_ws_url(http_url)
    if http_url.starts_with("https://")
      return "wss://" + http_url.substring(8, http_url.length())
    elsif http_url.starts_with("http://")
      return "ws://" + http_url.substring(7, http_url.length())
    end
    return http_url
  end

  # --- pure helpers (unit-tested without network) ---------------------------

  static def auth_headers(token)
    return {
      "Authorization": "Bearer " + token,
      "Content-Type":  "application/json",
      "Accept":        "application/json"
    }
  end

  static def is_unauthorized(resp)
    return false if resp.nil?
    return (resp["status"] ?? 0) == 401
  end

  # Raw HTTP response -> uniform result hash. nil means the request never got
  # out (connection refused / SSRF-blocked); status 0 marks that case so the
  # UI can show the "SoliDB unreachable" banner instead of a per-action error.
  static def interpret(resp)
    if resp.nil?
      return { "ok": false, "status": 0, "data": nil, "error": "SoliDB unreachable" }
    end
    status = resp["status"] ?? 0
    data = JSON.parse(resp["body"] ?? "") rescue nil
    if status >= 200 && status < 300
      return { "ok": true, "status": status, "data": data, "error": nil }
    end
    return { "ok": false, "status": status, "data": data, "error": SolidbClient.error_message(status, data) }
  end

  # Best-effort human message from a SoliDB error body.
  static def error_message(status, data)
    detail = ""
    detail = data["error"] ?? (data["message"] ?? "") unless data.nil?
    return "HTTP " + str(status) if detail.blank?
    return "HTTP " + str(status) + " - " + detail
  end

  # --- auth ------------------------------------------------------------------

  # One JWT per "host|username" connection.
  static def connection_key()
    return SolidbClient.host() + "|" + SolidbClient.username()
  end

  # Probe a login against an arbitrary server WITHOUT touching the current
  # connection - the /connection page validates credentials with this before
  # storing them in the session. Returns { "ok", "error", "token" }.
  static def probe_login(probe_host, probe_username, probe_password)
    headers = { "Content-Type": "application/json", "Accept": "application/json" }
    body = { "username": probe_username, "password": probe_password }
    resp = HTTP.request("POST", probe_host + "/auth/login", headers, body) rescue nil
    result = SolidbClient.interpret(resp)
    if !result["ok"]
      reason = result["error"] ?? "login failed"
      status = result["status"] ?? 0
      reason = "server unreachable (connection refused or blocked)" if status == 0
      return { "ok": false, "error": reason, "token": "" }
    end
    token = (result["data"] ?? {})["token"] ?? ""
    return { "ok": false, "error": "login response had no token", "token": "" } if token.blank?
    return { "ok": true, "error": nil, "token": token }
  end

  # Mint a JWT via POST /auth/login and cache it. Returns "" on failure (a
  # failed mint never poisons a previously-good cache entry -- it returns
  # before the assignment).
  static def login()
    probe = SolidbClient.probe_login(SolidbClient.host(), SolidbClient.username(), SolidbClient.password())
    return "" unless probe["ok"]
    SolidbClient.cached_tokens[SolidbClient.connection_key()] = probe["token"]
    return probe["token"]
  end

  static def token()
    cached = SolidbClient.cached_tokens[SolidbClient.connection_key()] ?? ""
    return cached unless cached.blank?
    return SolidbClient.login()
  end

  static def clear_token()
    SolidbClient.cached_tokens[SolidbClient.connection_key()] = ""
  end

  # --- generic request with one re-auth retry on 401 -------------------------

  static def api(method, path, body = nil)
    auth = SolidbClient.token()
    if auth.blank?
      return { "ok": false, "status": 0, "data": nil,
               "error": "SoliDB authentication failed (check the Connection page or the SOLIDB_* env vars)" }
    end
    resp = SolidbClient.send_request(method, path, body, auth)
    # A nil response can be a stale pooled keep-alive connection dying on
    # send ("error sending request") - the request never reached SoliDB,
    # so one immediate retry on a fresh connection is safe and self-heals.
    resp = SolidbClient.send_request(method, path, body, auth) if resp.nil?
    if SolidbClient.is_unauthorized(resp)
      SolidbClient.clear_token()
      auth = SolidbClient.login()
      return SolidbClient.interpret(resp) if auth.blank?
      resp = SolidbClient.send_request(method, path, body, auth)
    end
    return SolidbClient.interpret(resp)
  end

  static def send_request(method, path, body, auth)
    headers = SolidbClient.auth_headers(auth)
    full_url = SolidbClient.host() + path
    if body.nil?
      resp = HTTP.request(method, full_url, headers) rescue nil
      return resp
    end
    resp = HTTP.request(method, full_url, headers, body) rescue nil
    return resp
  end

  static def get_api(path)
    return SolidbClient.api("GET", path)
  end

  # GET returning the raw response body (no JSON parsing) - for proxying
  # downloads like the collection JSONL export. Same one-retry-on-401 shape
  # as api().
  static def get_raw(path)
    auth = SolidbClient.token()
    if auth.blank?
      return { "ok": false, "status": 0, "body": "",
               "error": "SoliDB authentication failed (check the Connection page or the SOLIDB_* env vars)" }
    end
    resp = SolidbClient.send_request("GET", path, nil, auth)
    resp = SolidbClient.send_request("GET", path, nil, auth) if resp.nil?
    if SolidbClient.is_unauthorized(resp)
      SolidbClient.clear_token()
      auth = SolidbClient.login()
      resp = SolidbClient.send_request("GET", path, nil, auth) unless auth.blank?
    end
    return { "ok": false, "status": 0, "body": "", "error": "SoliDB unreachable" } if resp.nil?
    status = resp["status"] ?? 0
    if status >= 200 && status < 300
      return { "ok": true, "status": status, "body": resp["body"] ?? "", "error": nil }
    end
    data = JSON.parse(resp["body"] ?? "") rescue nil
    return { "ok": false, "status": status, "body": "", "error": SolidbClient.error_message(status, data) }
  end

  # POST a single text file as multipart/form-data (field name "file") - the
  # shape the import API expects. Content is text (JSON/JSONL), so a
  # string-built body is byte-safe.
  static def post_multipart(path, filename, content)
    auth = SolidbClient.token()
    if auth.blank?
      return { "ok": false, "status": 0, "data": nil,
               "error": "SoliDB authentication failed (check the Connection page or the SOLIDB_* env vars)" }
    end
    boundary = "----solidb-admin-import-7f3d9a1c2e"
    # The filename is client-supplied: strip quote/CRLF so it cannot break out
    # of the Content-Disposition header.
    safe_filename = filename.replace("\"", "").replace("\r", "").replace("\n", "")
    body = "--" + boundary + "\r\n"
    body = body + "Content-Disposition: form-data; name=\"file\"; filename=\"" + safe_filename + "\"\r\n"
    body = body + "Content-Type: application/octet-stream\r\n\r\n"
    body = body + content + "\r\n"
    body = body + "--" + boundary + "--\r\n"
    headers = {
      "Authorization": "Bearer " + auth,
      "Content-Type":  "multipart/form-data; boundary=" + boundary,
      "Accept":        "application/json"
    }
    full_url = SolidbClient.host() + path
    resp = HTTP.request("POST", full_url, headers, body) rescue nil
    if SolidbClient.is_unauthorized(resp)
      SolidbClient.clear_token()
      auth = SolidbClient.login()
      if !auth.blank?
        headers["Authorization"] = "Bearer " + auth
        resp = HTTP.request("POST", full_url, headers, body) rescue nil
      end
    end
    return SolidbClient.interpret(resp)
  end

  static def post_api(path, body = nil)
    return SolidbClient.api("POST", path, body)
  end

  static def put_api(path, body = nil)
    return SolidbClient.api("PUT", path, body)
  end

  static def delete_api(path)
    return SolidbClient.api("DELETE", path)
  end

  # --- live queries -----------------------------------------------------------

  # Short-lived token for the browser's changefeed WebSocket.
  static def livequery_token()
    result = SolidbClient.get_api("/_api/livequery/token")
    return "" unless result["ok"]
    return (result["data"] ?? {})["token"] ?? ""
  end
end
