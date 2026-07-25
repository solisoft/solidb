describe("DatabasesController") do
  before_each() do
    as_guest()
  end

  describe("GET /databases") do
    test("lists databases including _system") do
      response = get("/databases")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "_system")
    end
  end

  describe("POST /databases + DELETE /databases/:db") do
    test("creates then drops a database (full-page round-trip)") do
      response = post("/databases", { "name": "admin_spec_scratch" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "database admin_spec_scratch created")
      assert_contains(body, "admin_spec_scratch/collections")

      # confirm_name is sent as a query param (DELETE body is not always parsed).
      response = delete("/databases/admin_spec_scratch?confirm_name=admin_spec_scratch")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "database admin_spec_scratch dropped")
      assert_not(body.includes?("admin_spec_scratch/collections"))
    end

    test("rejects drop without matching confirm_name") do
      response = post("/databases", { "name": "admin_spec_confirm" })
      assert_eq(res_status(response), 200)

      response = delete("/databases/admin_spec_confirm")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "type the database name")
      assert_contains(body, "admin_spec_confirm/collections")

      response = delete("/databases/admin_spec_confirm?confirm_name=wrong_name")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "type the database name")

      response = delete("/databases/admin_spec_confirm?confirm_name=admin_spec_confirm")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "database admin_spec_confirm dropped")
    end

    test("rejects a blank name with a flash error") do
      response = post("/databases", { "name": "  " })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "database name is required")
    end

    test("HTMX requests get the layout-free fragment branch") do
      response = request("POST", "/databases", { "name": "" },
                         { "headers": { "HX-Request": "true" } })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "database name is required")
      assert_not(body.includes?("<!DOCTYPE html>"))
    end
  end
end
