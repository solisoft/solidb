describe("LiveQueriesController") do
  before_each() do
    as_guest()
  end

  describe("GET /databases/:db/live") do
    test("renders the live console") do
      response = get("/databases/_system/live")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "Live queries")
      assert_contains(body, "/databases/_system/live/token")
    end
  end

  describe("GET /databases/:db/live/token") do
    test("mints a short-lived ws token") do
      response = get("/databases/_system/live/token")
      assert_eq(res_status(response), 200)
      data = res_json(response)
      assert(data["ok"])
      assert_not(data["token"].blank?)
      assert_contains(data["ws_url"], "/_api/ws/changefeed")
      assert_eq(data["database"], "_system")
    end
  end
end
