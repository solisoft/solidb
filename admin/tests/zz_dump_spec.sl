# Throwaway debug spec - bisect which cursor payloads fail.
describe("DumpCursorBisect") do
  before_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_dump"))
    SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_dump" })
    SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_dump"), { "name": "people" })
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_dump"))
  end

  test("plain return query works") do
    result = SolidbClient.post_api(SolidbEndpoints.cursor("admin_spec_dump"), { "query": "RETURN 1" })
    assert_eq(result["ok"], true)
  end

  test("long but fast query works") do
    long_query = "FOR p IN people FILTER p.name != 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxx' RETURN p"
    result = SolidbClient.post_api(SolidbEndpoints.cursor("admin_spec_dump"), { "query": long_query })
    assert_eq(result["ok"], true)
  end

  test("cpu heavy query works") do
    heavy = "FOR i IN 1..3000000 COLLECT WITH COUNT INTO c RETURN c"
    result = SolidbClient.post_api(SolidbEndpoints.cursor("admin_spec_dump"), { "query": heavy })
    assert_eq(result["ok"], true)
  end

  test("create materialized view") do
    create_query = "CREATE MATERIALIZED VIEW vpa AS FOR p IN people RETURN p"
    result = SolidbClient.post_api(SolidbEndpoints.cursor("admin_spec_dump"), { "query": create_query })
    assert_eq(result["ok"], true)
  end

  test("create materialized view retry") do
    create_query = "CREATE MATERIALIZED VIEW vpb AS FOR p IN people RETURN p"
    result = SolidbClient.post_api(SolidbEndpoints.cursor("admin_spec_dump"), { "query": create_query })
    assert_eq(result["ok"], true)
  end
end
