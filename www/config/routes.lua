-- Route definitions for Luaonbeans
-- This file is reloaded automatically in dev mode

local router = require("router")

-- Clear existing routes before defining new ones
router.clear()

-- Home
router.get("/", "home#index")
router.get("/up", "home#up")
router.get("/about", "home#about")

-- SoliDB Documentation
router.get("/docs", "docs#index")
router.get("/docs/:page", "docs#show")
router.get("/slides", "docs#slides")

-- Redirect /database to /database/_system
router.get("/database", function(params)
  return { status = 302, headers = { Location = "/database/_system" } }
end)

-- Dashboard auth routes (main controller)
router.get("/dashboard/login", "dashboard#login")
router.post("/dashboard/login", "dashboard#do_login")
router.get("/dashboard/logout", "dashboard#logout")

-- Database scoped routes (with auth middleware)
router.scope("/database/:db", { middleware = { "dashboard_auth" } }, function()
  -- Dashboard index
  router.get("/", "dashboard#index")
  router.get("", "dashboard#index")

  -- Collections routes
  router.get("/collections", "dashboard/collections#index")
  router.get("/collections/table", "dashboard/collections#table")
  router.get("/collections/modal/create", "dashboard/collections#modal_create")
  router.post("/collections", "dashboard/collections#create")
  router.delete("/collections/:collection", "dashboard/collections#destroy")

  -- Documents routes
  router.get("/documents/:collection", "dashboard/collections#documents")
  router.get("/documents/:collection/table", "dashboard/collections#documents_table")
  router.get("/documents/:collection/modal/create", "dashboard/collections#documents_modal_create")
  router.get("/documents/:collection/modal/upload", "dashboard/collections#blob_upload_modal")
  router.post("/documents/:collection", "dashboard/collections#create_document")
  router.get("/documents/:collection/:key/edit", "dashboard/collections#documents_modal_edit")
  router.put("/documents/:collection/:key", "dashboard/collections#update_document")
  router.delete("/documents/:collection/:key", "dashboard/collections#delete_document")
  router.delete("/documents/:collection/truncate", "dashboard/collections#truncate_collection")

  -- Schema routes
  router.get("/documents/:collection/modal/schema", "dashboard/collections#schema_modal")
  router.post("/documents/:collection/schema", "dashboard/collections#update_schema")
  router.post("/documents/:collection/schema/delete", "dashboard/collections#delete_schema")

  -- Collection indexes routes
  router.get("/indexes/:collection", "dashboard/indexes#collection_indexes")
  router.get("/indexes/:collection/table", "dashboard/indexes#collection_indexes_table")
  router.get("/indexes/:collection/modal/create", "dashboard/indexes#collection_indexes_modal_create")
  router.post("/indexes/:collection", "dashboard/indexes#create_collection_index")
  router.delete("/indexes/:collection/:index_name", "dashboard/indexes#delete_collection_index")

  -- Database-wide indexes
  router.get("/indexes", "dashboard/indexes#index")
  router.get("/indexes/table", "dashboard/indexes#table")

  -- Query routes
  router.get("/query", "dashboard/query#index")
  router.post("/query/execute", "dashboard/query#execute")
  router.post("/query/explain", "dashboard/query#explain")

  -- REPL routes
  router.get("/repl", "dashboard/query#repl")
  router.get("/repl/execute", "dashboard/query#repl_execute")
  router.post("/repl/execute", "dashboard/query#repl_execute")

  -- Scripts routes
  router.get("/scripts", "dashboard/query#scripts")
  router.get("/scripts/table", "dashboard/query#scripts_table")
  router.get("/scripts/stats", "dashboard/query#scripts_stats")
  router.get("/scripts/modal/create", "dashboard/query#scripts_modal_create")
  router.get("/scripts/:script_id/modal/edit", "dashboard/query#scripts_modal_edit")
  router.post("/scripts", "dashboard/query#create_script")
  router.put("/scripts/:script_id", "dashboard/query#update_script")
  router.delete("/scripts/:script_id", "dashboard/query#delete_script")

  -- Live Query
  router.get("/live-query", "dashboard/query#live_query")

  -- Columnar routes
  router.get("/columnar", "dashboard/collections#columnar")
  router.get("/columnar/table", "dashboard/collections#columnar_table")
  router.get("/columnar/modal/create", "dashboard/collections#modal_create")

  -- Stats routes (HTMX)
  router.get("/stats/collections", "dashboard/monitoring#stats_collections")
  router.get("/stats/documents", "dashboard/monitoring#stats_documents")
  router.get("/stats/indexes", "dashboard/monitoring#stats_indexes")
  router.get("/stats/size", "dashboard/monitoring#stats_size")

  -- Queue routes
  router.get("/queues", "dashboard/queue#index")
  router.get("/queues/stats", "dashboard/queue#stats")
  router.get("/queues/jobs", "dashboard/queue#jobs")
  router.get("/queues/jobs/:status", "dashboard/queue#jobs")
  router.get("/queues/cron", "dashboard/queue#cron")
  router.get("/queues/modal/create-job", "dashboard/queue#modal_create_job")
  router.post("/queues/jobs", "dashboard/queue#create_job")
  router.delete("/queues/jobs/:job_id", "dashboard/queue#cancel_job")
  router.get("/queues/modal/create-cron", "dashboard/queue#modal_create_cron")
  router.post("/queues/cron", "dashboard/queue#create_cron")
  router.delete("/queues/cron/:cron_id", "dashboard/queue#delete_cron")

  -- Environment routes
  router.get("/env", "dashboard/admin#env")
  router.get("/env/table", "dashboard/admin#env_table")
  router.get("/env/modal/create", "dashboard/admin#env_modal_create")
  router.get("/env/modal/edit/:key", "dashboard/admin#env_modal_edit")
  router.put("/env/:key", "dashboard/admin#env_set")
  router.delete("/env/:key", "dashboard/admin#env_delete")

  -- Cluster routes (HTMX)
  router.get("/cluster/stats", "dashboard/cluster#stats")
  router.get("/cluster/nodes", "dashboard/cluster#nodes")
  router.get("/cluster/replication-log", "dashboard/cluster#replication_log")

  -- Monitoring routes (HTMX)
  router.get("/monitoring/metrics", "dashboard/monitoring#metrics")
  router.get("/monitoring/extended-stats", "dashboard/monitoring#extended_stats")
  router.get("/monitoring/storage-stats", "dashboard/monitoring#storage_stats")
  router.get("/monitoring/operations", "dashboard/monitoring#operations")
  router.get("/monitoring/slow-queries", "dashboard/monitoring#slow_queries")

  -- Sharding routes (HTMX)
  router.get("/sharding/distribution", "dashboard/admin#sharding_distribution")
  router.get("/sharding/collections", "dashboard/admin#sharding_collections")

  -- Users/Roles tables (HTMX)
  router.get("/users/table", "dashboard/admin#users_table")
  router.get("/roles/table", "dashboard/admin#roles_table")
  router.get("/apikeys/table", "dashboard/admin#apikeys_table")

  -- AI routes
  router.get("/ai/contributions", "dashboard/ai#contributions")
  router.get("/ai/contributions/table", "dashboard/ai#contributions_table")
  router.get("/ai/contributions/stats", "dashboard/ai#contributions_stats")
  router.get("/ai/contributions/modal/create", "dashboard/ai#contributions_modal_create")
  router.post("/ai/contributions", "dashboard/ai#create_contribution")
  router.post("/ai/contributions/:contribution_id/cancel", "dashboard/ai#cancel_contribution")

  router.get("/ai/tasks", "dashboard/ai#tasks")
  router.get("/ai/tasks/stats", "dashboard/ai#tasks_stats")
  router.get("/ai/tasks/table", "dashboard/ai#tasks_table")
  router.delete("/ai/tasks/:task_id", "dashboard/ai#cancel_task")

  router.get("/ai/agents", "dashboard/ai#agents")
  router.get("/ai/agents/grid", "dashboard/ai#agents_grid")
  router.get("/ai/agents/modal/create", "dashboard/ai#agents_modal_create")
  router.get("/ai/agents/:agent_id/edit", "dashboard/ai#agents_modal_edit")
  router.post("/ai/agents", "dashboard/ai#create_agent")
  router.put("/ai/agents/:agent_id", "dashboard/ai#update_agent")
  router.delete("/ai/agents/:agent_id", "dashboard/ai#delete_agent")
end)

