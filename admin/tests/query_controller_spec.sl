describe("QueryController") do
  before_each() do
    as_guest()
  end

  describe("GET /databases/:db/query") do
    test("renders the console") do
      response = get("/databases/_system/query")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "Query console")
    end
  end

  describe("POST /databases/:db/query/run") do
    test("runs a query and renders rows with timing") do
      response = post("/databases/_system/query/run", { "query": "RETURN 41+1", "bind_vars": "" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "42")
      assert_contains(body, "ms")
    end

    test("binds variables from the json textarea") do
      response = post("/databases/_system/query/run",
                      { "query": "RETURN @x", "bind_vars": "{\"x\": \"bound_value\"}" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "bound_value")
    end

    test("rejects invalid bind vars json") do
      response = post("/databases/_system/query/run", { "query": "RETURN 1", "bind_vars": "{nope" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "bind vars must be a JSON object")
    end

    test("surfaces sdbql errors") do
      response = post("/databases/_system/query/run", { "query": "THIS IS NOT SDBQL", "bind_vars": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "query error")
    end
  end

  describe("slow query log") do
    before_all() do
      SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_slow" })
      # SoliDB pre-creates _slow_queries with the database; seed one entry the
      # way the server's slow-query logger writes them.
      SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_slow", "_slow_queries"),
                            { "query": "FOR d IN big RETURN d", "execution_time_ms": 250.5,
                              "timestamp": "2026-06-05T10:00:00Z", "results_count": 9000,
                              "documents_inserted": 0, "documents_updated": 0, "documents_removed": 0 })
    end

    after_all() do
      SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_slow"))
    end

    test("renders logged slow queries") do
      response = get("/databases/admin_spec_slow/query/slow")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "Slow queries")
      assert_contains(body, "FOR d IN big RETURN d")
      assert_contains(body, "250.5")
    end

    test("badge fragment shows the count") do
      response = get("/databases/admin_spec_slow/query/slow/count")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "1")
    end

    test("clear truncates the log and empties the badge") do
      response = put("/databases/admin_spec_slow/query/slow/clear", {})
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "slow query log cleared")
      assert_contains(res_body(response), "no slow queries logged")

      response = get("/databases/admin_spec_slow/query/slow/count")
      assert_eq(res_status(response), 200)
      assert_not(res_body(response).includes?("1"))
    end
  end

  describe("POST /databases/:db/query/explain") do
    test("renders the plan") do
      response = post("/databases/_system/query/explain", { "query": "RETURN 1", "bind_vars": "" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "explain")
      assert_contains(body, "scanned")
    end

    test("rejects invalid bind vars json") do
      response = post("/databases/_system/query/explain", { "query": "RETURN 1", "bind_vars": "[" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "bind vars must be a JSON object")
    end
  end
end
