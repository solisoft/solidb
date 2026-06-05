describe("ApiKeysController") do
  before_each() do
    as_guest()
  end

  describe("GET /api-keys") do
    test("renders the key list") do
      response = get("/api-keys")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "API Keys")
    end
  end

  describe("key lifecycle") do
    test("create shows the raw key once, then revoke") do
      response = post("/api-keys", { "name": "admin_spec_key",
                                     "roles": "viewer",
                                     "scoped_databases": "" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "api key admin_spec_key created")
      assert_contains(body, "raw key")

      list_result = SolidbClient.get_api(SolidbEndpoints.api_keys())
      keys = (list_result["data"] ?? {})["keys"] ?? []
      spec_keys = keys.filter do |key| key["name"] == "admin_spec_key" end
      assert_gt(spec_keys.length(), 0)

      response = delete("/api-keys/" + spec_keys[0]["id"])
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "api key revoked")
    end

    test("rejects a blank name") do
      response = post("/api-keys", { "name": "", "roles": "", "scoped_databases": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "key name is required")
    end
  end
end
