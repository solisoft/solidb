-- Authentication middleware
-- Checks if user is logged in via session

return function(ctx, next)
  local session = GetSession()

  if not session or not session.user_id then
    -- Store the original URL for redirect after login
    local return_url = ctx.path
    if return_url and return_url ~= "/" then
      SetFlash("return_url", return_url)
    end
    return ctx:redirect("/login")
  end

  -- Store user ID in context for controller access
  ctx.data.current_user_id = session.user_id

  next()
end
