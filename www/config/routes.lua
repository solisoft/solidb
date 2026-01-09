-- Route definitions for Luaonbeans
-- This file is reloaded automatically in dev mode

local router = require("router")

-- Clear existing routes before defining new ones
router.clear()

-- Home
router.get("/", "home#index")
router.get("/up", "home#up")
router.get("/about", "home#about")

-- Global Sidebar Widgets (use session auth, not dashboard auth)
router.get("/sidebar/tasks_progress", "dashboard#sidebar_tasks_progress")
router.get("/sidebar/tasks_priority", "dashboard#sidebar_tasks_priority")
router.get("/sidebar/pending_mrs", "dashboard#sidebar_pending_mrs")
router.get("/sidebar/recent_messages", "dashboard#sidebar_recent_messages")

-- Auth routes
router.get("/auth/login", "auth#login")
router.post("/auth/login", "auth#do_login")
router.get("/auth/signup", "auth#signup")
router.post("/auth/signup", "auth#do_signup")
router.get("/auth/logout", "auth#logout")

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
  router.get("/documents/:collection/edit/:edit_key", "dashboard/collections#documents_with_edit")
  router.get("/documents/:collection/table", "dashboard/collections#documents_table")
  router.get("/documents/:collection/modal/create", "dashboard/collections#documents_modal_create")
  router.get("/documents/:collection/modal/upload", "dashboard/collections#blob_upload_modal")
  router.post("/documents/:collection", "dashboard/collections#create_document")
  router.get("/documents/:collection/:key/edit", "dashboard/collections#documents_modal_edit")
  router.put("/documents/:collection/:key", "dashboard/collections#update_document")
  router.delete("/documents/:collection/truncate", "dashboard/collections#truncate_collection")
  router.delete("/documents/:collection/:key", "dashboard/collections#delete_document")

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
  router.post("/query/translate", "dashboard/query#translate")

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

  -- AI Agents
  router.get("/ai-agents", "dashboard/ai_dashboard#agents")
  router.get("/ai-agents/table", "dashboard/ai_dashboard#agents_table")
  router.get("/ai-agents/modal/create", "dashboard/ai_dashboard#modal_create_agent")
  router.post("/ai-agents", "dashboard/ai_dashboard#create_agent")
  router.put("/ai-agents/:key", "dashboard/ai_dashboard#update_agent")
  router.delete("/ai-agents/:key", "dashboard/ai_dashboard#delete_agent")


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
router.get("/talks/livequery_token", "talks#livequery_token")
router.get("/talks/user/:key", "talks#get_user")
router.get("/talks/setup_presence", "talks#setup_presence")

-- Talks Sidebar partials
router.get("/talks/sidebar/channels", "talks#sidebar_channels")
router.get("/talks/sidebar/dms", "talks#sidebar_dms")
router.get("/talks/sidebar/users", "talks#sidebar_users")

-- Talks Messages
router.get("/talks/messages/:channel", "talks#messages")
router.get("/talks/message/:key", "talks#show_message")
router.post("/talks/message", "talks#send_message")
router.delete("/talks/message/:key", "talks#delete_message")

-- Talks Reactions
router.get("/talks/emoji_picker/:key", "talks#emoji_picker")
router.post("/talks/react", "talks#toggle_reaction")

-- Talks Threads
router.get("/talks/thread/:message_id", "talks#thread")
router.post("/talks/thread/:message_id/reply", "talks#thread_reply")

-- Talks Channels
router.get("/talks/channel/modal/create", "talks#channel_modal")
router.get("/talks/channel/users", "talks#channel_users")
router.post("/talks/channel", "talks#create_channel")

-- Talks Groups
router.get("/talks/group/modal/create", "talks#group_modal")
router.post("/talks/group", "talks#create_group")

-- Talks DMs
router.post("/talks/dm/start/:user_key", "talks#dm_start")

-- Talks Files
router.get("/talks/file/:key", "talks#file")

-- Talks Calls
router.get("/talks/call/ui/:channel_key", "talks#call_ui")
router.get("/talks/call/ui/:channel_key/:type", "talks#call_ui")
router.post("/talks/call/join/:channel_key", "talks#call_join")
router.post("/talks/call/leave/:channel_key", "talks#call_leave")
router.get("/talks/call/participants/:channel_key", "talks#call_participants")
router.post("/talks/call/decline", "talks#call_decline")
router.post("/talks/call/signal", "talks#call_signal")
router.delete("/talks/call/signal/:key", "talks#call_signal_delete")

-- Repositories (explicit routes to avoid pluralization issues)
router.get("/repositories", "repositories#index")
router.get("/repositories/new", "repositories#new_form")
router.post("/repositories", "repositories#create")
router.get("/repositories/:id", "repositories#show")
router.get("/repositories/:id/edit", "repositories#edit")
router.put("/repositories/:id", "repositories#update")
router.delete("/repositories/:id", "repositories#destroy")

-- Code browser routes
router.get("/repositories/:id/tree", "repositories#tree")
router.get("/repositories/:id/tree/*path", "repositories#tree")
router.get("/repositories/:id/blob/*path", "repositories#blob")
router.get("/repositories/:id/raw/*path", "repositories#raw")
router.get("/repositories/:id/commits", "repositories#commits")

