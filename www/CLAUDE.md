# Web Application (www/)

# MCP Gemini Design

**Gemini is your frontend developer.** For all UI/design work, use this MCP. Tool descriptions contain all necessary instructions.

## Before writing any UI code, ask yourself:

- Is it a NEW visual component (popup, card, section, etc.)? → `snippet_frontend` or `create_frontend`
- Is it a REDESIGN of an existing element? → `modify_frontend`
- Is it just text/logic, or a trivial change? → Do it yourself

## Critical rules:

1. **If UI already exists and you need to redesign/restyle it** → use `modify_frontend`, NOT snippet_frontend.

3. **Tasks can be mixed** (logic + UI). Mentally separate them. Do the logic yourself, delegate the UI to Gemini.


## Purpose
LuaOnBeans web application providing the SoliDB Dashboard, Documentation site, and Talks chat application. Uses Riot.js components, TailwindCSS, and HTMX for dynamic interactions.

## Structure

```
www/
├── app/
│   ├── components/     # Riot.js components (.riot files)
│   ├── controllers/    # Lua controllers (name_controller.lua)
│   ├── models/         # Data models
│   └── views/          # Etlua templates
├── config/
│   ├── database.json   # DB connection config
│   └── routes.lua      # URL routing
├── public/             # Built assets (CSS, JS)
└── beans.lua           # LuaOnBeans initialization
```

## Applications

| App | URL | Description |
|-----|-----|-------------|
| Dashboard | `/database/:db` | Database management UI |
| Documentation | `/docs` | SoliDB documentation |
| Talks | `/talks` | Slack-like team chat |

## Key Patterns

### Routing (config/routes.lua)
```lua
router.get("/docs", "docs#index")           -- Simple route
router.scope("/database/:db", { middleware = { "dashboard_auth" } }, function()
  router.get("/collections", "dashboard/collections#index")
end)
```

### Controllers
Controllers extend `Controller` or `DashboardBaseController`:
```lua
local DashboardBaseController = require("dashboard/base_controller")
local MyController = DashboardBaseController:extend()

function MyController:index()
  local db = self:get_db()           -- Get current database from URL
  local data = self:api_get(path)    -- Make authenticated API call
  return self:render("my/view", { data = data })
end
```

### Views (Etlua)
```html
<%= variable %>           -- Output escaped value
<%- raw_html %>           -- Output unescaped HTML
<% lua_code %>            -- Execute Lua code
<%- include("partial") %> -- Include partial (prefix with _)
```

### HTMX Integration
```html
<div hx-get="/path" hx-trigger="click" hx-target="#result">
  Click me
</div>
```

## Development Commands

```bash
cd www
npm run build:css         # Build TailwindCSS
npm run watch:css         # Watch mode
npm run build:riot        # Compile .riot to JS

# LuaOnBeans
lua beans.lua create controller <name>
lua beans.lua create model <name>
lua beans.lua specs       # Run tests
```

## Common Tasks

### Adding a New Dashboard Page
1. Add route in `config/routes.lua` under database scope
2. Create controller in `app/controllers/dashboard/`
3. Create view in `app/views/dashboard/`
4. Add nav link in `_sidebar.etlua` partial

### Adding an HTMX Partial
1. Create partial file prefixed with `_` (e.g., `_my_partial.etlua`)
2. Add route returning just the partial
3. Use HTMX attributes to load dynamically

### Creating a Riot Component
1. Create `.riot` file in `app/components/`
2. Run `npm run build:riot`
3. Include compiled JS in layout
4. Use `<my-component>` tag in views

## Authentication

- Dashboard auth stored in cookies (`sdb_token`, `sdb_server`)
- `dashboard_auth` middleware checks authentication
- `dashboard_admin_auth` for _system database admin routes
- Tokens are JWT from SoliDB backend

## Gotchas
- This is **LuaOnBeans/Redbean**, NOT OpenResty - don't use `ngx.*` functions
- Use `self.params` for form parameters, not `ngx.req.get_post_args()`
- Array form inputs (`name[]`) need workaround - use comma-separated hidden input
- Partials must be prefixed with `_` for includes
- HTMX requests return just the partial, not full layout
