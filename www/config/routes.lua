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
router.get("/sidebar/calendar_invites", "dashboard#sidebar_calendar_invites")

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
  router.get("", "dashboard#index")

  -- Views routes
  router.get("/views", "dashboard/views#index")
  router.post("/views/:name/refresh", "dashboard/views#refresh")
  router.delete("/views/:name/delete", "dashboard/views#destroy")

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
  router.post("/query/nl", "dashboard/query#nl")
  router.post("/query/nl/feedback", "dashboard/query#nl_feedback")

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

  -- Slow Queries page
  router.get("/slow-queries", "dashboard/monitoring#slow_queries_page")
  router.get("/slow-queries/table", "dashboard/monitoring#slow_queries")
  router.delete("/slow-queries", "dashboard/monitoring#clear_slow_queries")

  -- Sharding routes (HTMX)
  router.get("/sharding/distribution", "dashboard/admin#sharding_distribution")
  router.get("/sharding/collections", "dashboard/admin#sharding_collections")

  -- Users/Roles tables (HTMX)
  router.get("/users/table", "dashboard/admin#users_table")
  router.get("/roles/table", "dashboard/admin#roles_table")
  router.get("/apikeys/table", "dashboard/admin#apikeys_table")

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
  router.get("/users/:username/modal/edit", "dashboard/admin#users_modal_edit")
  router.put("/users/:username", "dashboard/admin#update_user")
  router.get("/roles/table", "dashboard/admin#roles_table")
  router.get("/roles/modal/create", "dashboard/admin#roles_modal_create")
  router.post("/roles", "dashboard/admin#create_role")
  router.delete("/roles/:name", "dashboard/admin#delete_role")
  router.post("/users/:username/roles", "dashboard/admin#assign_role")
  router.delete("/users/:username/roles/:role", "dashboard/admin#revoke_role")
  router.get("/monitoring", "dashboard/monitoring#index")
end)

-- Admin Users Management - requires session auth + admin
router.scope("/admin", { middleware = { "session_auth" } }, function()
  -- Users
  router.get("/users", "admin/users#index")
  router.get("/users/:key", "admin/users#show")
  router.get("/users/:key/edit", "admin/users#edit")
  router.put("/users/:key", "admin/users#update")
  router.delete("/users/:key", "admin/users#destroy")

  -- Invitations
  router.get("/invitations", "admin/users#invitations")
  router.post("/invitations", "admin/users#create_invitation")
  router.delete("/invitations/:key", "admin/users#delete_invitation")
end)

-- Talks (Chat App) routes - requires session auth
router.scope("/talks", { middleware = { "session_auth" } }, function()
  router.get("", "talks#index")
  router.get("/livequery_token", "talks#livequery_token")
  router.get("/user/:key", "talks#get_user")
  router.get("/setup_presence", "talks#setup_presence")

  -- Sidebar partials
  router.get("/sidebar/channels", "talks#sidebar_channels")
  router.get("/sidebar/dms", "talks#sidebar_dms")
  router.get("/sidebar/users", "talks#sidebar_users")

  -- Messages
  router.get("/messages/:channel", "talks#messages")
  router.get("/message/:key", "talks#show_message")
  router.get("/message/:key/edit", "talks#edit_message")
  router.post("/message", "talks#send_message")
  router.put("/message/:key", "talks#update_message")
  router.delete("/message/:key", "talks#delete_message")

  -- Reactions
  router.get("/emoji_picker/:key", "talks#emoji_picker")
  router.post("/react", "talks#toggle_reaction")

  -- Threads
  router.get("/thread/:message_id", "talks#thread")
  router.post("/thread/:message_id/reply", "talks#thread_reply")

  -- Channels
  router.get("/channel/modal/create", "talks#channel_modal")
  router.get("/channel/users", "talks#channel_users")
  router.post("/channel", "talks#create_channel")

  -- Groups
  router.get("/group/modal/create", "talks#group_modal")
  router.post("/group", "talks#create_group")

  -- DMs
  router.post("/dm/start/:user_key", "talks#dm_start")

  -- Files
  router.get("/file/:key", "talks#file")

  -- Calls
  router.get("/call/ui/:channel_key", "talks#call_ui")
  router.get("/call/ui/:channel_key/:type", "talks#call_ui")
  router.post("/call/join/:channel_key", "talks#call_join")
  router.post("/call/leave/:channel_key", "talks#call_leave")
  router.get("/call/participants/:channel_key", "talks#call_participants")
  router.post("/call/decline", "talks#call_decline")
  router.post("/call/signal", "talks#call_signal")
  router.delete("/call/signal/:key", "talks#call_signal_delete")
end)

