-- Request logger middleware
-- Logs request method, path, and response time

return function(ctx, next)
  local start_time = os.clock()

  next()

  local elapsed = (os.clock() - start_time) * 1000

  -- Log the request (use redbean's Log if available)
  local log_fn = Log or function(level, msg) print(msg) end
  local log_level = kLogInfo or 6

  log_fn(log_level, string.format(
    "%s %s - %.2fms",
    ctx.method,
    ctx.path,
    elapsed
  ))
end
