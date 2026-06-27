# Enqueue `count` future-dated (so they stay pending) jobs on a queue.
def seed_spec_jobs(queue, count)
  for n in range(0, count)
    post("/databases/admin_spec_queues/queues/enqueue",
         { "queue": queue, "script": "spec_page_script",
           "run_at": "2030-01-01T00:00:00Z" })
  end
end

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
      # Details panel: full job id, params payload, and metadata labels.
      assert_contains(res_body(response), job_id)
      assert_contains(res_body(response), "retries")
      assert_contains(res_body(response), "run at")
      assert_contains(res_body(response), "params")
      assert_contains(res_body(response), "0 / 1")

      # Cancel refreshes only the queue's jobs fragment in place (the button
      # carries ?queue=) — no full-page reload, so no layout in the response.
      response = delete("/databases/admin_spec_queues/queues/jobs/" + job_id + "?queue=spec_queue")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "job cancelled")
      assert_not(res_body(response).includes?("<!DOCTYPE html>"))
    end

    test("run-now reschedules a pending job") do
      post("/databases/admin_spec_queues/queues/enqueue",
           { "queue": "spec_runs", "script": "spec_script", "job_params": "",
             "run_at": "2030-01-01T00:00:00Z" })
      jobs_result = SolidbClient.get_api(SolidbEndpoints.queue_jobs("admin_spec_queues", "spec_runs"))
      jobs = (jobs_result["data"] ?? {})["jobs"] ?? []
      assert_gt(jobs.length(), 0)

      # Run-now also refreshes the queue's jobs fragment in place.
      response = post("/databases/admin_spec_queues/queues/jobs/" + jobs[0]["_key"] + "/run-now?queue=spec_runs", {})
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "job scheduled to run now")
      assert_not(res_body(response).includes?("<!DOCTYPE html>"))
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

  describe("QueuesView.normalize") do
    # Older SoliDB builds return queues positionally as arrays instead of stat
    # objects; the view indexes queue["name"], so a raw array entry would 500
    # the page. QueuesView.normalize coerces every shape to a uniform hash.
    test("passes modern stat objects through unchanged") do
      queues = QueuesView.normalize(
        [{ "name": "default", "pending": 3, "running": 1, "completed": 2, "failed": 0 }])
      assert_eq(queues[0]["name"], "default")
      assert_eq(queues[0]["pending"], 3)
    end

    test("coerces legacy [name, stats] array pairs") do
      # Build via JSON.parse to mirror the real API body (a parsed array).
      queues = QueuesView.normalize(
        JSON.parse("[[\"qa\", {\"pending\": 5, \"running\": 0, \"completed\": 1, \"failed\": 2}]]"))
      assert_eq(queues[0]["name"], "qa")
      assert_eq(queues[0]["pending"], 5)
      assert_eq(queues[0]["failed"], 2)
    end

    test("degrades unknown positional arrays to zeroed counts, never a crash") do
      queues = QueuesView.normalize(JSON.parse("[[\"qb\", 7, 8, 9, 10, 11]]"))
      assert_eq(queues[0]["name"], "qb")
      assert_eq(queues[0]["pending"], 0)
    end

    test("returns an empty list for a non-array payload") do
      assert_eq(QueuesView.normalize(null).length(), 0)
    end
  end

  describe("jobs filtering and pagination") do
    # 26 pending jobs so a page of 25 leaves a second page to navigate to.
    before_all() do
      seed_spec_jobs("spec_page", 26)
    end

    test("renders the status filter bar") do
      response = get("/databases/admin_spec_queues/queues/spec_page/jobs")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      # Each status renders a pill whose hx-get carries the status param.
      assert_contains(body, "status=pending")
      assert_contains(body, "status=completed")
      assert_contains(body, "status=failed")
      assert_contains(body, "per page")
      # Free-text search box for filtering the loaded rows client-side.
      assert_contains(body, "type=\"search\"")
    end

    test("limit caps the page and surfaces a Next control") do
      response = get("/databases/admin_spec_queues/queues/spec_page/jobs?limit=25")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      # 26 jobs total, 25 per page → a next page exists at offset=25.
      assert_contains(body, "of 26")
      assert_contains(body, "Next")
      assert_contains(body, "offset=25")
    end

    test("an out-of-whitelist limit falls back to the default") do
      response = get("/databases/admin_spec_queues/queues/spec_page/jobs?limit=2")
      assert_eq(res_status(response), 200)
      # 2 is rejected → default 50 → all 26 fit on one page, no Next control.
      assert_contains(res_body(response), "of 26")
      assert_not(res_body(response).includes?("Next"))
    end

    test("offset pages forward and surfaces a Prev control") do
      response = get("/databases/admin_spec_queues/queues/spec_page/jobs?limit=25&offset=25")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "Prev")
      assert_contains(body, "offset=0")
    end

    test("status filter narrows the listing") do
      # All jobs are pending, so completed yields an empty, filter-aware message.
      response = get("/databases/admin_spec_queues/queues/spec_page/jobs?status=completed")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "no completed jobs in spec_page")

      response = get("/databases/admin_spec_queues/queues/spec_page/jobs?status=pending")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "spec_page_script")
    end

    test("an unknown status falls back to no filter") do
      response = get("/databases/admin_spec_queues/queues/spec_page/jobs?status=bogus")
      assert_eq(res_status(response), 200)
      # Falls back to all statuses, so the pending jobs are listed.
      assert_contains(res_body(response), "spec_page_script")
    end
  end

  describe("queue settings") do
    test("saves settings and surfaces them on the queue card") do
      response = post("/databases/admin_spec_queues/queues/spec_settings/settings",
                      { "paused": "1", "concurrency": "3", "default_priority": "7" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "settings saved for spec_settings")
      # The queue had no jobs, but a configured queue still shows up in the
      # list with its settings rendered (the "max N" badge only renders when
      # concurrency > 0, so it uniquely marks spec_settings).
      assert_contains(res_body(response), "max 3")
    end

    test("clamps a negative concurrency to 0 (unlimited)") do
      post("/databases/admin_spec_queues/queues/spec_clamp/settings",
           { "concurrency": "-5", "default_priority": "0" })
      config = SolidbClient.get_api(SolidbEndpoints.queues("admin_spec_queues"))
      queues = QueuesView.normalize(config["data"] ?? [])
      clamp = queues.filter do |queue| queue["name"] == "spec_clamp" end
      assert_eq(clamp.length(), 1)
      assert_eq(clamp[0]["concurrency"], 0)
    end

    test("applies the queue default priority to jobs enqueued without one") do
      SolidbClient.put_api(SolidbEndpoints.queue_config("admin_spec_queues", "spec_prio"),
                           { "default_priority": 9 })
      post("/databases/admin_spec_queues/queues/enqueue",
           { "queue": "spec_prio", "script": "spec_script",
             "run_at": "2030-01-01T00:00:00Z" })
      jobs_result = SolidbClient.get_api(SolidbEndpoints.queue_jobs("admin_spec_queues", "spec_prio"))
      jobs = (jobs_result["data"] ?? {})["jobs"] ?? []
      assert_gt(jobs.length(), 0)
      assert_eq(jobs[0]["priority"], 9)
    end

    test("renders the settings editor on the index") do
      seed_spec_jobs("spec_form", 1)
      response = get("/databases/admin_spec_queues/queues")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "Save settings")
      assert_contains(body, "max concurrency")
      assert_contains(body, "default priority")
    end
  end
end
