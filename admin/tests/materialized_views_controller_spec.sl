# Exercises materialized views CRUD against a scratch database with a
# seeded source collection.
describe("MaterializedViewsController") do
  before_all() do
    # Drop first: a crashed earlier run can leave the scratch db behind with
    # the view already created, and CREATE MATERIALIZED VIEW has no
    # IF NOT EXISTS here - leftovers turn every later run into a 409.
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_views"))
    SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_views" })
    SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_views"), { "name": "people" })
    SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_views", "people"),
                          { "name": "ada", "active": true })
    SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_views", "people"),
                          { "name": "bob", "active": false })
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_views"))
  end

  before_each() do
    as_guest()
  end

  describe("GET /databases/:db/views") do
    test("renders the empty view list") do
      response = get("/databases/admin_spec_views/views")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "Materialized views")
    end

    test("create modal wires the query field to the Monaco SDBQL editor") do
      response = get("/databases/admin_spec_views/views")
      body = res_body(response)
      # Mounted explicitly via x-effect when the modal opens (autoMount would
      # mis-measure inside the hidden, teleported <template>).
      assert_contains(body, "x-ref=\"queryEditor\"")
      assert_contains(body, "language: 'sdbql'")
      # Collection names are handed to the editor for autocompletion.
      assert_contains(body, "data-editor-collections")
      assert_contains(body, "people")
    end
  end

  describe("view lifecycle") do
    test("create, list with definition, refresh, drop") do
      response = post("/databases/admin_spec_views/views",
                      { "name": "active_people",
                        "query": "FOR p IN people FILTER p.active == true RETURN p" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "view active_people created")
      assert_contains(body, "active_people")
      # the original SDBQL source is annotated onto the metadata and displayed
      assert_contains(body, "FILTER p.active == true")

      response = put("/databases/admin_spec_views/views/active_people/refresh", {})
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "view active_people refreshed (1 documents)")

      response = delete("/databases/admin_spec_views/views/active_people")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "view active_people dropped")

      # backing collection is gone too
      coll_result = SolidbClient.get_api(SolidbEndpoints.collection("admin_spec_views", "active_people"))
      assert_not(coll_result["ok"])
    end

    test("rejects a blank or invalid view name") do
      response = post("/databases/admin_spec_views/views",
                      { "name": "", "query": "FOR p IN people RETURN p" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "view name must be an identifier")

      response = post("/databases/admin_spec_views/views",
                      { "name": "bad name; REMOVE", "query": "FOR p IN people RETURN p" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "view name must be an identifier")

      response = post("/databases/admin_spec_views/views",
                      { "name": "1starts_with_digit", "query": "FOR p IN people RETURN p" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "view name must be an identifier")
    end

    test("rejects a blank query") do
      response = post("/databases/admin_spec_views/views", { "name": "no_query", "query": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "view query is required")
    end

    test("surfaces a server error for an invalid query") do
      response = post("/databases/admin_spec_views/views",
                      { "name": "broken_view", "query": "THIS IS NOT SDBQL" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "HTTP 4")
    end

    test("refreshing an unknown view surfaces the error") do
      response = put("/databases/admin_spec_views/views/ghost_view/refresh", {})
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "HTTP 4")
    end

    test("dropping an unknown view surfaces the error") do
      response = delete("/databases/admin_spec_views/views/ghost_view")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "HTTP 4")
    end
  end
end
