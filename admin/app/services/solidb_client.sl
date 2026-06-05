# app/services/solidb_client.sl
#
# Single gateway to the SoliDB HTTP API. Controllers never call HTTP.request
# directly -- they go through SolidbClient.get_api/post_api/put_api/delete_api,
# which return a uniform { "ok", "status", "data", "error" } hash.
#
# Auth: POST /auth/login with the SOLIDB_USERNAME/SOLIDB_PASSWORD env creds
# yields a JWT (~24h exp). The token is cached process-wide in a static field
# (same pattern as bonfire's SolidbLive). On any 401 the cache is cleared and
# the request retried once with a fresh login, so expiry is self-healing.
#
# NOTE: HTTP.request()'s SSRF guard blocks loopback hosts unless
# SOLI_DEV_ALLOW_SSRF=1 is set (dev only -- see .env).

class SolidbClient
  # Process-wide JWT cache. One shared admin token is correct here: every
  # request acts as the single SOLIDB_USERNAME identity, nothing per-user.
  static cached_token: String = ""

  static def host()
    return getenv("SOLIDB_HOST") ?? "http://localhost:6745"
  end

  static def username()
    return getenv("SOLIDB_USERNAME") ?? "admin"
  end

  static def password()
    return getenv("SOLIDB_PASSWORD") ?? "admin"
  end

  # Browser-reachable WebSocket base for the changefeed page. Prefer an
  # explicit SOLIDB_PUBLIC_WS_URL (prod, behind a proxy); otherwise derive
  # from SOLIDB_HOST by swapping the scheme to ws(s).
  static def public_ws_url()
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

  # Mint a JWT via POST /auth/login and cache it. Returns "" on failure (a
  # failed mint never poisons a previously-good cache entry -- it returns
  # before the assignment).
  static def login()
    headers = { "Content-Type": "application/json", "Accept": "application/json" }
    body = { "username": SolidbClient.username(), "password": SolidbClient.password() }
    resp = HTTP.request("POST", SolidbClient.host() + "/auth/login", headers, body) rescue nil
    result = SolidbClient.interpret(resp)
    return "" unless result["ok"]
    token = (result["data"] ?? {})["token"] ?? ""
    return "" if token.blank?
    SolidbClient.cached_token = token
    return token
  end

  static def token()
    return SolidbClient.cached_token unless SolidbClient.cached_token.blank?
    return SolidbClient.login()
  end

  static def clear_token()
    SolidbClient.cached_token = ""
  end

  # --- generic request with one re-auth retry on 401 -------------------------

  static def api(method, path, body = nil)
    auth = SolidbClient.token()
    if auth.blank?
      return { "ok": false, "status": 0, "data": nil,
               "error": "SoliDB authentication failed (check SOLIDB_HOST / SOLIDB_USERNAME / SOLIDB_PASSWORD)" }
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
