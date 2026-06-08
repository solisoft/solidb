# Per-session SoliDB connection override. The "external server" in these
# specs is the same test instance (reached via its env host), which still
# exercises the full probe -> session -> override -> reset path.
describe("ConnectionController") do
  before_each() do
    as_guest()
  end

  describe("GET /connection") do
    test("renders the form with the current (env) connection") do
      response = get("/connection")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "Connection")
      assert_contains(body, "env defaults")
      assert_contains(body, "name=\"host\"")
      assert_contains(body, "name=\"username\"")
      assert_contains(body, "name=\"password\"")
    end

    test("ships the saved-connections component (localStorage)") do
      response = get("/connection")
      body = res_body(response)
      assert_contains(body, "saved connections")
      assert_contains(body, "connectionPage")
      assert_contains(body, "solidb-admin:connections")
      assert_contains(body, "save this connection in this browser")
      assert_contains(body, "remember the password")
    end
  end

  describe("POST /connection") do
    test("rejects a non-http url") do
      response = post("/connection", { "host": "ftp://example.com", "username": "admin", "password": "x" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "must start with http:// or https://")
    end

    test("rejects a blank username") do
      response = post("/connection", { "host": "http://127.0.0.1:6745", "username": "", "password": "x" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "username is required")
    end

    test("surfaces an unreachable server") do
      response = post("/connection", { "host": "http://127.0.0.1:1", "username": "admin", "password": "x" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "could not connect")
    end

    test("surfaces bad credentials") do
      response = post("/connection",
                      { "host": SolidbClient.host(), "username": "admin", "password": "definitely-wrong" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "could not connect")
    end

    test("connects, overrides the session, and resets back") do
      response = post("/connection",
                      { "host": SolidbClient.host(), "username": "admin", "password": "admin" })
      assert(res_redirect?(response))
      assert_contains(res_location(response), "/databases")

      # The override is visible on the connection page...
      response = get("/connection")
      assert_contains(res_body(response), "session override")
      # ...and the app keeps working through the overridden connection.
      response = get("/databases")
      assert_eq(res_status(response), 200)

      response = post("/connection/reset", {})
      assert(res_redirect?(response))
      response = get("/connection")
      assert_contains(res_body(response), "env defaults")
    end

    test("a bare host:port is normalized to http://") do
      env_host = SolidbClient.host()
      bare_host = env_host.replace("http://", "")
      response = post("/connection", { "host": bare_host + "/", "username": "admin", "password": "admin" })
      assert(res_redirect?(response))
      response = get("/connection")
      assert_contains(res_body(response), env_host)
      post("/connection/reset", {})
    end
  end
end
