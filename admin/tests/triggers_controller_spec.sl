# Triggers against a scratch database created per suite. Triggers require an
# existing target collection, so one is created alongside the database.
describe("TriggersController") do
  before_all() do
    SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_triggers" })
    SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_triggers"), { "name": "orders" })
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_triggers"))
  end

  before_each() do
    as_guest()
  end

  describe("GET /databases/:db/triggers") do
    test("renders the trigger list") do
      response = get("/databases/admin_spec_triggers/triggers")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "Triggers")
    end
  end

  describe("trigger lifecycle") do
    test("create, toggle, delete") do
      response = post("/databases/admin_spec_triggers/triggers",
                      { "trigger_name": "spec_audit", "collection": "orders",
                        "event_insert": "1", "event_update": "1",
                        "script_path": "audit_orders", "queue": "default",
                        "priority": "1", "max_retries": "2",
                        "filter": "doc.status == 'paid'" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "trigger spec_audit created")

      list_result = SolidbClient.get_api(SolidbEndpoints.triggers("admin_spec_triggers"))
      triggers = (list_result["data"] ?? {})["triggers"] ?? []
      assert_gt(triggers.length(), 0)
      trigger_id = triggers[0]["_key"] ?? triggers[0]["id"]

      response = post("/databases/admin_spec_triggers/triggers/" + trigger_id + "/toggle", {})
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "trigger disabled")

      response = post("/databases/admin_spec_triggers/triggers/" + trigger_id + "/toggle", {})
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "trigger enabled")

      response = delete("/databases/admin_spec_triggers/triggers/" + trigger_id)
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "trigger deleted")
    end

    test("creates a webhook trigger") do
      response = post("/databases/admin_spec_triggers/triggers",
                      { "trigger_name": "spec_hook", "collection": "orders",
                        "event_delete": "1",
                        "webhook_url": "https://example.com/hooks/orders",
                        "webhook_secret": "s3cret" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "trigger spec_hook created")
    end

    test("rejects missing fields") do
      response = post("/databases/admin_spec_triggers/triggers",
                      { "trigger_name": "", "collection": "", "script_path": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "a script or webhook target are required")
    end

    test("rejects a trigger with no events") do
      response = post("/databases/admin_spec_triggers/triggers",
                      { "trigger_name": "spec_no_events", "collection": "orders",
                        "script_path": "audit_orders" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "at least one event")
    end

    test("rejects an unknown collection") do
      response = post("/databases/admin_spec_triggers/triggers",
                      { "trigger_name": "spec_bad_coll", "collection": "nope",
                        "event_insert": "1", "script_path": "audit_orders" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "does not exist")
    end
  end
end
