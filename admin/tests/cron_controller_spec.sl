# Cron jobs against a scratch database created per suite.
describe("CronController") do
  before_all() do
    SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_cron" })
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_cron"))
  end

  before_each() do
    as_guest()
  end

  describe("GET /databases/:db/cron") do
    test("renders the cron list") do
      response = get("/databases/admin_spec_cron/cron")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "Cron jobs")
    end
  end

  describe("cron lifecycle") do
    test("create, update, delete") do
      response = post("/databases/admin_spec_cron/cron",
                      { "cron_name": "spec_nightly", "cron_expression": "0 0 2 * * *",
                        "script": "cleanup", "queue": "default",
                        "priority": "1", "max_retries": "2", "job_params": "{\"days\": 30}" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "cron job spec_nightly created")

      list_result = SolidbClient.get_api(SolidbEndpoints.cron_jobs("admin_spec_cron"))
      cron_jobs = list_result["data"] ?? []
      assert_gt(cron_jobs.length(), 0)
      cron_id = cron_jobs[0]["_key"] ?? cron_jobs[0]["id"]

      response = put("/databases/admin_spec_cron/cron/" + cron_id,
                     { "cron_name": "spec_nightly", "cron_expression": "0 0 3 * * *",
                       "script": "cleanup", "job_params": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "cron job spec_nightly updated")

      response = delete("/databases/admin_spec_cron/cron/" + cron_id)
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "cron job deleted")
    end

    test("rejects missing fields") do
      response = post("/databases/admin_spec_cron/cron", { "cron_name": "", "cron_expression": "", "script": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "name, cron expression and a script or webhook target are required")
    end

    test("rejects a job with no script and no webhook") do
      response = post("/databases/admin_spec_cron/cron",
                      { "cron_name": "no_target", "cron_expression": "0 * * * * *", "script": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "a script or webhook target are required")
    end

    test("creates a cron job with a webhook target") do
      response = post("/databases/admin_spec_cron/cron",
                      { "cron_name": "spec_webhook", "cron_expression": "0 0 2 * * *",
                        "script": "", "webhook_url": "https://example.com/hooks/nightly" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "cron job spec_webhook created")
    end

    test("rejects invalid params json") do
      response = post("/databases/admin_spec_cron/cron",
                      { "cron_name": "x", "cron_expression": "0 * * * * *", "script": "s", "job_params": "[" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "params must be a JSON object")
    end
  end
end
