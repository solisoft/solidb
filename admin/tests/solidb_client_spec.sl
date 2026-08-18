# Tests the pure seams of app/services/solidb_client.sl without any network:
# response interpretation, header building, ws-url derivation, and the token
# cache fields driven directly (same approach as bonfire's solidb_live_spec).
describe("SolidbClient") do
  before_each() do
    # Start every test from an empty token cache so order can't leak state.
    SolidbClient.cached_tokens = {}
  end

  describe("derive_ws_url") do
    test("swaps http to ws") do
      assert_eq(SolidbClient.derive_ws_url("http://localhost:6745"), "ws://localhost:6745")
    end

    test("swaps https to wss") do
      assert_eq(SolidbClient.derive_ws_url("https://db.example.com"), "wss://db.example.com")
    end

    test("passes through unknown schemes untouched") do
      assert_eq(SolidbClient.derive_ws_url("ws://already"), "ws://already")
    end
  end

  describe("public_ws_url") do
    test("derives from SOLIDB_HOST when no override is set") do
      # .env.test points SOLIDB_HOST at http://127.0.0.1:6745 and sets no override.
      assert_eq(SolidbClient.public_ws_url(), "ws://127.0.0.1:6745")
    end
  end

  describe("auth_headers") do
    test("builds a bearer authorization header") do
      headers = SolidbClient.auth_headers("tok123")
      assert_eq(headers["Authorization"], "Bearer tok123")
      assert_eq(headers["Content-Type"], "application/json")
      assert_eq(headers["Accept"], "application/json")
    end
  end

  describe("is_unauthorized") do
    test("nil response is not a 401") do
      assert_not(SolidbClient.is_unauthorized(nil))
    end

    test("401 status is unauthorized") do
      assert(SolidbClient.is_unauthorized({ "status": 401, "body": "" }))
    end

    test("200 status is not unauthorized") do
      assert_not(SolidbClient.is_unauthorized({ "status": 200, "body": "" }))
    end
  end

  describe("interpret") do
    test("nil response means unreachable with status 0") do
      result = SolidbClient.interpret(nil)
      assert_not(result["ok"])
      assert_eq(result["status"], 0)
      assert_eq(result["error"], "SoliDB unreachable")
    end

    test("2xx with JSON body is ok with parsed data") do
      result = SolidbClient.interpret({ "status": 200, "body": "{\"databases\":[\"_system\"]}" })
      assert(result["ok"])
      assert_eq(result["status"], 200)
      assert_eq(result["data"]["databases"][0], "_system")
      assert_null(result["error"])
    end

    test("2xx with invalid JSON is still ok with nil data") do
      result = SolidbClient.interpret({ "status": 204, "body": "" })
      assert(result["ok"])
      assert_null(result["data"])
    end

    test("4xx surfaces the server error detail") do
      result = SolidbClient.interpret({ "status": 409, "body": "{\"error\":\"database exists\"}" })
      assert_not(result["ok"])
      assert_eq(result["status"], 409)
      assert_eq(result["error"], "HTTP 409 - database exists")
    end

    test("5xx without a parseable body falls back to the bare status") do
      result = SolidbClient.interpret({ "status": 500, "body": "boom" })
      assert_not(result["ok"])
      assert_eq(result["error"], "HTTP 500")
    end
  end

  describe("error_message") do
    test("uses the message field when error is absent") do
      assert_eq(SolidbClient.error_message(403, { "message": "forbidden" }), "HTTP 403 - forbidden")
    end

    test("handles nil data") do
      assert_eq(SolidbClient.error_message(404, nil), "HTTP 404")
    end
  end

  describe("env accessors") do
    test("host/username come from the .env") do
      assert_eq(SolidbClient.host(), "http://127.0.0.1:6745")
      assert_eq(SolidbClient.username(), "admin")
    end

    test("credentials are configured when SOLIDB_USERNAME is set") do
      assert(SolidbClient.credentials_configured())
    end
  end

  describe("loopback probe diagnostics") do
    test("rewrites localhost to 127.0.0.1 and leaves explicit hosts alone") do
      assert_eq(SolidbClient.prefer_ipv4_loopback("http://localhost:6745"), "http://127.0.0.1:6745")
      # An explicit [::1] is the operator's choice, not an ambiguous name.
      assert_eq(SolidbClient.prefer_ipv4_loopback("http://[::1]:6745"), "http://[::1]:6745")
      assert_eq(SolidbClient.prefer_ipv4_loopback("https://db.example.com"), "https://db.example.com")
      assert_eq(SolidbClient.prefer_ipv4_loopback("http://localhost.example.com"),
                "http://localhost.example.com")
    end

    test("extracts the host from http(s) URLs") do
      assert_eq(SolidbClient.host_of("http://localhost:6745"), "localhost")
      assert_eq(SolidbClient.host_of("https://127.0.0.1:6745/auth"), "127.0.0.1")
      assert_eq(SolidbClient.host_of("http://[::1]:6745"), "[::1]")
    end

    test("recognizes loopback hosts") do
      assert(SolidbClient.loopback_host?("http://localhost:6745"))
      assert(SolidbClient.loopback_host?("http://127.0.0.1:6745"))
      assert(SolidbClient.loopback_host?("http://[::1]:6745"))
      assert_not(SolidbClient.loopback_host?("https://db.example.com"))
    end

    test("recognizes private and link-local hosts") do
      assert(SolidbClient.private_host?("http://10.0.0.5:6745"))
      assert(SolidbClient.private_host?("http://192.168.1.10:6745"))
      assert(SolidbClient.private_host?("http://169.254.169.254"))
      assert(SolidbClient.private_host?("http://172.16.0.1:6745"))
      assert(SolidbClient.private_host?("http://172.31.255.254:6745"))
      # 172.15 and 172.32 sit outside RFC1918.
      assert_not(SolidbClient.private_host?("http://172.15.0.1:6745"))
      assert_not(SolidbClient.private_host?("http://172.32.0.1:6745"))
      assert_not(SolidbClient.private_host?("http://172.example.com"))
      assert_not(SolidbClient.private_host?("https://db.example.com"))
    end

    test("test env allows loopback HTTP") do
      assert(SolidbClient.http_allowed?("http://127.0.0.1:6745"))
    end

    test("remote unreachable stays a generic connection error") do
      reason = SolidbClient.unreachable_reason("https://db.example.com")
      assert_eq(reason, "server unreachable (connection refused or blocked)")
    end

    test("an allow-list that names other hosts does not allow this one") do
      # The suite sets SOLI_DEV_ALLOW_SSRF, so exercise membership directly
      # rather than by unsetting a process-wide variable.
      assert(SolidbClient.host_in_allow_list?("127.0.0.1", "localhost,127.0.0.1,[::1]"))
      assert(SolidbClient.host_in_allow_list?("[::1]", "localhost, 127.0.0.1, [::1]"))
      assert_not(SolidbClient.host_in_allow_list?("127.0.0.1", "db.internal"))
      assert_not(SolidbClient.host_in_allow_list?("127.0.0.1", ""))
    end
  end

  describe("setup_bypass_path?") do
    test("allows the connection form, health probe, and static assets") do
      for allowed in ["/health", "/connection", "/connection/reset",
                      "/manifest.json", "/sw.js", "/offline.html",
                      "/css/application.css", "/js/htmx.min.js",
                      "/icons/icon-192.png", "/screenshots/wide.png"]
        assert(SolidbClient.setup_bypass_path?(allowed))
      end
    end

    test("does not bypass app pages") do
      for blocked in ["/", "/databases", "/settings", "/users"]
        assert_not(SolidbClient.setup_bypass_path?(blocked))
      end
    end
  end

  describe("token cache") do
    test("token() serves the cached value without logging in") do
      SolidbClient.cached_tokens[SolidbClient.connection_key()] = "cached.jwt"
      assert_eq(SolidbClient.token(), "cached.jwt")
    end

    test("clear_token() empties the current connection's entry") do
      SolidbClient.cached_tokens[SolidbClient.connection_key()] = "cached.jwt"
      SolidbClient.clear_token()
      assert_eq(SolidbClient.cached_tokens[SolidbClient.connection_key()], "")
    end

    test("tokens are cached per host|username connection") do
      SolidbClient.cached_tokens["http://other:6745|admin"] = "other.jwt"
      assert_ne(SolidbClient.token() ?? "", "other.jwt")
    end
  end

  describe("live API round-trip") do
    test("login mints and caches a token") do
      token = SolidbClient.login()
      assert_not(token.blank?)
      assert_eq(SolidbClient.cached_tokens[SolidbClient.connection_key()], token)
    end

    test("get_api lists databases") do
      result = SolidbClient.get_api(SolidbEndpoints.databases())
      assert(result["ok"])
      assert_contains(result["data"]["databases"], "_system")
    end

    test("a stale token self-heals via the 401 retry") do
      SolidbClient.cached_tokens[SolidbClient.connection_key()] = "stale.invalid.jwt"
      result = SolidbClient.get_api(SolidbEndpoints.databases())
      assert(result["ok"])
      assert_ne(SolidbClient.cached_tokens[SolidbClient.connection_key()], "stale.invalid.jwt")
    end

    test("livequery_token returns a non-empty token") do
      token = SolidbClient.livequery_token()
      assert_not(token.blank?)
    end
  end
end
