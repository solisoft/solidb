# Routes configuration
#
# Admin UI for a SoliDB server. Global resources (users, roles, api keys) are
# flat; database-scoped resources nest under /databases/:db. No auth
# middleware here -- access protection happens at the reverse-proxy level.

get("/", "home#index", name: "root")
get("/health", "home#health")

# --- Connection (per-session SoliDB server override) ---
get("/connection", "connection#show", name: "connection")
post("/connection", "connection#create")
post("/connection/reset", "connection#reset")

# --- Settings (theme presets) ---
get("/settings", "settings#index", name: "settings")
post("/settings/theme", "settings#update_theme")
post("/settings/font", "settings#update_font")

# --- Cluster monitoring ---
get("/cluster", "cluster#show", name: "cluster")
get("/cluster/panel", "cluster#panel")
post("/cluster/sync-log/prune", "cluster#prune_sync_log")

# --- Databases ---
get("/databases", "databases#index", name: "databases")
post("/databases", "databases#create")
delete("/databases/:db", "databases#delete")

# --- Users (+ per-user role assignment) ---
get("/users", "users#index", name: "users")
post("/users", "users#create")
delete("/users/:username", "users#delete")
post("/users/:username/roles", "users#add_role")
delete("/users/:username/roles/:role", "users#remove_role")

# --- Roles (RBAC) ---
get("/roles", "roles#index", name: "roles")
post("/roles", "roles#create")
get("/roles/:name", "roles#show", name: "role")
put("/roles/:name", "roles#update")
delete("/roles/:name", "roles#delete")

# --- API keys ---
get("/api-keys", "api_keys#index", name: "api_keys")
post("/api-keys", "api_keys#create")
delete("/api-keys/:id", "api_keys#delete")

# --- Collections (db-scoped) ---
get("/databases/:db/collections", "collections#index", name: "db_collections")
post("/databases/:db/collections", "collections#create")
get("/databases/:db/collections/:name/stats", "collections#stats")
delete("/databases/:db/collections/:name", "collections#delete")

# --- Indexes (collection-scoped, HTMX fragments) ---
get("/databases/:db/collections/:name/indexes", "collections#indexes")
post("/databases/:db/collections/:name/indexes", "collections#create_index")
put("/databases/:db/collections/:name/indexes/rebuild", "collections#rebuild_indexes")
delete("/databases/:db/collections/:name/indexes/:index_name", "collections#delete_index")

# --- Documents browser (collection-scoped) ---
# Static segments (truncate / schema / upload / ...) are declared before the
# :key routes so they never get captured as a document key.
get("/databases/:db/collections/:name/docs", "documents#index", name: "db_collection_docs")
post("/databases/:db/collections/:name/docs", "documents#create")
post("/databases/:db/collections/:name/docs/upload", "documents#upload")
get("/databases/:db/collections/:name/docs/export", "documents#export_docs")
post("/databases/:db/collections/:name/docs/import", "documents#import_docs")
put("/databases/:db/collections/:name/docs/truncate", "documents#truncate")
put("/databases/:db/collections/:name/docs/schema", "documents#update_schema")
delete("/databases/:db/collections/:name/docs/schema", "documents#delete_schema")
get("/databases/:db/collections/:name/docs/:key/blob", "documents#blob")
put("/databases/:db/collections/:name/docs/:key", "documents#update")
delete("/databases/:db/collections/:name/docs/:key", "documents#delete")

# --- Timeseries explorer (collection-scoped) ---
# Static segments (data / points / ttl / prune) before any :param route so
# they are never captured as a path parameter.
get("/databases/:db/collections/:name/timeseries", "timeseries#show", name: "db_collection_timeseries")
get("/databases/:db/collections/:name/timeseries/data", "timeseries#data")
get("/databases/:db/collections/:name/timeseries/points", "timeseries#points")
post("/databases/:db/collections/:name/timeseries/ttl", "timeseries#create_ttl")
delete("/databases/:db/collections/:name/timeseries/ttl/:index_name", "timeseries#delete_ttl")
post("/databases/:db/collections/:name/timeseries/prune", "timeseries#prune")

# --- Materialized views (db-scoped) ---
get("/databases/:db/views", "materialized_views#index", name: "db_views")
post("/databases/:db/views", "materialized_views#create")
put("/databases/:db/views/:name/refresh", "materialized_views#refresh")
delete("/databases/:db/views/:name", "materialized_views#delete")

# --- Graph explorer (db-scoped) ---
get("/databases/:db/graph", "graph#show", name: "db_graph")
post("/databases/:db/graph/traverse", "graph#traverse")
post("/databases/:db/graph/demo", "graph#seed_demo")

# --- Query console (db-scoped) ---
get("/databases/:db/query", "query#show", name: "db_query")
post("/databases/:db/query/run", "query#run")
post("/databases/:db/query/explain", "query#explain")
get("/databases/:db/query/slow", "query#slow", name: "db_slow_queries")
get("/databases/:db/query/slow/count", "query#slow_count")
put("/databases/:db/query/slow/clear", "query#clear_slow")

# --- Lua scripts (db-scoped) ---
get("/databases/:db/scripts", "scripts#index", name: "db_scripts")
get("/databases/:db/scripts/new", "scripts#new")
post("/databases/:db/scripts", "scripts#create")
get("/databases/:db/scripts/:id", "scripts#show")
get("/databases/:db/scripts/:id/edit", "scripts#edit")
put("/databases/:db/scripts/:id", "scripts#update")
delete("/databases/:db/scripts/:id", "scripts#delete")

# --- Lua REPL (db-scoped) ---
get("/databases/:db/repl", "repl#show", name: "db_repl")
post("/databases/:db/repl/eval", "repl#eval")

# --- Queues & jobs (db-scoped) ---
get("/databases/:db/queues", "queues#index", name: "db_queues")
get("/databases/:db/queues/:name/jobs", "queues#jobs")
post("/databases/:db/queues/enqueue", "queues#enqueue")
post("/databases/:db/queues/:name/settings", "queues#update_settings")
delete("/databases/:db/queues/jobs/:id", "queues#cancel_job")
post("/databases/:db/queues/jobs/:id/run-now", "queues#run_now")

# --- Cron jobs (db-scoped) ---
get("/databases/:db/cron", "cron#index", name: "db_cron")
post("/databases/:db/cron", "cron#create")
put("/databases/:db/cron/:id", "cron#update")
delete("/databases/:db/cron/:id", "cron#delete")

# --- Triggers (db-scoped) ---
get("/databases/:db/triggers", "triggers#index", name: "db_triggers")
post("/databases/:db/triggers", "triggers#create")
post("/databases/:db/triggers/:id/toggle", "triggers#toggle")
delete("/databases/:db/triggers/:id", "triggers#delete")

# --- Env vars (db-scoped) ---
get("/databases/:db/env", "env_vars#index", name: "db_env")
post("/databases/:db/env", "env_vars#upsert")
delete("/databases/:db/env/:key", "env_vars#delete")

# --- Live queries / changefeed (db-scoped) ---
get("/databases/:db/live", "live_queries#show", name: "db_live")
get("/databases/:db/live/token", "live_queries#token")
