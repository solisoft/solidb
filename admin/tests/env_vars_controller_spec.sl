# Env vars against a scratch database created per suite.
describe("EnvVarsController") do
  before_all() do
    SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_env" })
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_env"))
  end

  before_each() do
    as_guest()
  end

  describe("GET /databases/:db/env") do
    test("renders the env var list") do
      response = get("/databases/admin_spec_env/env")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "Env vars")
    end
  end

  describe("env var lifecycle") do
    test("set, overwrite, delete") do
      response = post("/databases/admin_spec_env/env",
                      { "env_key": "SPEC_API_KEY", "env_value": "v1" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "env var SPEC_API_KEY saved")
      assert_contains(res_body(response), "v1")

      # PUT /env/{key} is an upsert -- saving the same key overwrites.
      response = post("/databases/admin_spec_env/env",
                      { "env_key": "SPEC_API_KEY", "env_value": "v2" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "v2")

      response = delete("/databases/admin_spec_env/env/SPEC_API_KEY")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "env var deleted")
    end

    test("rejects a blank key") do
      response = post("/databases/admin_spec_env/env", { "env_key": "", "env_value": "x" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "key is required")
    end
  end
end
