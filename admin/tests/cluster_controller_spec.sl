# Cluster monitoring against the live SoliDB node.
describe("ClusterController") do
  before_each() do
    as_guest()
  end

  describe("GET /cluster") do
    test("renders the cluster page with node stats") do
      response = get("/cluster")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "Cluster")
      assert_contains(body, "Uptime")
      assert_contains(body, "Sync log")
    end
  end

  describe("GET /cluster/panel") do
    test("returns the polled fragment without layout") do
      response = get("/cluster/panel")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "Memory")
      # <main> only exists in the application layout (the dev bar injects an
      # <aside> into every response, so that tag can't be the discriminator).
      assert_not(body.includes?("<main"))
    end
  end

  describe("POST /cluster/sync-log/prune") do
    test("prunes up to the current sequence") do
      response = post("/cluster/sync-log/prune", {})
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "sync log pruned")
    end
  end
end