-- Merge requests (explicit routes)
router.get("/repositories/:repo_id/merge_requests", "merge_requests#index")
router.get("/repositories/:repo_id/merge_requests/new", "merge_requests#new_form")
router.post("/repositories/:repo_id/merge_requests/compare", "merge_requests#compare")
router.post("/repositories/:repo_id/merge_requests", "merge_requests#create")
router.get("/repositories/:repo_id/merge_requests/:id", "merge_requests#show")
router.get("/repositories/:repo_id/merge_requests/:id/edit", "merge_requests#edit")
router.put("/repositories/:repo_id/merge_requests/:id", "merge_requests#update")
router.delete("/repositories/:repo_id/merge_requests/:id", "merge_requests#destroy")
router.post("/repositories/:repo_id/merge_requests/:id/comments", "merge_requests#add_comment")
router.post("/repositories/:repo_id/merge_requests/:id/close", "merge_requests#close")
router.post("/repositories/:repo_id/merge_requests/:id/reopen", "merge_requests#reopen")
router.post("/repositories/:repo_id/merge_requests/:id/merge", "merge_requests#merge")

-- Git Smart HTTP
-- Matches /git/repo_name.git/info/refs, /git/repo_name.git/git-upload-pack, etc.
-- We use a catch-all for the git path
router.get("/git/:repo_path/info/refs", "git_http#info_refs")
router.post("/git/:repo_path/git-upload-pack", "git_http#upload_pack")
router.post("/git/:repo_path/git-receive-pack", "git_http#receive_pack")

-- Projects routes (Apps CRUD)
router.get("/projects", "projects#index")
router.get("/projects/sidebar/in-progress", "projects#sidebar_in_progress")
router.get("/projects/sidebar/my-tasks", "projects#sidebar_my_tasks")
router.get("/projects/sidebar/apps", "projects#sidebar_apps")
router.get("/projects/app/modal/create", "projects#app_modal_create")
router.post("/projects/app", "projects#create_app")
router.get("/projects/app/:key/edit", "projects#app_edit")
router.put("/projects/app/:key", "projects#update_app")
router.delete("/projects/app/:key", "projects#delete_app")

-- Features routes (within an App)
router.get("/projects/app/:app_key", "projects#features")
router.get("/projects/app/:app_key/feature/modal/create", "projects#feature_modal_create")
router.post("/projects/app/:app_key/feature", "projects#create_feature")
router.get("/projects/app/:app_key/feature/:key/edit", "projects#feature_edit")
router.put("/projects/app/:app_key/feature/:key", "projects#update_feature")
router.delete("/projects/app/:app_key/feature/:key", "projects#delete_feature")

-- Feature Board routes (Tasks Kanban)
router.get("/projects/app/:app_key/feature/:key", "projects#board")
router.get("/projects/app/:app_key/feature/:feature_key/column/:status", "projects#column")
router.get("/projects/app/:app_key/feature/:feature_key/task/modal/create", "projects#task_modal_create")
router.post("/projects/app/:app_key/feature/:feature_key/task", "projects#create_task")

-- Columns management routes
router.get("/projects/app/:app_key/feature/:feature_key/columns/modal", "projects#columns_modal")
router.post("/projects/app/:app_key/feature/:feature_key/columns", "projects#add_column")
router.put("/projects/app/:app_key/feature/:feature_key/columns", "projects#update_columns")
router.delete("/projects/app/:app_key/feature/:feature_key/columns/:column_id", "projects#delete_column")

-- Task routes
router.post("/projects/status", "projects#update_status")
router.post("/projects/priority", "projects#update_priority")
router.get("/projects/task/:key", "projects#show")
router.put("/projects/task/:key", "projects#update")
router.delete("/projects/task/:key", "projects#delete")
router.post("/projects/task/:key/comment", "projects#add_comment")
router.get("/projects/task/:key/assignees", "projects#assignee_dropdown")
router.post("/projects/task/:key/assign", "projects#assign_task")
router.post("/projects/task/:key/tags", "projects#update_tags")
router.post("/projects/task/:key/file", "projects#attach_file")
router.delete("/projects/task/:key/file/:file_key", "projects#remove_file")
router.get("/projects/task/:key/card", "projects#task_card")
router.get("/projects/task/:key/row", "projects#task_row")

-- CRUDs App routes (Dynamic CRUD builder)
router.get("/cruds", "cruds#index")

-- Datatype management
router.get("/cruds/datatypes/new", "cruds#new_datatype")
router.post("/cruds/datatypes", "cruds#create_datatype")
router.get("/cruds/datatypes/:slug/edit", "cruds#edit_datatype")
router.put("/cruds/datatypes/:slug", "cruds#update_datatype")
router.delete("/cruds/datatypes/:slug", "cruds#delete_datatype")
router.get("/cruds/datatypes/:slug/schema", "cruds#schema_editor")
router.put("/cruds/datatypes/:slug/schema", "cruds#update_schema")

-- Dynamic data CRUD (must come after /datatypes routes to avoid conflicts)
router.get("/cruds/data/:datatype_slug", "cruds#data_index")
router.get("/cruds/data/:datatype_slug/new", "cruds#data_new")
router.post("/cruds/data/:datatype_slug", "cruds#data_create")
router.get("/cruds/data/:datatype_slug/:key", "cruds#data_show")
router.get("/cruds/data/:datatype_slug/:key/edit", "cruds#data_edit")
router.put("/cruds/data/:datatype_slug/:key", "cruds#data_update")
router.delete("/cruds/data/:datatype_slug/:key", "cruds#data_delete")

-- File uploads
router.get("/cruds/upload/config", "cruds#upload_config")
router.get("/cruds/file/:key", "cruds#file_proxy")

return router
