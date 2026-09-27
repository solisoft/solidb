# Routes configuration
# Define your application routes here

# Home page
get("/", "home#index")

# Health check endpoint
get("/health", "home#health")

# Documentation (migrated from the old www site)
get("/docs", "docs#index")
get("/docs/:page", "docs#show")

get("/blog", "blog#index")
# Before /blog/:slug, which would otherwise take "feed.xml" as a slug.
get("/blog/feed.xml", "blog#feed")
get("/blog/:slug", "blog#show")

print("Routes loaded!")
