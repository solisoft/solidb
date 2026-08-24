describe("UsersController") do
  before_each() do
    as_guest()
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.user("admin_spec_user"))
  end

  describe("GET /users") do
    test("lists users including admin") do
      response = get("/users")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "admin")
    end
  end

  describe("user lifecycle") do
    test("create, grant role, revoke role, delete") do
      # 12 chars minimum, enforced by the server since the security hardening
      response = post("/users", { "username": "admin_spec_user",
                                  "password": "spec-secret-123", "initial_role": "viewer" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "user admin_spec_user created")

      response = post("/users/admin_spec_user/roles", { "role": "editor" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "role editor granted to admin_spec_user")

      response = delete("/users/admin_spec_user/roles/editor")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "role editor revoked from admin_spec_user")

      response = delete("/users/admin_spec_user")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "user admin_spec_user deleted")
    end

    test("rejects missing credentials") do
      response = post("/users", { "username": "", "password": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "username and password are required")
    end

    test("rejects a blank role grant") do
      response = post("/users/admin/roles", { "role": " " })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "role is required")
    end
  end
end