-- MailBox (Internal Webmail + Calendar) routes - requires session auth
router.scope("/mailbox", { middleware = { "session_auth" } }, function()
  -- Main dashboard
  -- Main dashboard
  router.get("", "mailbox#index")
  router.get("/sidebar", "mailbox#update_sidebar")

  -- Messages
  router.get("/messages", "mailbox#inbox")
  router.get("/messages/:folder", "mailbox#folder")
  router.get("/messages/view/:id", "mailbox#view_message")
  router.get("/messages/table", "mailbox#messages_table")
  router.get("/compose", "mailbox#compose")
  router.get("/compose/reply/:id", "mailbox#compose_reply")
  router.get("/compose/forward/:id", "mailbox#compose_forward")
  router.post("/send", "mailbox#send")
  router.post("/draft", "mailbox#save_draft")
  router.post("/messages/:id/star", "mailbox#toggle_star")
  router.post("/messages/:id/read", "mailbox#mark_read")
  router.post("/messages/:id/archive", "mailbox#archive")
  router.delete("/messages/:id", "mailbox#delete_message")

  -- Recipients search (HTMX autocomplete)
  router.get("/recipients/search", "mailbox#search_recipients")

  -- Calendar
  router.get("/calendar", "mailbox/calendar#index")
  router.get("/calendar/events", "mailbox/calendar#events")
  router.get("/calendar/event/:id", "mailbox/calendar#show")
  router.get("/calendar/event/modal/create", "mailbox/calendar#modal_create")
  router.get("/calendar/event/:id/modal/edit", "mailbox/calendar#modal_edit")
  router.post("/calendar/event", "mailbox/calendar#create")
  router.put("/calendar/event/:id", "mailbox/calendar#update")
  router.delete("/calendar/event/:id", "mailbox/calendar#delete")
  router.get("/calendar/debug_data", "mailbox/calendar#debug_data")
  router.post("/calendar/event/:id/respond", "mailbox/calendar#respond")

  -- Contacts
  router.get("/contacts", "mailbox/contacts#index")
  router.get("/contacts/search", "mailbox/contacts#search")

  -- Settings
  router.get("/settings", "mailbox/settings#index")
  router.post("/settings", "mailbox/settings#update")
end)

