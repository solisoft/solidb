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
