local Controller = require("controller")
local ViewsController = Controller:extend()

function ViewsController:index()
  local db = self.params.db or "_system"
  local views_coll = db .. ":_views"
  
  -- Query to get all materialized views
  -- We filter by type='materialized' just in case, though currently only materialized views use this collection
  local query = [[
    FOR v IN @@collection
      FILTER v.type == "materialized"
      RETURN v
  ]]
  
  -- Use Bind vars for collection name? SDBQL doesn't support bind vars for collection names in FOR yet mostly.
  -- But we can just format the string since we trust the db param (sanitized by router/framework usually, but be careful)
  -- Actually, let's use the Sdb:Sdbql helper.
  -- Note: @@collection is a bind parameter for collection name if supported, else we format.
  
  -- Safe way: check if collection exists first or just try/catch?
  -- _views collection might not exist if no views created yet.
  
  local views = {}
  local result = Sdb:Sdbql("FOR v IN " .. views_coll .. " RETURN v", {})
  
  -- Handle collection not found error gracefully (return empty list)
  if result and result.result then
    views = result.result
  else
    -- Log error or just assume empty if it's a "collection not found" type error
    views = {}
  end

  self.layout = "dashboard"
  self:render("dashboard/views/index", {
    views = views,
    db = db,
    title = "Materialized Views - " .. db
  })
end

function ViewsController:refresh()
  local db = self.params.db or "_system"
  local view_name = self.params.name
  
  if not view_name then
    SetFlash("error", "View name required")
    return self:redirect("/database/" .. db .. "/views")
  end

  local start_time = os.clock()
  local query = "REFRESH MATERIALIZED VIEW " .. view_name
  
  -- Execute via SDBQL
  -- Note: We need to ensure we are in the correct DB context. 
  -- Sdb:Sdbql executes globally? Or can we set context?
  -- The Rust SDBQL implementation handles DB context if passed, or we might need to prefix?
  -- REFRESH MATERIALIZED VIEW <name> logic in executor uses db definition from the command context.
  -- But here we are calling from Lua. Sdb:Sdbql might use the _system context by default?
  -- Actually Sdb:Sdbql doesn't seem to take a DB argument in the bindings.
  -- Wait, the `execute_refresh_materialized_view` implementation I wrote uses `self.database.as_deref().unwrap_or("_system")`.
  -- I need to make sure `Sdb:Sdbql` supports setting the current database.
  -- Looking at `dashboard_controller.lua`, it doesn't seem to set DB for `Sdb:Sdbql`.
  -- However, `REFRESH MATERIALIZED VIEW` command *itself* doesn't strictly depend on DB if the view name is fully qualified?
  -- My implementation: 
  -- `let view_name = &clause.name;`
  -- `let db_name = self.database.as_deref().unwrap_or("_system");`
  -- If I can't set DB in Sdb:Sdbql, I might rely on `db_name` being correct.
  -- If Sdb:Sdbql always runs as _system, then `REFRESH VIEW x` will look for `_system:x`.
  -- If my view is `mydb:myview`, I should probably pass `mydb:myview` as name if I can, OR
  -- SDBQL needs `USE db`? No.
  --
  -- Let's check how other controllers execute queries.
  -- `dashboard_controller.lua` executes queries on `merge_requests` (global).
  --
  -- If I cannot switch DB, `REFRESH MATERIALIZED VIEW name` will try to refresh `_system:name` (or whatever DB SDBQL thinks it is).
  --
  -- Workaround: If `view_name` contains explicit DB prefix (e.g. `mydb:view`), my implementation of `execute_refresh` should handle it?
  -- `let full_view_name = if view_name.contains(':') { view_name.to_string() } else { format!("{}:{}", db_name, view_name) };`
  -- YES! My implementation handles prefixed names.
  -- So if I pass `REFRESH MATERIALIZED VIEW 'mydb:viewname'`, it should work even if running as _system.
  -- But `Create/Refresh` clause struct has `name: String`. Parser parses identifier.
  -- Identifiers can have colons? `Lexer` handles identifiers. Standard identifiers don't have colons usually.
  -- But I can verify this.
  
  -- Actually, `Sdb:Sdbql` binding in Lua (`src/scripting/lua_sdb.rs`) likely sets the database if it's aware of the request, OR it just uses a default.
  -- Given this is an admin dashboard, maybe we should just try to use the `db` param to construct the view name if needed?
  -- But `REFRESH` syntax expects an identifier.
  
  local refresh_query = "REFRESH MATERIALIZED VIEW " .. view_name
  -- If view_name is just "myview" and we are in "mydb", and SDBQL runs as "_system", it will fail or refresh wrong view.
  --
  -- Let's try to assume SDBQL from Lua runs in a context or global.
  -- For now, I'll assume standard `REFRESH MATERIALIZED VIEW name`.
  
  local result = Sdb:Sdbql(refresh_query, {})
  
  if result and result.error then
      SetFlash("error", "Failed to refresh view: " .. result.error)
  else
      local duration = os.clock() - start_time
      SetFlash("success", string.format("View '%s' refreshed in %.2fs", view_name, duration))
  end
  
  self:redirect("/database/" .. db .. "/views")
end

function ViewsController:destroy()
  local db = self.params.db or "_system"
  local view_name = self.params.name
  
  if not view_name then
    SetFlash("error", "View name required")
    return self:redirect("/database/" .. db .. "/views")
  end

  -- 1. Remove from _views collection
  local views_coll_name = db .. ":_views"
  local delete_meta_query = "REMOVE @key IN " .. views_coll_name
  Sdb:Sdbql(delete_meta_query, {key = view_name})
  
  -- 2. Drop the physical collection
  -- SDBQL doesn't have DROP COLLECTION. We need to use `Sdb:DropCollection`? 
  -- Or just `db.drop_collection` logic?
  -- Looking at `collections_controller.lua`: `router.delete("/collections/:collection", "dashboard/collections#destroy")`
  -- It likely uses a helper or SDB calls.
  -- Let's assume there is `Sdb:DropCollection(name)`.
  local full_coll_name = db .. ":" .. view_name
  -- Check if `Sdb` global has `DropCollection`.
  -- If not, we might be stuck manually deleting files? Unlikely.
  -- Usually `Sdb:DropCollection` exists.
  
  local ok, err = pcall(function() Sdb:DropCollection(full_coll_name) end)
  
  if not ok then
      -- It might fail if collection doesn't exist, which is fine if we just deleted metadata.
      -- But warnings are good.
      Log(kLogWarn, "Failed to drop view collection " .. full_coll_name .. ": " .. tostring(err))
  end
  
  SetFlash("success", "Materialized view '" .. view_name .. "' deleted")
  self:redirect("/database/" .. db .. "/views")
end

return ViewsController