-- Repositories - requires session auth
router.scope("/repositories", { middleware = { "session_auth" } }, function()
  router.get("", "repositories#index")
  router.get("/new", "repositories#new_form")
  router.post("", "repositories#create")
  router.get("/:id", "repositories#show")
  router.get("/:id/edit", "repositories#edit")
  router.put("/:id", "repositories#update")
  router.delete("/:id", "repositories#destroy")

  -- Code browser routes
  router.get("/:id/tree", "repositories#tree")
  router.get("/:id/tree/*path", "repositories#tree")
  router.get("/:id/blob/*path", "repositories#blob")
  router.get("/:id/raw/*path", "repositories#raw")
  router.get("/:id/commits", "repositories#commits")

  -- Collaborators
  router.get("/:id/collaborators", "repositories#collaborators")
  router.get("/:id/collaborators/search", "repositories#collaborators_search")
  router.post("/:id/collaborators", "repositories#add_collaborator")
  router.delete("/:id/collaborators/:user_key", "repositories#remove_collaborator")

  -- Sync & Restore (blob storage)
  router.post("/:id/restore", "repositories#restore")
  router.post("/:id/sync", "repositories#sync")
  router.get("/:id/sync_status", "repositories#sync_status")

  -- Merge requests
  router.get("/:repo_id/merge_requests", "merge_requests#index")
  router.get("/:repo_id/merge_requests/new", "merge_requests#new_form")
  router.post("/:repo_id/merge_requests/compare", "merge_requests#compare")
  router.post("/:repo_id/merge_requests", "merge_requests#create")
  router.get("/:repo_id/merge_requests/:id", "merge_requests#show")
  router.get("/:repo_id/merge_requests/:id/edit", "merge_requests#edit")
  router.put("/:repo_id/merge_requests/:id", "merge_requests#update")
  router.delete("/:repo_id/merge_requests/:id", "merge_requests#destroy")
  router.post("/:repo_id/merge_requests/:id/comments", "merge_requests#add_comment")
  router.post("/:repo_id/merge_requests/:id/close", "merge_requests#close")
  router.post("/:repo_id/merge_requests/:id/reopen", "merge_requests#reopen")
  router.post("/:repo_id/merge_requests/:id/merge", "merge_requests#merge")
end)

-- Pages routes - requires session auth
router.scope("/pages", { middleware = { "session_auth" } }, function()
  router.get("", "pages#index")
  router.get("/sidebar", "pages#sidebar")
  router.get("/sidebar/children/:parent_id", "pages#sidebar_children")
  router.get("/favorites", "pages#favorites")
  router.post("/:key/favorite", "pages#toggle_favorite")
  router.get("/modal/create", "pages#new_form")
  router.post("/upload_cover", "pages#upload_cover")
  router.post("/upload_file", "pages#upload_file")
  router.post("/reorder", "pages#reorder_pages")
  router.post("", "pages#create")
  router.get("/:key", "pages#show")
  router.get("/:key/edit", "pages#edit")
  router.put("/:key", "pages#update")
  router.delete("/:key", "pages#destroy")
  
  -- Block management
  router.get("/:key/blocks", "pages#blocks")
  router.post("/:key/blocks", "pages#add_block")
  router.get("/:key/blocks/:block_id", "pages#get_block")
  router.get("/:key/blocks/:block_id/edit", "pages#get_block_edit")
  router.put("/:key/blocks/:block_id", "pages#update_block")
  router.delete("/:key/blocks/:block_id", "pages#delete_block")
  router.post("/:key/blocks/:block_id/comment", "pages#add_block_comment")
  router.post("/:key/blocks/reorder", "pages#reorder_blocks")

  -- AI block generation
  router.post("/:key/blocks/:block_id/generate", "pages#generate_ai")
  router.get("/ai/providers", "pages#ai_providers")

  -- History
  router.get("/:key/history", "pages#history")
  router.get("/revision/:revision_id", "pages#revision")
end)

-- Belote Game (Demo) - requires session auth
router.scope("/belote", { middleware = { "session_auth" } }, function()
  router.get("", "belote#index")
  router.post("/new", "belote#create")
  router.post("/join/:id", "belote#join")
  router.get("/game/:id", "belote#show")
  router.get("/game/:id/state", "belote#state") -- HTMX polling endpoint
  router.post("/game/:id/play", "belote#play_card")
  router.post("/game/:id/start", "belote#start_game") -- Manual start for testing
  router.post("/game/:id/take", "belote#take_trump") -- Accept trump
  router.post("/game/:id/pass", "belote#pass_trump") -- Pass on trump
end)

-- Git Smart HTTP (uses HTTP Basic Auth with user credentials)
router.get("/git/:repo_path/info/refs", "git_http#info_refs")
router.post("/git/:repo_path/git-upload-pack", "git_http#upload_pack")
router.post("/git/:repo_path/git-receive-pack", "git_http#receive_pack")

