# Lua REPL against a scratch database created per suite.
describe("ReplController") do
  before_all() do
    SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_repl" })
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_repl"))
  end

  before_each() do
    as_guest()
  end

  describe("GET /databases/:db/repl") do
    test("renders the repl page") do
      response = get("/databases/admin_spec_repl/repl")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "Lua REPL")
    end
  end

  describe("POST /databases/:db/repl/eval") do
    test("evaluates an expression") do
      response = post("/databases/admin_spec_repl/repl/eval", { "code": "return 1 + 1" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "= 2")
    end

    test("captures print output") do
      response = post("/databases/admin_spec_repl/repl/eval",
                      { "code": "print('hello_repl') return 5" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "hello_repl")
      assert_contains(body, "= 5")
    end

    test("surfaces lua errors") do
      response = post("/databases/admin_spec_repl/repl/eval", { "code": "this is not lua" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "text-red-400")
    end

    test("rejects empty code") do
      response = post("/databases/admin_spec_repl/repl/eval", { "code": "   " })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "nothing to run")
    end

    test("session state persists across evals") do
      # Drive the SoliDB session API directly: set a variable, read it back
      # with the same session id.
      first_eval = SolidbClient.post_api(SolidbEndpoints.repl("admin_spec_repl"),
                                         { "code": "x = 41 return x" })
      assert_eq(first_eval["ok"], true)
      session_id = (first_eval["data"] ?? {})["session_id"] ?? ""
      assert_not(session_id.blank?)

      second_eval = SolidbClient.post_api(SolidbEndpoints.repl("admin_spec_repl"),
                                          { "code": "return x + 1", "session_id": session_id })
      assert_eq(second_eval["ok"], true)
      assert_eq((second_eval["data"] ?? {})["result"], 42)
    end
  end
end
