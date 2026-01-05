-- Security headers middleware
-- Adds common security headers to responses

return function(ctx, next)
  -- Prevent clickjacking
  ctx:set_header("X-Frame-Options", "SAMEORIGIN")

  -- Prevent MIME type sniffing
  ctx:set_header("X-Content-Type-Options", "nosniff")

  -- Enable XSS filter (legacy browsers)
  ctx:set_header("X-XSS-Protection", "1; mode=block")

  -- Referrer policy
  ctx:set_header("Referrer-Policy", "strict-origin-when-cross-origin")

  -- Permissions policy (formerly Feature-Policy)
  ctx:set_header("Permissions-Policy", "geolocation=(), microphone=(), camera=()")

  -- Content Security Policy (basic - customize for your app)
  -- ctx:set_header("Content-Security-Policy", "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'")

  next()
end