-- System database specific routes (admin only)
router.scope("/database/_system", { middleware = { "dashboard_admin_auth" } }, function()
  router.get("/databases", "dashboard/admin#databases")
  router.get("/databases/list", "dashboard/admin#databases_list")
  router.get("/databases/modal/create", "dashboard/admin#databases_modal_create")
  router.post("/databases", "dashboard/admin#create_database")
  router.delete("/databases/:name", "dashboard/admin#delete_database")
  router.get("/cluster", "dashboard/cluster#index")
  router.get("/sharding", "dashboard/admin#sharding")
  router.get("/sharding/distribution", "dashboard/admin#sharding_distribution")
  router.get("/sharding/collections", "dashboard/admin#sharding_collections")
  router.get("/sharding/modal/configure", "dashboard/admin#sharding_modal_configure")
  router.post("/sharding/rebalance", "dashboard/admin#sharding_rebalance")
  router.get("/apikeys", "dashboard/admin#apikeys")
  router.get("/apikeys/table", "dashboard/admin#apikeys_table")
  router.get("/apikeys/modal/create", "dashboard/admin#apikeys_modal_create")
  router.post("/apikeys", "dashboard/admin#create_apikey")
  router.delete("/apikeys/:key_id", "dashboard/admin#delete_apikey")
  router.get("/users", "dashboard/admin#users")
  router.get("/users/table", "dashboard/admin#users_table")
  router.get("/users/modal/create", "dashboard/admin#users_modal_create")
  router.post("/users", "dashboard/admin#create_user")
  router.delete("/users/:username", "dashboard/admin#delete_user")
  router.get("/roles/table", "dashboard/admin#roles_table")
  router.get("/roles/modal/create", "dashboard/admin#roles_modal_create")
  router.post("/roles", "dashboard/admin#create_role")
  router.delete("/roles/:name", "dashboard/admin#delete_role")
  router.post("/users/:username/roles", "dashboard/admin#assign_role")
  router.delete("/users/:username/roles/:role", "dashboard/admin#revoke_role")
  router.get("/monitoring", "dashboard/monitoring#index")
end)

-- Talks (Chat App) routes
router.get("/talks", "talks#index")
router.get("/talks/login", "talks#login")
router.post("/talks/login", "talks#do_login")
router.get("/talks/signup", "talks#signup")
router.post("/talks/signup", "talks#do_signup")
router.get("/talks/logout", "talks#logout")

-- Talks HTMX partials
router.get("/talks/channels/list", "talks#channels_list")
router.get("/talks/dm/list", "talks#dm_list")
router.get("/talks/messages", "talks#messages")
router.post("/talks/message", "talks#send_message")
router.get("/talks/thread/:message_id", "talks#thread")
router.post("/talks/thread/:message_id/reply", "talks#thread_reply")

return router
