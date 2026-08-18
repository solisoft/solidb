# First-connection gate. SOLIDB_USERNAME is set in .env.test, so the
# middleware must let the rest of the suite through; the unconfigured branch
# is exercised through the pure gate, which needs no process-wide state.
describe("require_solidb_connection") do
  before_each() do
    as_guest()
  end

  test("does not redirect when credentials come from the environment") do
    result = require_solidb_connection({ "path": "/" })
    assert(result["continue"])
    response = get("/")
    assert_eq(res_status(response), 200)
    assert_contains(res_body(response), "Dashboard")
  end

  test("still serves /connection and /health without a session override") do
    assert_eq(res_status(get("/connection")), 200)
    assert_eq(res_status(get("/health")), 200)
  end

  test("redirects app pages to /connection when no credentials are set") do
    result = SolidbClient.connection_gate({ "path": "/" }, false)
    assert_not(result["continue"])
    assert_eq(result["response"]["status"], 302)
    assert_eq(result["response"]["headers"]["Location"], "/connection")
  end

  test("lets the connection form and health probe through when unconfigured") do
    for allowed in ["/connection", "/health", "/css/application.css"]
      result = SolidbClient.connection_gate({ "path": allowed }, false)
      assert(result["continue"])
    end
  end

  test("passes the request through untouched when it continues") do
    req = { "path": "/databases", "method": "GET" }
    result = SolidbClient.connection_gate(req, true)
    assert(result["continue"])
    assert_eq(result["request"]["path"], "/databases")
  end
end
