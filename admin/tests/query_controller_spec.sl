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
      # timing renders in the best-fit unit: µs for sub-ms queries, else ms/s
      assert_match(body, "(µs|ms)")
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
      # Drop first: an interrupted earlier run leaves the scratch db (and its
      # _slow_queries rows) behind, inflating the badge count.
      SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_slow"))
      SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_slow" })
      # SoliDB pre-creates _slow_queries with the database; seed one entry the
      # way the server's slow-query logger writes them.
      SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_slow", "_slow_queries"),
                            { "query": "FOR d IN big RETURN d", "execution_time_ms": 250.5,
                              "timestamp": "2026-06-05T10:00:00Z", "results_count": 9000,
                              "documents_inserted": 0, "documents_updated": 0, "documents_removed": 0,
                              "origin": "gc-worker", "cf_ops_during": 3, "cf_ops_ms_during": 1200.0 })
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
      # origin + cf-churn contention columns from the seeded entry
      assert_contains(body, "gc-worker")
      assert_contains(body, "3 ops")
    end

    test("badge fragment shows the count") do
      # Re-baseline first: under full-suite load the earlier tests' own page
      # reads can be logged as slow queries (CF-churn contention), inflating
      # the count past the entry seeded in before_all.
      SolidbClient.put_api(SolidbEndpoints.collection_truncate("admin_spec_slow", "_slow_queries"))
      SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_slow", "_slow_queries"),
                            { "query": "RETURN 1", "execution_time_ms": 120.0,
                              "timestamp": "2026-06-05T11:00:00Z", "results_count": 1,
                              "documents_inserted": 0, "documents_updated": 0, "documents_removed": 0 })
      response = get("/databases/admin_spec_slow/query/slow/count")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), ">1</span>")
    end

    test("renders dashes for entries logged before origin/cf tracking existed") do
      SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_slow", "_slow_queries"),
                            { "query": "FOR old IN legacy RETURN old", "execution_time_ms": 150.0,
                              "timestamp": "2026-06-04T10:00:00Z", "results_count": 1,
                              "documents_inserted": 0, "documents_updated": 0, "documents_removed": 0 })
      response = get("/databases/admin_spec_slow/query/slow")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "FOR old IN legacy RETURN old")
    end

    test("clear truncates the log and empties the badge") do
      response = put("/databases/admin_spec_slow/query/slow/clear", {})
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "slow query log cleared")
      assert_contains(res_body(response), "no slow queries logged")

      response = get("/databases/admin_spec_slow/query/slow/count")
      assert_eq(res_status(response), 200)
      assert_not(res_body(response).includes?("text-red-400"))
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
