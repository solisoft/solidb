# Pin every SoliDB API path builder so a typo in solidb_endpoints.sl fails
# here instead of as a mysterious 404 in a controller.
describe("SolidbEndpoints") do
  test("server and cluster paths") do
    assert_eq(SolidbEndpoints.health(), "/_api/health")
    assert_eq(SolidbEndpoints.cluster_info(), "/_api/cluster/info")
    assert_eq(SolidbEndpoints.livequery_token(), "/_api/livequery/token")
  end

  test("database paths") do
    assert_eq(SolidbEndpoints.databases(), "/_api/databases")
    assert_eq(SolidbEndpoints.database_create(), "/_api/database")
    assert_eq(SolidbEndpoints.database("app"), "/_api/database/app")
  end

  test("user and role paths") do
    assert_eq(SolidbEndpoints.users(), "/_api/auth/users")
    assert_eq(SolidbEndpoints.user("alice"), "/_api/auth/users/alice")
    assert_eq(SolidbEndpoints.user_roles("alice"), "/_api/auth/users/alice/roles")
    assert_eq(SolidbEndpoints.user_role("alice", "editor"), "/_api/auth/users/alice/roles/editor")
    assert_eq(SolidbEndpoints.roles(), "/_api/auth/roles")
    assert_eq(SolidbEndpoints.role("analyst"), "/_api/auth/roles/analyst")
  end

  test("api key paths") do
    assert_eq(SolidbEndpoints.api_keys(), "/_api/auth/api-keys")
    assert_eq(SolidbEndpoints.api_key("k1"), "/_api/auth/api-keys/k1")
  end

  test("collection paths") do
    assert_eq(SolidbEndpoints.collections("app"), "/_api/database/app/collection")
    assert_eq(SolidbEndpoints.collection("app", "users"), "/_api/database/app/collection/users")
    assert_eq(SolidbEndpoints.collection_stats("app", "users"), "/_api/database/app/collection/users/stats")
    assert_eq(SolidbEndpoints.collection_truncate("app", "users"), "/_api/database/app/collection/users/truncate")
    assert_eq(SolidbEndpoints.columnar("app"), "/_api/database/app/columnar")
    assert_eq(SolidbEndpoints.columnar_collection("app", "metrics"), "/_api/database/app/columnar/metrics")
  end

  test("index paths") do
    assert_eq(SolidbEndpoints.collection_indexes("app", "users"), "/_api/database/app/index/users")
    assert_eq(SolidbEndpoints.collection_index("app", "users", "by_email"), "/_api/database/app/index/users/by_email")
    assert_eq(SolidbEndpoints.collection_indexes_rebuild("app", "users"), "/_api/database/app/index/users/rebuild")
    assert_eq(SolidbEndpoints.geo_indexes("app", "places"), "/_api/database/app/geo/places")
    assert_eq(SolidbEndpoints.ttl_indexes("app", "sessions"), "/_api/database/app/ttl/sessions")
  end

  test("document paths") do
    assert_eq(SolidbEndpoints.documents("app", "users"), "/_api/database/app/document/users")
    assert_eq(SolidbEndpoints.document("app", "users", "k1"), "/_api/database/app/document/users/k1")
  end

  test("query paths") do
    assert_eq(SolidbEndpoints.cursor("app"), "/_api/database/app/cursor")
    assert_eq(SolidbEndpoints.explain("app"), "/_api/database/app/explain")
  end

  test("script paths") do
    assert_eq(SolidbEndpoints.scripts("app"), "/_api/database/app/scripts")
    assert_eq(SolidbEndpoints.script("app", "s1"), "/_api/database/app/scripts/s1")
  end

  test("queue and cron paths") do
    assert_eq(SolidbEndpoints.queues("app"), "/_api/database/app/queues")
    assert_eq(SolidbEndpoints.queue_jobs("app", "mail"), "/_api/database/app/queues/mail/jobs")
    assert_eq(SolidbEndpoints.queue_enqueue("app", "mail"), "/_api/database/app/queues/mail/enqueue")
    assert_eq(SolidbEndpoints.queue_job("app", "j1"), "/_api/database/app/queues/jobs/j1")
    assert_eq(SolidbEndpoints.queue_job_run_now("app", "j1"), "/_api/database/app/queues/jobs/j1/run-now")
    assert_eq(SolidbEndpoints.cron_jobs("app"), "/_api/database/app/cron")
    assert_eq(SolidbEndpoints.cron_job("app", "c1"), "/_api/database/app/cron/c1")
  end

  test("trigger paths") do
    assert_eq(SolidbEndpoints.triggers("app"), "/_api/database/app/triggers")
    assert_eq(SolidbEndpoints.trigger("app", "t1"), "/_api/database/app/triggers/t1")
    assert_eq(SolidbEndpoints.trigger_toggle("app", "t1"), "/_api/database/app/triggers/t1/toggle")
  end

  test("env var paths") do
    assert_eq(SolidbEndpoints.env_vars("app"), "/_api/database/app/env")
    assert_eq(SolidbEndpoints.env_var("app", "API_KEY"), "/_api/database/app/env/API_KEY")
  end
end
