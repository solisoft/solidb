# Dashboard Controllers

## Purpose
Lua controllers for the SoliDB Dashboard web UI. Handle database management operations including collections, indexes, queries, monitoring, and AI features.

## Key Files

| File | Description |
|------|-------------|
| `base_controller.lua` | Base class with API helpers - all dashboard controllers extend this |
| `collections_controller.lua` | Collection/document CRUD, columnar storage |
| `indexes_controller.lua` | Index management per collection and database-wide |
| `query_controller.lua` | SDBQL editor, REPL, scripts management |
| `monitoring_controller.lua` | Metrics, stats, slow queries |
| `cluster_controller.lua` | Cluster status, nodes, replication |
| `queue_controller.lua` | Background jobs and cron |
| `admin_controller.lua` | Users, roles, API keys, databases, sharding, env vars |
| `ai_controller.lua` | AI contributions, tasks, agents |

## Base Controller API

All dashboard controllers extend `DashboardBaseController`:

```lua
local DashboardBaseController = require("dashboard/base_controller")
local MyController = DashboardBaseController:extend()

-- Get current database from URL params
local db = self:get_db()  -- Returns self.params.db or "_system"

-- API helpers (authenticated with Bearer token)
local body, status = self:api_get("/path")
local body, status = self:api_post("/path", json_body)
local body, status = self:api_delete("/path")

-- Low-level fetch with options
local status, headers, body = self:fetch_api("/path", {
  method = "PUT",
  body = '{"key": "value"}',
  headers = { ["X-Custom"] = "value" }
})
```

## Common Patterns

### Standard CRUD Actions
```lua
function Controller:index()
  return self:render("template")  -- Full page
end

function Controller:table()
  -- HTMX partial for table refresh
  local data = self:api_get("/_api/...")
  return self:render("_table_partial", { data = DecodeJson(data) })
end

function Controller:create()
  local result = self:api_post("/_api/...", EncodeJson(self.params))
  return self:render("_table_partial", ...)
end

function Controller:destroy()
  self:api_delete("/_api/..." .. self.params.id)
  return self:render("_table_partial", ...)
end
```

### Modal Pattern
```lua
function Controller:modal_create()
  return self:render("_modal_create")  -- Returns modal HTML
end
```

### Form Parameters
Access via `self.params`:
```lua
local name = self.params.name
local db = self.params.db           -- From URL (/database/:db/...)
local collection = self.params.collection
```

**Important**: Array inputs (`name[]`) don't work properly in LuaOnBeans. Use a hidden input with comma-separated values instead:
```html
<input type="hidden" name="values" id="values-input" value="">
<script>
function updateValues() {
  var checkboxes = document.querySelectorAll('.value-checkbox:checked');
  var values = Array.from(checkboxes).map(cb => cb.value);
  document.getElementById('values-input').value = values.join(',');
}
</script>
```

Then parse in controller:
```lua
local raw = self.params.values or ""
local values = {}
for v in string.gmatch(raw, "[^,]+") do
  table.insert(values, v:match("^%s*(.-)%s*$"))
end
```

## Common Tasks

### Adding a New Dashboard Section
1. Create `new_controller.lua` extending `DashboardBaseController`
2. Add routes in `config/routes.lua` under `/database/:db` scope
3. Create views in `app/views/dashboard/`
4. Add sidebar link in `_sidebar.etlua`

### Adding an HTMX Table
1. Create main view with `<div id="table-container" hx-get="..." hx-trigger="load">`
2. Create `_table.etlua` partial with table rows
3. Add `table()` action returning just the partial

### Debugging API Calls
Check server logs for "Fetching API:" entries which show all outbound API calls.

## Dependencies
- **Uses**: SoliDB REST API (via `fetch_api`)
- **Used by**: Dashboard views, HTMX partials

## Gotchas
- Always use `self.params`, not `ngx.req.get_post_args()` (this is Redbean, not OpenResty)
- JSON responses from API need `DecodeJson()` before use
- Database name comes from URL param `self.params.db`
- Modal forms use HTMX `hx-post` with `hx-target` for partial updates
- Auth token stored in `sdb_token` cookie, accessed via `GetCookie()`
