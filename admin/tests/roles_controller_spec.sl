describe("RolesController") do
  before_each() do
    as_guest()
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.role("spec_custom_role"))
  end

  describe("GET /roles") do
    test("lists builtin roles") do
      response = get("/roles")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "admin")
      assert_contains(body, "editor")
      assert_contains(body, "viewer")
      assert_contains(body, "builtin")
    end
  end

  describe("GET /roles/:name") do
    test("shows a builtin role read-only") do
      response = get("/roles/viewer")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "builtin roles are read-only")
    end
  end

  describe("custom role lifecycle") do
    test("create, show, update, delete") do
      permissions_json = "[{\"action\": \"read\", \"scope\": \"global\"}]"
      response = post("/roles", { "name": "spec_custom_role",
                                  "description": "spec role",
                                  "permissions": permissions_json })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "role spec_custom_role created")

      response = get("/roles/spec_custom_role")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "spec role")

      response = put("/roles/spec_custom_role", { "description": "updated spec role",
                                                 "permissions": permissions_json })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "role spec_custom_role updated")

      response = delete("/roles/spec_custom_role")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "role spec_custom_role deleted")
    end

    test("rejects a blank name") do
      response = post("/roles", { "name": "", "description": "", "permissions": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "role name is required")
    end

    test("rejects invalid permissions json") do
      response = post("/roles", { "name": "broken", "description": "", "permissions": "{nope" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "permissions must be a JSON array")
    end
  end
end
