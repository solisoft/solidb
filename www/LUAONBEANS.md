# Luaonbeans MVC Framework

A modular MVC framework for [redbean.dev](https://redbean.dev) - the single-file distributable web server.

## Project Structure

```
luaonbeans/
├── .init.lua                    # Bootstrap file
├── .lua/                        # Core framework (don't modify)
│   ├── router.lua               # Routing system
│   ├── controller.lua           # Base controller class
│   ├── model.lua                # Model base class
│   ├── view.lua                 # View/template renderer
│   ├── helpers.lua              # Helper functions
│   ├── i18n.lua                 # Internationalization
│   ├── migrate.lua              # Database migrations
│   ├── test.lua                 # Test framework
│   ├── test_helpers.lua         # Test utilities
│   ├── solidb_model.lua         # SoliDB ORM
│   └── db/                      # Database drivers
│       └── solidb.lua           # SoliDB driver
├── app/
│   ├── controllers/             # Your controllers (*_controller.lua)
│   ├── models/                  # Your models (*.lua)
│   └── views/
│       ├── layouts/             # Layout templates (name/name.etlua)
│       ├── errors/              # Error pages (404.etlua, 500.etlua)
│       └── [controller_name]/   # View templates
├── config/
│   ├── routes.lua               # Route definitions
│   ├── database.lua             # Database configuration
│   └── locales/                 # Translation files
│       ├── en.lua               # English
│       └── fr.lua               # French
├── db/
│   ├── migrate.lua              # Migration CLI runner
│   └── migrations/              # Migration files
├── test/                        # Test files (*_test.lua)
│   └── run.lua                  # Test runner
├── public/                      # Static assets (served first)
│   ├── css/
│   ├── js/
│   └── images/
└── luaonbeans.org               # Redbean executable
```

## Running the Server

```bash
beans dev    # Development mode (watches for changes)
```

## Defining Routes

Routes are defined in `config/routes.lua`:

```lua
router.get("/", "home#index")           -- GET /
router.get("/users/:id", "users#show")  -- GET /users/123
router.post("/users", "users#create")   -- POST /users
router.resources("posts")               -- All CRUD routes for posts
router.resources("users", function()    -- Nested resources
  router.resources("comments")          -- /users/:user_id/comments
end)
```

Route format: `"controller_name#action_name"`

## Creating Controllers

Controllers go in `app/controllers/` and must be named `*_controller.lua`:

```lua
-- app/controllers/users_controller.lua
local Controller = require("controller")
local UsersController = Controller:extend()

function UsersController:index()
  self:render("users/index", { users = get_users() })
end

function UsersController:show()
  local id = self.params.id  -- URL params
  self:json({ id = id })     -- JSON response
end

function UsersController:create()
  -- self.params contains form/query data
  self:redirect("/users")
end

return UsersController
```

### Controller Methods

| Method | Description |
|--------|-------------|
| `self:render(template, locals)` | Render view with data |
| `self:json(data, status)` | JSON response |
| `self:text(content)` | Plain text response |
| `self:html(content)` | HTML response |
| `self:redirect(url, status)` | Redirect |
| `self.params` | URL + query + form params |
| `self.layout` | Set layout name |

## Creating Models

Models go in `app/models/` and use SoliDB as the database:

```lua
-- app/models/user.lua
local Model = require("model")

local User = Model.create("users", {
  validations = {
    email = { presence = true },
    username = { presence = true, length = { between = {3, 50} } }
  }
})

return User
```

### Database Configuration

Create `config/database.lua`:

```lua
return {
  solidb = {
    url = "http://localhost:8529",
    db_name = "myapp",
    username = "root",
    password = "secret"
  }
}
```

### Model Operations

| Operation | Code |
|-----------|------|
| Find by ID | `User.find("users/123")` |
| Find by criteria | `User.find_by({ email = "..." })` |
| Query | `User.where({ active = true }):all()` |
| First/Last | `User.first()`, `User.last()` |
| Create | `User.create({ email = "..." })` |
| Update | `user:update({ name = "..." })` |
| Delete | `user:delete()` |
| Count | `User.count()` |

### Mass Assignment Protection

Protect your models from malicious form submissions by defining `permitted_fields`:

```lua
-- app/models/user.lua
local Model = require("model")

local User = Model.create("users", {
  permitted_fields = { "email", "username", "password" },
  validations = {
    email = { presence = true },
    username = { presence = true }
  }
})

return User
```

Use `Model.permit()` in controllers to filter incoming params:

```lua
-- app/controllers/users_controller.lua
function UsersController:create()
  local user = User:new(User.permit(self.params.user))
  if user:save() then
    self:redirect_to("/users/" .. user.id)
  else
    self:render("users/new", { user = user })
  end
end

function UsersController:update()
  local user = User:find(self.params.id)
  if user:update(User.permit(self.params.user)) then
    self:redirect_to("/users/" .. user.id)
  else
    self:render("users/edit", { user = user })
  end
end
```

This prevents attackers from injecting fields like `is_admin`, `role`, or `user_id` through form submissions.

## Creating Views

Views go in `app/views/[controller]/` with `.etlua` extension:

```html
<!-- app/views/users/index.etlua -->
<h1>Users</h1>
<% for _, user in ipairs(users) do %>
  <p><%- user.name %></p>
<% end %>
```

### Template Syntax

| Syntax | Description |
|--------|-------------|
| `<%- var %>` | Output (unescaped) |
| `<%= var %>` | Output (HTML escaped) |
| `<% code %>` | Lua code (no output) |

### Partials

Partial files start with underscore: `_name.etlua`

```html
<%- partial("shared/header", { title = "Page" }) %>
```

### Layouts

Layouts in `app/views/layouts/`:

```html
<!-- app/views/layouts/application.etlua -->
<!DOCTYPE html>
<html>
<head><title><%- title %></title></head>
<body>
  <%- yield() %>
</body>
</html>
```

### View Variants

Render device-specific templates (e.g., iPhone, tablet):

```
app/views/posts/
├── show.etlua           # Default
├── show.iphone.etlua    # iPhone variant
└── show.tablet.etlua    # Tablet variant
```

```lua
-- Set variant in controller
self.variant = "iphone"
self:render("posts/show", { post = post })

-- Or per-render
self:render("posts/show", { post = post }, { variant = "iphone" })
```

Falls back to default template if variant doesn't exist.

## Static Assets

Files in `public/` are served automatically at their URL path:
- `public/css/style.css` → `/css/style.css`

Use `public_path()` for cache-busting:

```html
<link href="<%- public_path('css/style.css') %>">
<!-- Output: /css/style.css?v=abc123 -->
```

## Request Flow

1. Check `public/` folder for static file
2. Match URL against routes
3. Instantiate controller, call action
4. Render view → Apply layout → Send response

## Adding a New Page

1. Add route in `.init.lua`:
   ```lua
   router.get("/about", "pages#about")
   ```

2. Create controller `app/controllers/pages_controller.lua`:
   ```lua
   local Controller = require("controller")
   local PagesController = Controller:extend()
   
   function PagesController:about()
     self:render("pages/about", { title = "About" })
   end
   
   return PagesController
   ```

3. Create view `app/views/pages/about.etlua`:
   ```html
   <h1>About Us</h1>
   <p>Content here</p>
   ```

## Common Tasks

### JSON API endpoint
```lua
router.get("/api/users", function(params)
  return { json = { users = {} } }
end)
```

### Change layout for controller
```lua
function MyController:before_action()
  self.layout = "admin"
end
```

### Render without layout
```lua
self:render("widget", {}, { layout = false })
```

### Error handling
Error pages: `app/views/errors/404.etlua`, `500.etlua`, `error.etlua`

## Testing

Tests go in `test/` with naming pattern `*_test.lua`:

```lua
-- test/user_test.lua
package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local describe, it, expect = Test.describe, Test.it, Test.expect

describe("User", function()
  it("should create a user", function()
    expect.truthy(true)
  end)
end)
```

### Running Tests

```bash
./luaonbeans.org -i test/run.lua
```

### Assertions

| Assertion | Description |
|-----------|-------------|
| `expect.eq(a, b)` | Assert equal |
| `expect.neq(a, b)` | Assert not equal |
| `expect.truthy(v)` | Assert truthy |
| `expect.falsy(v)` | Assert falsy |
| `expect.nil_value(v)` | Assert nil |
| `expect.not_nil(v)` | Assert not nil |
| `expect.contains(t, v)` | Table contains value |
| `expect.has_key(t, k)` | Table has key |
| `expect.matches(s, p)` | String matches pattern |
| `expect.error(fn)` | Function throws |

## I18n (Internationalization)

Translation files go in `config/locales/`:

```lua
-- config/locales/en.lua
return {
  hello = "Hello",
  welcome = "Welcome, %s!"
}
```

### Usage

```lua
-- In controller or view
t("hello")              -- "Hello"
t("welcome", "John")    -- "Welcome, John!"
t("nav.home")           -- Nested keys with dots

-- Change locale
I18n:set_locale("fr")
```

### In Views

```html
<h1><%- t("hello") %></h1>
<p><%- t("welcome", user.name) %></p>
```

## Migrations

Database migrations allow you to version control your database schema. Migrations support both `up()` (apply) and `down()` (rollback) operations.

### Migration Files

Migrations are stored in `db/migrations/` with timestamped filenames:

```
db/migrations/
├── 20251231120000_create_users_collection.lua
├── 20251231120100_add_email_index_to_users.lua
└── 20251231120200_seed_admin_user.lua
```

### Creating Migrations

```bash
./luaonbeans.org -i db/migrate.lua create add_users_collection
# Creates: db/migrations/20251231143022_add_users_collection.lua
```

### Migration File Format

```lua
-- db/migrations/20251231120000_create_users_collection.lua
local M = {}

function M.up(db, helpers)
  helpers.create_collection("users")
  helpers.add_index("users", { "email" }, { unique = true })
  helpers.seed("users", {
    { email = "admin@example.com", role = "admin" }
  })
end

function M.down(db, helpers)
  helpers.drop_collection("users")
end

return M
```

### Running Migrations

```bash
# Run all pending migrations
./luaonbeans.org -i db/migrate.lua up

# Run only N migrations
./luaonbeans.org -i db/migrate.lua up 1

# Rollback last migration
./luaonbeans.org -i db/migrate.lua down

# Rollback N migrations
./luaonbeans.org -i db/migrate.lua down 2

# Check migration status
./luaonbeans.org -i db/migrate.lua status
```

### Migration Status Output

```
Migration Status
================
  [x] 20251231120000_create_users_collection (batch 1)
  [x] 20251231120100_add_email_index (batch 1)
  [ ] 20251231120200_create_posts_collection (pending)
```

### Helper Functions

| Helper | Description |
|--------|-------------|
| `helpers.create_collection(name, options)` | Create a new collection |
| `helpers.drop_collection(name)` | Delete a collection |
| `helpers.truncate_collection(name)` | Remove all documents from collection |
| `helpers.add_index(collection, fields, options)` | Add an index |
| `helpers.drop_index(collection, index_name)` | Remove an index |
| `helpers.seed(collection, documents)` | Insert seed data |
| `helpers.transform(collection, callback)` | Transform all documents |
| `helpers.execute(query, bindvars)` | Run raw SDBQL query |

### Index Options

```lua
helpers.add_index("users", { "email" }, {
  unique = true,      -- Unique constraint
  sparse = false,     -- Include null values
  name = "idx_email"  -- Custom index name
})
```

### Data Transformation

```lua
function M.up(db, helpers)
  -- Add a field to all existing documents
  helpers.transform("users", function(doc)
    return { status = "active" }
  end)
end
```

### Migration Tracking

Migrations are tracked in the `_migrations` collection with batch numbers. Running `up` creates a new batch; `down` rolls back the last batch by default.
