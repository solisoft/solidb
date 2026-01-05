-- CORS (Cross-Origin Resource Sharing) middleware
-- Configurable CORS headers for API endpoints

return function(ctx, next)
  -- Default CORS configuration
  local config = {
    origin = "*",
    methods = "GET, POST, PUT, PATCH, DELETE, OPTIONS",
    headers = "Content-Type, Authorization, X-Requested-With",
    credentials = false,
    max_age = 86400  -- 24 hours
  }

  -- Set CORS headers
  ctx:set_header("Access-Control-Allow-Origin", config.origin)
  ctx:set_header("Access-Control-Allow-Methods", config.methods)
  ctx:set_header("Access-Control-Allow-Headers", config.headers)
  ctx:set_header("Access-Control-Max-Age", tostring(config.max_age))

  if config.credentials then
    ctx:set_header("Access-Control-Allow-Credentials", "true")
  end

  -- Handle preflight OPTIONS requests
  if ctx.method == "OPTIONS" then
    return ctx:halt(204)
  end

  next()
end
