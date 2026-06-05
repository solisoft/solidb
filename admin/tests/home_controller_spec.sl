describe("HomeController") do
  before_each() do
    as_guest()
  end

  describe("GET /") do
    test("renders the dashboard with server status and node info") do
      response = get("/")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "Dashboard")
      assert_contains(body, "ONLINE")
      assert_contains(body, "node id")
      assert_contains(body, "Databases")
    end
  end

  describe("GET /health") do
    test("returns ok JSON") do
      response = get("/health")
      assert_eq(res_status(response), 200)
      assert_eq(res_json(response)["status"], "ok")
    end
  end
end
