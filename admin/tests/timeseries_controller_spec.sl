# Exercises the timeseries explorer against a scratch database: field
# discovery, bucketed chart data, raw points, TTL indexes and pruning.
#
# Seeded layout (all in "metrics"): two adjacent 5-minute buckets at a FIXED
# absolute time (2026-01-02T00:00Z, aligned) — bucket A holds values 10/20/30,
# bucket B holds 5 — so every aggregation has a hand-computable expectation
# and the custom query window never races a wall-clock bucket boundary.

# 2026-01-02T00:00:00Z in epoch ms; midnight is 5-minute aligned.
def ts_base_ms()
  return 1767312000000
end

# Custom window covering exactly the two seeded buckets.
def ts_seeded_window()
  return "range=custom&from=" + str(ts_base_ms()) + "&to=" + str(ts_base_ms() + 600000)
end

def ts_data_path(query)
  return "/databases/admin_spec_ts/collections/metrics/timeseries/data?" + query
end

describe("TimeseriesController") do
  before_all() do
    SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_ts" })
    SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_ts"),
                          { "name": "metrics", "type": "timeseries" })
    bucket_a = ts_base_ms()
    bucket_b = bucket_a + 300000
    seeds = [
      { "timestamp": bucket_a + 1000, "value": 10, "host": "web01" },
      { "timestamp": bucket_a + 2000, "value": 20, "host": "web02" },
      { "timestamp": bucket_a + 3000, "value": 30, "host": "web01" },
      { "timestamp": bucket_b + 1000, "value": 5, "host": "web02" }
    ]
    for seed in seeds
      SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_ts", "metrics"), seed)
    end
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_ts"))
  end

  before_each() do
    as_guest()
  end

  describe("GET /timeseries (page)") do
    test("renders the explorer with discovered fields") do
      response = get("/databases/admin_spec_ts/collections/metrics/timeseries")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "timeseries")
      assert_contains(body, "ts-chart")
      assert_contains(body, "retention")
      # Field discovery feeds the toolbar: "value" is a value-field option,
      # "host" a group-by option, and the ttl form defaults to the time field.
      assert_contains(body, "<option value=\"value\">value</option>")
      assert_contains(body, "<option value=\"host\">host</option>")
      assert_contains(body, "value=\"timestamp\"")
    end

    test("shows the empty state for a collection without documents") do
      SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_ts"),
                            { "name": "ts_empty", "type": "timeseries" })
      response = get("/databases/admin_spec_ts/collections/ts_empty/timeseries")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "no documents yet")
      SolidbClient.delete_api(SolidbEndpoints.collection("admin_spec_ts", "ts_empty"))
    end

    test("redirects non-timeseries collections to the docs browser") do
      SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_ts"), { "name": "plain_docs" })
      response = get("/databases/admin_spec_ts/collections/plain_docs/timeseries")
      assert_eq(res_status(response), 302)
      assert_contains(res_location(response), "/docs")
      SolidbClient.delete_api(SolidbEndpoints.collection("admin_spec_ts", "plain_docs"))
    end
  end

  describe("GET /timeseries/data") do
    test("returns aligned uPlot arrays with gap-filled buckets") do
      response = get(ts_data_path(ts_seeded_window() + "&bucket=5m&agg=avg&value_field=value"))
      assert_eq(res_status(response), 200)
      data = res_json(response)
      assert_eq(data["ok"], true)
      assert_eq(data["bucket"], "5m")
      assert_eq(data["capped"], false)
      assert_eq(data["series"], ["time", "value"])
      assert_eq(data["data"].length(), 2)
      xs = data["data"][0]
      values = data["data"][1]
      assert_eq(xs.length(), 2)
      assert_eq(values.length(), 2)
      assert_eq(xs[1] - xs[0], 300)
      assert_eq(int(values[0]), 20)
      assert_eq(int(values[1]), 5)
    end

    test("supports min, max, sum and count aggregations") do
      window = ts_seeded_window()
      response = get(ts_data_path(window + "&bucket=5m&agg=min&value_field=value"))
      assert_eq(int(res_json(response)["data"][1][0]), 10)

      response = get(ts_data_path(window + "&bucket=5m&agg=max&value_field=value"))
      assert_eq(int(res_json(response)["data"][1][0]), 30)

      response = get(ts_data_path(window + "&bucket=5m&agg=sum&value_field=value"))
      assert_eq(int(res_json(response)["data"][1][0]), 60)

      response = get(ts_data_path(window + "&bucket=5m&agg=count"))
      data = res_json(response)
      assert_eq(data["series"], ["time", "count"])
      assert_eq(int(data["data"][1][0]), 3)
      assert_eq(int(data["data"][1][1]), 1)
    end

    test("splits series with group_by") do
      response = get(ts_data_path(ts_seeded_window() +
                                  "&bucket=5m&agg=avg&value_field=value&group_by=host"))
      data = res_json(response)
      assert_eq(data["ok"], true)
      assert_contains(data["series"], "web01")
      assert_contains(data["series"], "web02")
      assert_eq(data["series_capped"], false)
      assert_eq(data["data"].length(), 3)
      # web01 averaged (10+30)/2=20 in bucket A and has a null gap in bucket B.
      web01_index = data["series"].index_of("web01")
      web01 = data["data"][web01_index]
      assert_eq(int(web01[0]), 20)
      assert_null(web01[1])
    end

    test("widens the bucket when the range would exceed the point cap") do
      response = get(ts_data_path("range=30d&bucket=1m&agg=avg&value_field=value"))
      data = res_json(response)
      assert_eq(data["ok"], true)
      assert_eq(data["capped"], true)
      assert_eq(data["bucket"], "1h")
    end

    test("rejects invalid params with 422") do
      response = get(ts_data_path("range=1h&agg=avg&value_field=nope"))
      assert_eq(res_status(response), 422)
      assert_eq(res_json(response)["ok"], false)

      response = get(ts_data_path("range=fortnight&agg=avg&value_field=value"))
      assert_eq(res_status(response), 422)

      response = get(ts_data_path("range=custom&from=900&to=500&agg=avg&value_field=value"))
      assert_eq(res_status(response), 422)

      response = get(ts_data_path("range=1h&agg=median&value_field=value"))
      assert_eq(res_status(response), 422)

      response = get(ts_data_path("range=1h&agg=avg&value_field=value&group_by=nope"))
      assert_eq(res_status(response), 422)

      response = get(ts_data_path("range=custom&from=garbage&to=alsogarbage&agg=avg&value_field=value"))
      assert_eq(res_status(response), 422)
    end

    test("rejects an empty collection with 422") do
      SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_ts"),
                            { "name": "ts_blank", "type": "timeseries" })
      response = get("/databases/admin_spec_ts/collections/ts_blank/timeseries/data?range=1h")
      assert_eq(res_status(response), 422)
      assert_contains(res_json(response)["error"], "no documents")
      SolidbClient.delete_api(SolidbEndpoints.collection("admin_spec_ts", "ts_blank"))
    end
  end

  describe("alternate timestamp encodings") do
    test("buckets ISO8601 string timestamps") do
      SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_ts"),
                            { "name": "ts_iso", "type": "timeseries" })
      SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_ts", "ts_iso"),
                            { "timestamp": "2026-01-01T00:01:00Z", "value": 7 })
      SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_ts", "ts_iso"),
                            { "timestamp": "2026-01-01T00:02:00Z", "value": 9 })
      from_ms = DateTime.parse("2026-01-01T00:00:00Z").to_unix() * 1000
      query = "range=custom&from=" + str(from_ms) + "&to=" + str(from_ms + 300000) +
              "&bucket=5m&agg=avg&value_field=value"
      response = get("/databases/admin_spec_ts/collections/ts_iso/timeseries/data?" + query)
      data = res_json(response)
      assert_eq(data["ok"], true)
      assert_eq(int(data["data"][1][0]), 8)
      SolidbClient.delete_api(SolidbEndpoints.collection("admin_spec_ts", "ts_iso"))
    end

    test("buckets epoch-second timestamps") do
      SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_ts"),
                            { "name": "ts_secs", "type": "timeseries" })
      base_seconds = DateTime.parse("2026-01-01T00:00:00Z").to_unix()
      SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_ts", "ts_secs"),
                            { "timestamp": base_seconds + 60, "value": 4 })
      query = "range=custom&from=" + str(base_seconds * 1000) +
              "&to=" + str(base_seconds * 1000 + 300000) + "&bucket=5m&agg=avg&value_field=value"
      response = get("/databases/admin_spec_ts/collections/ts_secs/timeseries/data?" + query)
      data = res_json(response)
      assert_eq(data["ok"], true)
      assert_eq(int(data["data"][1][0]), 4)
      SolidbClient.delete_api(SolidbEndpoints.collection("admin_spec_ts", "ts_secs"))
    end
  end

  describe("GET /timeseries/points") do
    test("lists newest-first inside the window") do
      response = get("/databases/admin_spec_ts/collections/metrics/timeseries/points?" +
                     ts_seeded_window())
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "points 1–4")
      assert_contains(body, "web01")
      assert_not(body.includes?("Next →"))
    end

    test("paginates with offset") do
      response = get("/databases/admin_spec_ts/collections/metrics/timeseries/points?" +
                     ts_seeded_window() + "&offset=2")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "points 3–4")
      assert_contains(res_body(response), "← Prev")
    end
  end

  describe("retention") do
    test("creates, lists and drops a ttl index") do
      response = post("/databases/admin_spec_ts/collections/metrics/timeseries/ttl",
                      { "field": "timestamp", "expire_after_seconds": "86400" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "ttl index ttl_timestamp created")
      assert_contains(body, "ttl_timestamp")

      response = delete("/databases/admin_spec_ts/collections/metrics/timeseries/ttl/ttl_timestamp")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "ttl index ttl_timestamp dropped")
    end

    test("rejects a ttl index without a field or expiry") do
      response = post("/databases/admin_spec_ts/collections/metrics/timeseries/ttl",
                      { "field": "", "expire_after_seconds": "86400" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "ttl field is required")

      response = post("/databases/admin_spec_ts/collections/metrics/timeseries/ttl",
                      { "field": "timestamp", "expire_after_seconds": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "must be a positive integer")
    end

    test("prunes points inserted before a custom cutoff") do
      SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_ts"),
                            { "name": "ts_prune", "type": "timeseries" })
      for value in [1, 2, 3]
        SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_ts", "ts_prune"),
                              { "timestamp": ts_base_ms(), "value": value })
      end
      # Prune cuts by INSERT time (UUIDv7 key), so a cutoff just past "now"
      # catches all three seeds regardless of their timestamp field.
      cutoff = DateTime.from_unix(DateTime.now().to_unix() + 60).to_iso()
      response = post("/databases/admin_spec_ts/collections/ts_prune/timeseries/prune",
                      { "older_than_preset": "custom", "older_than": cutoff })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "pruned 3 point(s)")
      SolidbClient.delete_api(SolidbEndpoints.collection("admin_spec_ts", "ts_prune"))
    end

    test("accepts a relative prune preset") do
      # 90 days back predates every insert, so nothing is deleted - the
      # interesting part is the preset -> ISO conversion succeeding.
      response = post("/databases/admin_spec_ts/collections/metrics/timeseries/prune",
                      { "older_than_preset": "90d" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "pruned 0 point(s)")
    end

    test("rejects an unparseable prune cutoff") do
      response = post("/databases/admin_spec_ts/collections/metrics/timeseries/prune",
                      { "older_than_preset": "custom", "older_than": "not-a-date" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "prune needs a cutoff")
    end
  end

  describe("endpoint builders") do
    test("prune and ttl-index paths") do
      assert_eq(SolidbEndpoints.collection_prune("db1", "metrics"),
                "/_api/database/db1/collection/metrics/prune")
      assert_eq(SolidbEndpoints.ttl_index("db1", "metrics", "ttl_ts"),
                "/_api/database/db1/ttl/metrics/ttl_ts")
    end
  end
end
