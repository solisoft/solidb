Routes = { ["GET"] = {
    [""] = "welcome#index",
    ["pdf"] = { ["@"] = "welcome#pdf" },
    ["chart"] = { ["@"] = "welcome#chart" },
    ["redis-incr"] = { ["@"] = "welcome#redis_incr" },
    ["docs"] = {
      ["@"] = "docs#index"
    },
    ["dashboard"] = {
      ["@"] = "dashboard#index"
    },
    ["talks"] = {
      ["@"] = "talks#index"
    }
  }
}
-- Talks API routes
CustomRoute("POST", "/talks/create_message", "talks#create_message")
CustomRoute("POST", "/talks/create_channel", "talks#create_channel")
CustomRoute("POST", "/talks/update_status", "talks#update_status")
CustomRoute("POST", "/talks/toggle_reaction", "talks#toggle_reaction")
CustomRoute("POST", "/talks/create_dm", "talks#create_dm")
CustomRoute("POST", "/talks/toggle_favorite", "talks#toggle_favorite")
CustomRoute("GET", "/talks/livequery_token", "talks#livequery_token")
CustomRoute("GET", "/talks/login", "talks#login_form")
CustomRoute("POST", "/talks/login", "talks#login")
CustomRoute("GET", "/talks/signup", "talks#signup_form")
CustomRoute("POST", "/talks/signup", "talks#signup")
CustomRoute("GET", "/talks/logout", "talks#logout")
CustomRoute("GET", "/talks/channel_data", "talks#channel_data")
CustomRoute("GET", "/talks/og_metadata", "talks#og_metadata")
CustomRoute("GET", "/talks/search", "talks#search")

-- File routes
CustomRoute("POST", "/talks/upload", "talks#upload")
CustomRoute("GET", "/talks/file", "talks#file")
CustomRoute("POST", "/talks/signal", "talks#send_signal")
CustomRoute("POST", "/talks/delete_signal", "talks#delete_signal")
CustomRoute("POST", "/talks/join_call", "talks#join_call")
CustomRoute("POST", "/talks/leave_call", "talks#leave_call")

CustomRoute("GET", "/database/:db/query", "dashboard#query")
CustomRoute("GET", "/database/:db/collections", "dashboard#collections")
CustomRoute("GET", "/database/:db/collection/:collection/indexes", "dashboard#indexes")
CustomRoute("GET", "/database/:db/collection/:collection/documents", "dashboard#documents")
CustomRoute("GET", "/database/:db/collection/:collection/live", "dashboard#live")
CustomRoute("GET", "/database/:db/live_query", "dashboard#live_query")

CustomRoute("GET", "/database/:db/databases", "dashboard#databases")
CustomRoute("GET", "/database/:db/cluster", "dashboard#cluster")
CustomRoute("GET", "/database/:db/apikeys", "dashboard#apikeys")
CustomRoute("GET", "/database/:db/scripts", "dashboard#scripts")
CustomRoute("GET", "/database/:db/queues", "dashboard#queues")
CustomRoute("GET", "/database/:db/sharding", "dashboard#sharding")
CustomRoute("GET", "/database/:db/monitoring", "dashboard#monitoring")

-- docs pages
CustomRoute("GET", "/docs/:page", "docs#show")

CustomRoute("POST", "/upload/:collection/:key/:field", "uploads#upload")
CustomRoute("GET", "/o/:uuid/:format", "uploads#original_image")
CustomRoute("GET", "/r/:uuid/:width/:format", "uploads#resized_image_x", { [":width"] = "([0-9]+)" })
CustomRoute("GET", "/xy/:uuid/:width/:height/:format", "uploads#resized_image_x_y", { [":width"] = "([0-9]+)", [":height"] = "([0-9]+)" })

Logger(Routes)
-- CustomRoute("GET", "demo/with/:id/nested/:demo/route", "welcome#ban", {
--  [":demo"] = "([0-9]+)" -- you can define regex per params
-- })

-- CustomRoute("GET", "ban*", "welcome#ban") -- use splat

-- Resource("customers", {
--   var_name = "customer_id",         -- default value is "id"
--   var_regex = "([0-9a-zA-Z_\\-]+)", -- default value
-- })
-- -- Will generate :
-- -- GET /customers                    -- customers#index
-- -- GET /customers/new                -- customers#new
-- -- GET /customers/:customer_id       -- customers#show
-- -- POST /customers                   -- customers#create
-- -- GET /customers/:customer_id/edit  -- customers#edit
-- -- PUT /customers/:customer_id       -- customers#update
-- -- DELETE /customers/:customer_id    -- customers#delete
--
-- CustomRoute("GET", "ban", "customers#ban", {
--   parent = { "customers" },
--   type = "member", -- collection or member -- customers#ban
-- })
-- -- Will generate :
-- -- GET /customers/:id/ban
--
-- Resource("comments", {
--   var_name = "comment_id",          -- default value is "id"
--   var_regex = "([0-9a-zA-Z_\\-]+)", -- default value
--   parent = { "customers" }
-- })
-- -- Will generate :
-- -- GET /customers/:customer_id/comments                   -- comments#index
-- -- GET /customers/:customer_id/comments/new               -- comments#new
-- -- GET /customers/:customer_id/comments/:comment_id       -- comments#show
-- -- POST /customers/:customer_id/comments                  -- comments#create
-- -- GET /customers/:customer_id/comments/:comment_id/edit  -- comments#edit
-- -- PUT /customers/:customer_id/comments/:comment_id       -- comments#update
-- -- DELETE /customers/:customer_id/comments/:comment_id    -- comments#delete

-- Resource("likes", {
--   var_name = "like_id",          -- default value is "id"
--   var_regex = "([0-9a-zA-Z_\\-]+)", -- default value
--   parent = { "customers", "comments" }
-- })
-- -- Will generate :
-- -- GET /customers/:customer_id/comments/:comment_id/likes                -- likes#index
-- -- GET /customers/:customer_id/comments/:comment_id/likes/new            -- likes#new
-- -- GET /customers/:customer_id/comments/:comment_id/likes/:like_id       -- likes#show
-- -- POST /customers/:customer_id/comments/:comment_id/likes               -- likes#create
-- -- GET /customers/:customer_id/comments/:comment_id/likes/:like_id/edit  -- likes#edit
-- -- PUT /customers/:customer_id/comments/:comment_id/likes/:like_id       -- likes#update
-- -- DELETE /customers/:customer_id/comments/:comment_id/likes/:like_id    -- likes#delete

-- Resource("books", {
--   var_name = "book_id",
--   var_regex = "([0-9a-zA-Z_\\-]+)",
--   parent = { "customers" },
--   only = { "index", "show" },
-- })
-- -- Will generate :
-- -- GET /customers/:customer_id/books                   -- books#index
-- -- GET /customers/:customer_id/books/:book_id       -- books#show