-- Projects routes - requires session auth
router.scope("/projects", { middleware = { "session_auth" } }, function()
  router.get("", "projects#index")
  router.get("/sidebar/in-progress", "projects#sidebar_in_progress")
  router.get("/sidebar/my-tasks", "projects#sidebar_my_tasks")
  router.get("/sidebar/apps", "projects#sidebar_apps")
  router.get("/app/modal/create", "projects#app_modal_create")
  router.post("/app", "projects#create_app")
  router.get("/app/:key/edit", "projects#app_edit")
  router.put("/app/:key", "projects#update_app")
  router.delete("/app/:key", "projects#delete_app")

  -- Features routes (within an App)
  router.get("/app/:app_key", "projects#features")
  router.get("/app/:app_key/feature/modal/create", "projects#feature_modal_create")
  router.post("/app/:app_key/feature", "projects#create_feature")
  router.get("/app/:app_key/feature/:key/edit", "projects#feature_edit")
  router.put("/app/:app_key/feature/:key", "projects#update_feature")
  router.delete("/app/:app_key/feature/:key", "projects#delete_feature")

  -- Feature Board routes (Tasks Kanban)
  router.get("/app/:app_key/feature/:key", "projects#board")
  router.get("/app/:app_key/feature/:feature_key/column/:status", "projects#column")
  router.get("/app/:app_key/feature/:feature_key/task/modal/create", "projects#task_modal_create")
  router.post("/app/:app_key/feature/:feature_key/task", "projects#create_task")

  -- Columns management routes
  router.get("/app/:app_key/feature/:feature_key/columns/modal", "projects#columns_modal")
  router.post("/app/:app_key/feature/:feature_key/columns", "projects#add_column")
  router.put("/app/:app_key/feature/:feature_key/columns", "projects#update_columns")
  router.delete("/app/:app_key/feature/:feature_key/columns/:column_id", "projects#delete_column")

  -- Task routes
  router.post("/status", "projects#update_status")
  router.post("/priority", "projects#update_priority")
  router.get("/task/:key", "projects#show")
  router.put("/task/:key", "projects#update")
  router.delete("/task/:key", "projects#delete")
  router.post("/task/:key/comment", "projects#add_comment")
  router.get("/task/:key/assignees", "projects#assignee_dropdown")
  router.post("/task/:key/assign", "projects#assign_task")
  router.post("/task/:key/tags", "projects#update_tags")
  router.post("/task/:key/file", "projects#attach_file")
  router.delete("/task/:key/file/:file_key", "projects#remove_file")
  router.get("/task/:key/card", "projects#task_card")
  router.get("/task/:key/row", "projects#task_row")
end)

-- CRUDs App routes (Dynamic CRUD builder) - requires dashboard auth
router.scope("/cruds", { middleware = { "dashboard_auth" } }, function()
  router.get("", "cruds#index")

  -- Datatype management
  router.get("/datatypes/new", "cruds#new_datatype")
  router.post("/datatypes", "cruds#create_datatype")
  router.get("/datatypes/:slug/edit", "cruds#edit_datatype")
  router.put("/datatypes/:slug", "cruds#update_datatype")
  router.delete("/datatypes/:slug", "cruds#delete_datatype")
  router.get("/datatypes/:slug/schema", "cruds#schema_editor")
  router.put("/datatypes/:slug/schema", "cruds#update_schema")

  -- Dynamic data CRUD (must come after /datatypes routes to avoid conflicts)
  router.get("/data/:datatype_slug", "cruds#data_index")
  router.get("/data/:datatype_slug/new", "cruds#data_new")
  router.post("/data/:datatype_slug", "cruds#data_create")
  router.get("/data/:datatype_slug/:key", "cruds#data_show")
  router.get("/data/:datatype_slug/:key/edit", "cruds#data_edit")
  router.put("/data/:datatype_slug/:key", "cruds#data_update")
  router.delete("/data/:datatype_slug/:key", "cruds#data_delete")

  -- File uploads
  router.get("/upload/config", "cruds#upload_config")
  router.get("/file/:key", "cruds#file_proxy")
end)

return router
