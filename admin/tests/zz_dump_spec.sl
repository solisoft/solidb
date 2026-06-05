# Throwaway debug spec - prints the create-view response body.
describe("DumpMaterializedViewCreate") do
  before_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_dump"))
    SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_dump" })
    SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_dump"), { "name": "people" })
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_dump"))
  end

  test("dump create result") do
    create_result = SolidbClient.post_api(
      SolidbEndpoints.cursor("admin_spec_dump"),
      { "query": "CREATE MATERIALIZED VIEW vp9 AS FOR p IN people RETURN p" })
    print("DUMP-ok: " + str(create_result["ok"]))
    print("DUMP-status: " + str(create_result["status"]))
    print("DUMP-error: " + str(create_result["error"]))
    print("DUMP-data: " + str(create_result["data"]))
    assert_eq(create_result["ok"], true)
  end

  test("dump controller response banners") do
    response = post("/databases/admin_spec_dump/views",
                    { "name": "vp3", "query": "FOR p IN people RETURN p" })
    body = res_body(response)
    print("DUMP-has-unreachable: " + str(body.includes?("SoliDB unreachable")))
    print("DUMP-has-error-banner: " + str(body.includes?("text-red-200")))
    print("DUMP-has-ok-banner: " + str(body.includes?("text-teal-100")))
    start_index = body.index_of("text-red-200") ?? -1
    if start_index >= 0
      print("DUMP-error-text: " + body.substring(start_index, start_index + 200))
    end
    assert_contains(body, "view vp3 created")
  end
end
