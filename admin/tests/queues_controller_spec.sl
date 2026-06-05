# Queues against a scratch database created per suite.
describe("QueuesController") do
  before_all() do
    SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_queues" })
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_queues"))
  end

  before_each() do
    as_guest()
  end

  describe("GET /databases/:db/queues") do
    test("renders queue stats") do
      response = get("/databases/admin_spec_queues/queues")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "Queues")
    end
  end

  describe("job lifecycle") do
    test("enqueue then cancel a job") do
      response = post("/databases/admin_spec_queues/queues/enqueue",
                      { "queue": "spec_queue", "script": "spec_script",
                        "job_params": "{\"k\": 1}", "priority": "5", "max_retries": "1",
                        "run_at": "2030-01-01T00:00:00Z" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "job enqueued on spec_queue")

      jobs_result = SolidbClient.get_api(SolidbEndpoints.queue_jobs("admin_spec_queues", "spec_queue"))
      jobs = (jobs_result["data"] ?? {})["jobs"] ?? []
      assert_gt(jobs.length(), 0)
      job_id = jobs[0]["_key"]

      response = get("/databases/admin_spec_queues/queues/spec_queue/jobs")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "spec_script")

      response = delete("/databases/admin_spec_queues/queues/jobs/" + job_id)
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "job cancelled")
    end

    test("run-now reschedules a pending job") do
      post("/databases/admin_spec_queues/queues/enqueue",
           { "queue": "spec_runs", "script": "spec_script", "job_params": "",
             "run_at": "2030-01-01T00:00:00Z" })
      jobs_result = SolidbClient.get_api(SolidbEndpoints.queue_jobs("admin_spec_queues", "spec_runs"))
      jobs = (jobs_result["data"] ?? {})["jobs"] ?? []
      assert_gt(jobs.length(), 0)

      response = post("/databases/admin_spec_queues/queues/jobs/" + jobs[0]["_key"] + "/run-now", {})
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "job scheduled to run now")
    end

    test("rejects a job without a script") do
      response = post("/databases/admin_spec_queues/queues/enqueue", { "queue": "q", "script": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "script is required")
    end

    test("rejects invalid params json") do
      response = post("/databases/admin_spec_queues/queues/enqueue",
                      { "queue": "q", "script": "s", "job_params": "{nope" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "params must be a JSON object")
    end
  end
end
