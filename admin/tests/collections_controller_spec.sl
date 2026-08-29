# Exercises collections CRUD against a scratch database created per suite.
describe("CollectionsController") do
  before_all() do
    SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_colls" })
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_colls"))
  end

  before_each() do
    as_guest()
  end

  describe("GET /databases/:db/collections") do
    test("renders the collection list") do
      response = get("/databases/admin_spec_colls/collections")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "Collections")
    end
  end

  describe("collection lifecycle") do
    test("create, stats, drop") do
      response = post("/databases/admin_spec_colls/collections", { "name": "specs", "type": "document" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "collection specs created")

      response = get("/databases/admin_spec_colls/collections/specs/stats")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "documents")

      response = delete("/databases/admin_spec_colls/collections/specs")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "collection specs dropped")
    end

    test("creation modal documents each collection kind") do
      response = get("/databases/admin_spec_colls/collections")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "general-purpose JSON documents")
      assert_contains(body, "graph relations between documents")
      assert_contains(body, "binary files (uploads, images)")
      assert_contains(body, "append-heavy timestamped events")
      assert_contains(body, "column-oriented analytics table")
      # Schema editing moved to the collection page; the modal no longer
      # carries a schema textarea.
      assert_not(body.includes?("name=\"schema\""))
    end

    test("rejects a blank collection name") do
      response = post("/databases/admin_spec_colls/collections", { "name": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "collection name is required")
    end

    test("creates a timeseries collection") do
      response = post("/databases/admin_spec_colls/collections", { "name": "ts_specs", "type": "timeseries" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "collection ts_specs created")
      assert_contains(res_body(response), "timeseries")

      response = delete("/databases/admin_spec_colls/collections/ts_specs")
      assert_eq(res_status(response), 200)
    end

    test("creates and drops a columnar collection") do
      columns_json = "[{\"name\": \"host\", \"type\": \"string\"}, {\"name\": \"value\", \"type\": \"float\"}]"
      response = post("/databases/admin_spec_colls/collections",
                      { "name": "col_specs", "type": "columnar", "columns": columns_json })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "columnar collection col_specs created")
      assert_contains(body, "col_specs")

      response = delete("/databases/admin_spec_colls/collections/col_specs?ctype=columnar")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "columnar collection col_specs dropped")
    end

    test("rejects columnar without column definitions") do
      response = post("/databases/admin_spec_colls/collections",
                      { "name": "col_bad", "type": "columnar", "columns": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "columnar collections need a JSON array")
    end

    test("creates with sharding options") do
      response = post("/databases/admin_spec_colls/collections",
                      { "name": "sharded_specs", "type": "document",
                        "num_shards": "2", "shard_key": "_key", "replication_factor": "1" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "collection sharded_specs created")

      response = delete("/databases/admin_spec_colls/collections/sharded_specs")
      assert_eq(res_status(response), 200)
    end
  end

  describe("index management") do
    before_all() do
      SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_colls"), { "name": "idx_specs" })
    end

    after_all() do
      SolidbClient.delete_api(SolidbEndpoints.collection("admin_spec_colls", "idx_specs"))
    end

    test("lists indexes for a collection") do
      response = get("/databases/admin_spec_colls/collections/idx_specs/indexes")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "indexes")
      assert_contains(res_body(response), "new index")
    end

    test("create, rebuild, drop a persistent index") do
      response = post("/databases/admin_spec_colls/collections/idx_specs/indexes",
                      { "index_name": "by_email", "fields": "email", "index_type": "persistent",
                        "unique": "true" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "index by_email created")
      assert_contains(body, "by_email")
      assert_contains(body, "email")

      response = put("/databases/admin_spec_colls/collections/idx_specs/indexes/rebuild", {})
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "indexes rebuilt")

      response = delete("/databases/admin_spec_colls/collections/idx_specs/indexes/by_email")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "index by_email dropped")
    end

    test("enable then disable document versioning") do
      response = put("/databases/admin_spec_colls/collections/idx_specs/versioning",
                     { "versioning": "true" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "versioning enabled")
      assert_contains(body, "Versioning: on")

      response = put("/databases/admin_spec_colls/collections/idx_specs/versioning",
                     { "versioning": "false" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "versioning disabled")
      assert_contains(body, "Versioning: off")
    end

    test("enable then disable query-driven auto-index") do
      response = put("/databases/admin_spec_colls/collections/idx_specs/auto_index",
                     { "auto_index": "true" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "auto-index enabled")
      assert_contains(body, "Auto-index: on")

      response = put("/databases/admin_spec_colls/collections/idx_specs/auto_index",
                     { "auto_index": "false" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "auto-index disabled")
      assert_contains(body, "Auto-index: off")
    end

    test("creates a multi-field hash index") do
      response = post("/databases/admin_spec_colls/collections/idx_specs/indexes",
                      { "index_name": "by_pair", "fields": "first, last", "index_type": "hash" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "index by_pair created")
      assert_contains(body, "first, last")

      delete("/databases/admin_spec_colls/collections/idx_specs/indexes/by_pair")
    end

    test("creates and drops a ttl index") do
      response = post("/databases/admin_spec_colls/collections/idx_specs/indexes",
                      { "index_name": "expiry", "fields": "created_at", "index_type": "ttl",
                        "expire_after_seconds": "3600" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "index expiry created")
      assert_contains(body, "expires after 3600s")

      response = delete("/databases/admin_spec_colls/collections/idx_specs/indexes/expiry")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "index expiry dropped")
    end

    test("creates and drops a geo index") do
      response = post("/databases/admin_spec_colls/collections/idx_specs/indexes",
                      { "index_name": "by_location", "fields": "location", "index_type": "geo" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "index by_location created")

      response = delete("/databases/admin_spec_colls/collections/idx_specs/indexes/by_location")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "index by_location dropped")
    end

    test("rejects an index without name or fields") do
      response = post("/databases/admin_spec_colls/collections/idx_specs/indexes",
                      { "index_name": "", "fields": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "index name and fields are required")
    end

    test("rejects a ttl index without expiry") do
      response = post("/databases/admin_spec_colls/collections/idx_specs/indexes",
                      { "index_name": "expiry", "fields": "created_at", "index_type": "ttl",
                        "expire_after_seconds": "" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "ttl indexes need expire_after_seconds")
    end

    test("surfaces an error when dropping a missing index") do
      response = delete("/databases/admin_spec_colls/collections/idx_specs/indexes/nope")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "HTTP 4")
    end
  end

end
