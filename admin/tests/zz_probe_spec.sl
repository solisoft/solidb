# TEMPORARY bisection probe - delete after debugging.
describe("cursor statement probe") do
  test("bisect which statements fail at the HTTP layer") do
    statements = [
      "FOR c IN companies LIMIT 1 RETURN c",
      "CREATE MATERIALIZED VIEW zz1 AS FOR c IN companies RETURN c",
      "REFRESH MATERIALIZED VIEW nonexistent",
      "create materialized view zz2 as return 1",
      "CREATE MATERIALIZED",
      "MATERIALIZED VIEW",
      "CREATE VIEW zz3 AS RETURN 1",
      "XCREATE MATERIALIZED VIEW zz4 AS RETURN 1"
    ]
    outcomes = []
    for stmt in statements
      result = SolidbClient.post_api(SolidbEndpoints.cursor("alu"), { "query": stmt })
      outcomes.push({ "stmt": stmt.substring(0, 40), "status": result["status"], "err": result["error"] })
    end
    SolidbClient.post_api(SolidbEndpoints.collections("alu"), { "name": "debug_dump" })
    SolidbClient.post_api(SolidbEndpoints.documents("alu", "debug_dump"), { "outcomes": outcomes })
    assert_eq(1, 1)
  end
end
