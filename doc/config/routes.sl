# Routes configuration
# Define your application routes here

# Home page
get("/", "home#index")

# Health check endpoint
get("/health", "home#health")

# Documentation (migrated from the old www site)
get("/docs", "docs#index")
get("/docs/:page", "docs#show")

print("Routes loaded!")
