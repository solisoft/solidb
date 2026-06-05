# Scripts CRUD against a scratch database created per suite.
describe("ScriptsController") do
  before_all() do
    SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_scripts" })
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_scripts"))
  end

  before_each() do
    as_guest()
  end

  describe("GET /databases/:db/scripts") do
    test("renders the empty list") do
      response = get("/databases/admin_spec_scripts/scripts")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "Lua scripts")
    end

    test("renders the new-script form") do
      response = get("/databases/admin_spec_scripts/scripts/new")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "lua code")
    end
  end

  describe("script lifecycle") do
    test("create, show, edit, update, delete") do
      response = post("/databases/admin_spec_scripts/scripts",
                      { "name": "hello", "path": "hello", "service": "demo",
                        "description": "spec script", "method_get": "on",
                        "code": "return { message = 'hi' }" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "script hello created")

      list_result = SolidbClient.get_api(SolidbEndpoints.scripts("admin_spec_scripts"))
      scripts = (list_result["data"] ?? {})["scripts"] ?? []
      assert_gt(scripts.length(), 0)
      script_id = scripts[0]["id"] ?? scripts[0]["_key"]

      response = get("/databases/admin_spec_scripts/scripts/" + script_id)
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "return { message = ")

      response = get("/databases/admin_spec_scripts/scripts/" + script_id + "/edit")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "Save script")

      response = put("/databases/admin_spec_scripts/scripts/" + script_id,
                     { "name": "hello", "path": "hello", "service": "demo",
                       "description": "updated", "method_get": "on", "method_post": "on",
                       "code": "return { message = 'updated' }" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "script hello updated")

      response = delete("/databases/admin_spec_scripts/scripts/" + script_id)
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "script deleted")
    end

    test("rejects a script without name or path") do
      response = post("/databases/admin_spec_scripts/scripts", { "name": "", "path": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "name and path are required")
    end
  end
end
