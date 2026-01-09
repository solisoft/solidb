# AI Agent Development Guide for SolidB & LuaOnBeans

This comprehensive guide helps AI agents effectively work on the SolidB and LuaOnBeans projects. It covers project architecture, development workflows, code patterns, and best practices for both the Rust database backend and Lua web framework frontend.

## Table of Contents

1. [Project Overview](#project-overview)
2. [SolidB Architecture & Patterns](#solidb-architecture--patterns)
3. [LuaOnBeans Architecture & Patterns](#luaonbeans-architecture--patterns)
4. [Development Environment Setup](#development-environment-setup)
5. [Common Development Tasks](#common-development-tasks)
6. [Cross-Project Integration](#cross-project-integration)
7. [Testing & Debugging](#testing--debugging)
8. [Best Practices & Gotchas](#best-practices--gotchas)
9. [Deployment & Production](#deployment--production)
10. [Troubleshooting Guide](#troubleshooting-guide)

## Project Overview

### SolidB (Rust Database Backend)

**SolidB** is a high-performance multi-document database written in Rust featuring:
- **Custom Query Language (SDBQL)**: ArangoDB-inspired syntax for document operations
- **ACID Transactions**: Configurable isolation levels with WAL support
- **Multi-Node Replication**: Eventual consistency with conflict resolution
- **Horizontal Sharding**: Automatic rebalancing with configurable replication factor
- **Lua Scripting**: Embedded Lua 5.4 runtime for custom endpoints
- **Real-Time Subscriptions**: WebSocket-based live queries and changefeeds
- **Multiple Client SDKs**: Go, Python, Node.js, PHP, Ruby, Elixir, JavaScript

**Key Technologies:**
- **Runtime**: Tokio async runtime
- **Storage**: RocksDB with custom indexing and TTL
- **HTTP Server**: Axum with WebSocket support
- **Serialization**: Serde, MessagePack, Bincode
- **Authentication**: JWT with Argon2 password hashing

### LuaOnBeans (Web Framework Frontend)

**LuaOnBeans** is a lightweight MVC web framework built on [redbean.dev](https://redbean.dev) - a single-file distributable web server. It powers SolidB's web interface including:

- **Database Management Dashboard**: Collection browsing, query execution, index management
- **Documentation Website**: API docs and guides
- **Real-Time Chat (Talks)**: Slack-like interface with channels, DMs, threads
- **Admin Interface**: User management, system monitoring, configuration

**Key Technologies:**
- **Language**: Lua 5.4
- **Templates**: etlua (Embedded Lua templating)
- **Frontend**: HTMX, Riot.js components, TailwindCSS
- **Database**: HTTP API calls to SolidB backend
- **Real-Time**: WebSocket LiveQuery subscriptions

## SolidB Architecture & Patterns

### Core Module Structure

```
src/
├── sdbql/              # Query language implementation
│   ├── executor.rs     # Query execution engine (297KB)
│   ├── parser.rs       # Query parsing and AST generation
│   ├── ast.rs          # Abstract syntax tree definitions
│   └── lexer.rs        # Tokenization
├── storage/            # RocksDB-backed persistence layer
│   ├── engine.rs       # Main storage engine
│   ├── collection.rs   # Document operations (125KB)
│   ├── database.rs     # Database-level operations
│   └── index.rs        # Index management
├── server/             # Axum-based HTTP API
│   ├── handlers.rs     # All endpoint logic (241KB)
│   ├── routes.rs       # Route definitions
│   ├── auth.rs         # Authentication/authorization
│   └── ai_handlers.rs  # AI-specific endpoints
├── cluster/            # Multi-node coordination
├── sync/               # Replication system
├── sharding/           # Horizontal partitioning
├── transaction/        # ACID transactions
├── scripting/          # Lua runtime integration
└── ai/                 # AI-augmented features
```

### Key Data Flow Patterns

1. **HTTP Request** → Axum router → Authentication middleware → Handler function
2. **Query Execution** → SDBQL parser → AST generation → Executor → Storage engine → RocksDB
3. **Replication** → Sync worker → Replication log → Transport layer → Peer nodes
4. **Sharding** → Coordinator → Migration logic → Automatic rebalancing

### Code Patterns & Conventions

#### Error Handling
```rust
#[derive(Error, Debug)]
pub enum DbError {
    #[error("Collection '{0}' not found")]
    CollectionNotFound(String),
    #[error("Query execution failed: {0}")]
    QueryError(String),
    // ... many variants
}

pub type DbResult<T> = Result<T, DbError>;

// Usage in handlers
pub async fn handler() -> Result<Json<Response>, DbError> {
    let collection = db.get_collection("users")?;
    Ok(Json(response))
}
```

#### Async Patterns
```rust
// CPU-intensive operations (blocking)
let result = tokio::task::spawn_blocking(move || {
    expensive_computation(data)
}).await?;

// Background tasks
tokio::spawn(async move {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        perform_maintenance().await;
    }
});
```

#### Handler Structure
```rust
pub async fn api_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    Json(request): Json<CreateDocumentRequest>,
) -> Result<Json<Document>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection(&collection)?;
    let doc = coll.insert(request.data)?;
    Ok(Json(doc))
}
```

#### Testing Patterns
```rust
#[tokio::test]
async fn test_document_operations() {
    let tmp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap();

    let db = engine.get_database("test").unwrap();
    let collection = db.create_collection("docs".to_string(), None).unwrap();

    // Test operations
    assert_eq!(collection.count(), 0);
}
```

### Common SolidB Tasks

#### Adding New Query Functions
```rust
// 1. Define function in sdbql/executor.rs
fn my_custom_function(args: &[Value]) -> DbResult<Value> {
    match args.get(0) {
        Some(Value::String(s)) => Ok(Value::String(s.to_uppercase())),
        _ => Err(DbError::InvalidArguments("Expected string argument".to_string()))
    }
}

// 2. Register in function registry
let registry = FunctionRegistry::new();
// ... add other functions
registry.insert("UPPER".to_string(), my_custom_function);

// 3. Add tests in tests/sdbql_function_tests.rs
#[test]
fn test_upper_function() {
    let result = execute_single(&engine, "RETURN UPPER('hello')");
    assert_eq!(result, Value::String("HELLO".to_string()));
}
```

#### Adding API Endpoints
```rust
// 1. Create handler in server/handlers.rs
pub async fn custom_endpoint(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(request): Json<CustomRequest>,
) -> Result<Json<CustomResponse>, DbError> {
    // Implementation
    Ok(Json(CustomResponse { success: true }))
}

// 2. Add route in server/routes.rs
.route("/_api/database/{db}/custom", post(custom_endpoint))

// 3. Add tests in tests/http_api_test.rs
#[tokio::test]
async fn test_custom_endpoint() {
    // Test implementation
}
```

#### Storage Engine Extensions
```rust
// Add methods to storage/collection.rs
impl Collection {
    pub fn custom_operation(&self, params: CustomParams) -> DbResult<Value> {
        // Implementation with proper indexing and error handling
        Ok(Value::Null)
    }
}
```

## LuaOnBeans Architecture & Patterns

### MVC Structure

```
www/
├── app/
│   ├── controllers/     # Request handlers (*_controller.lua)
│   │   └── dashboard/
│   │       ├── ai_controller.lua
│   │       ├── collections_controller.lua
│   │       └── index_controller.lua
│   ├── models/          # Data models (*.lua)
│   │   └── user.lua
│   └── views/           # Templates (.etlua files)
│       ├── layouts/     # Base layouts
│       └── dashboard/   # Controller views
├── config/
│   ├── routes.lua       # Route definitions
│   ├── database.lua     # DB configuration
│   └── middleware_config.lua
├── db/                  # Migrations
├── public/              # Static assets
└── .lua/               # Framework core (don't modify)
```

### Code Patterns & Conventions

#### Controller Structure
```lua
local DashboardBaseController = require("dashboard.base_controller")
local AIController = DashboardBaseController:extend()

function AIController:agents()
    self.layout = "dashboard"
    self:render("dashboard/ai/agents", {
        title = "AI Agents - " .. self:get_db(),
        db = self:get_db(),
        current_page = "ai_agents"
    })
end

function AIController:create_agent()
    local db = self:get_db()
    local name = self.params.name or ""

    if name == "" then
        SetHeader("HX-Trigger", '{"showToast": {"message": "Agent name is required", "type": "error"}}')
        return self:agents_grid()
    end

    -- API call to SolidB backend
    local status, _, body = self:fetch_api("/_api/database/" .. db .. "/ai/agents", {
        method = "POST",
        body = EncodeJson({
            name = name,
            -- ... other fields
        })
    })

    if status == 200 or status == 201 then
        SetHeader("HX-Trigger", '{"showToast": {"message": "Agent created successfully", "type": "success"}}')
        return self:agents_grid()
    else
        -- Error handling
    end
end

return AIController
```

#### Template Rendering (etlua)
```html
<!-- app/views/dashboard/ai/agents.etlua -->
<div class="max-w-6xl mx-auto">
  <div class="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 mb-6">
    <div>
      <h1 class="text-2xl font-bold text-text"><%= title %></h1>
      <p class="text-text-muted">Manage AI agents for <%= db %></p>
    </div>

    <button type="button"
            hx-get="/database/<%= db %>/ai/agents/modal/create"
            hx-target="#modal-container"
            class="btn-primary">
      <i class="fas fa-plus mr-2"></i>
      Add Agent
    </button>
  </div>

  <!-- Agent grid with HTMX -->
  <div id="agents-grid"
       hx-get="/database/<%= db %>/ai/agents/grid"
       hx-trigger="load, agentUpdated from:body"
       hx-swap="innerHTML">
    <!-- Loading state -->
  </div>
</div>
```

#### Model Patterns (ORM)
```lua
local Model = require("model")

local Agent = Model.create("agents", {
    permitted_fields = {"name", "model", "system_prompt", "capabilities"},
    validations = {
        name = { presence = true, length = { minimum = 1 } },
        model = { presence = true }
    }
})

-- Usage
local agent = Agent.create({
    name = "My Agent",
    model = "claude-3-5-haiku",
    capabilities = {"chat", "code"}
})

local agents = Agent.where({ active = true }):all()
```

### Common LuaOnBeans Tasks

#### Adding New Routes & Controllers
```lua
-- 1. Define route in config/routes.lua
router.get("/dashboard/custom", "dashboard/custom#index")
router.post("/dashboard/custom", "dashboard/custom#create")

-- 2. Create controller in app/controllers/dashboard/custom_controller.lua
local Controller = require("dashboard.base_controller")
local CustomController = Controller:extend()

function CustomController:index()
    local items = CustomModel.all()
    self:render("dashboard/custom/index", { items = items })
end

function CustomController:create()
    local item = CustomModel:new(CustomModel.permit(self.params.item))
    if item:save() then
        self:redirect_to("/dashboard/custom/" .. item.id)
    else
        self:render("dashboard/custom/new", { item = item })
    end
end

return CustomController
```

#### HTMX Integration
```html
<!-- Progressive enhancement with HTMX -->
<button hx-get="/dashboard/items/new"
        hx-target="#modal-container"
        hx-swap="innerHTML"
        class="btn btn-primary">
  Add Item
</button>

<!-- Form with validation -->
<form hx-post="/dashboard/items"
      hx-target="#items-list"
      hx-swap="innerHTML">
  <input name="item[name]" required>
  <button type="submit">Create</button>
</form>
```

#### API Endpoints
```lua
function ItemsController:api_index()
    local items = Item.all()
    self:json({ items = items })
end

function ItemsController:api_create()
    local item = Item:new(Item.permit(self.params))
    if item:save() then
        self:json(item, 201)
    else
        self:json({ errors = item.errors }, 422)
    end
end
```

## Development Environment Setup

### SolidB Development Setup

```bash
# Install Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup toolchain install stable

# Clone and setup
git clone <repo>
cd solidb

# Install dependencies
cargo build

# Run tests
cargo test

# Start development server
./target/debug/solidb --port 6745 --data-dir ./data
```

### LuaOnBeans Development Setup

```bash
# Ensure Lua 5.4 is available
lua -v  # Should show 5.4.x

# Download redbean
curl -L https://redbean.dev/redbean-3.0.0.com > luaonbeans.org
chmod +x luaonbeans.org

# Start development server with hot reload
./luaonbeans.org
```

### Key Configuration Files

**SolidB:**
- `Cargo.toml` - Dependencies and build configuration
- `src/main.rs` - CLI arguments and server startup
- `src/server/routes.rs` - API route definitions

**LuaOnBeans:**
- `config/routes.lua` - URL routing
- `config/database.lua` - SolidB connection settings
- `.init.lua` - Framework bootstrap

## Cross-Project Integration

### API Communication Patterns

#### LuaOnBeans → SolidB API Calls
```lua
-- In controller methods
function DashboardController:some_action()
    local db = self:get_db()

    -- API call with authentication
    local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collections", {
        method = "GET"
    })

    if status == 200 then
        local data = DecodeJson(body)
        self:render("dashboard/collections", { collections = data.collections })
    else
        self:halt(500, "API call failed")
    end
end
```

#### Authentication Flow
```lua
-- Cookies are automatically included in fetch_api calls
-- JWT tokens stored in sdb_token cookie
local token = GetCookie("sdb_token")
local server_url = GetCookie("sdb_server") or "http://localhost:6745"
```

### Data Synchronization

#### Real-Time Updates (LiveQuery)
```javascript
// Frontend WebSocket connection
const ws = new WebSocket('ws://localhost:6745/_api/ws/changefeed?token=' + token);

// Subscribe to collection changes
ws.send(JSON.stringify({
  type: 'subscribe',
  collection: 'users',
  query: 'FOR doc IN users RETURN doc'
}));
```

#### HTMX Event System
```lua
-- Trigger frontend events from server
SetHeader("HX-Trigger", '{"collectionUpdated": true, "showToast": {"message": "Updated", "type": "success"}}')
```

## Testing & Debugging

### SolidB Testing

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_document_operations

# Run with output
cargo test -- --nocapture

# Run integration tests
cargo test --test http_api_test
```

#### Debug Output
```rust
// Tracing for structured logging
tracing::info!("User {} logged in", username);
tracing::error!("Database connection failed: {}", error);

// Debug endpoints
// GET /_api/stats - Performance metrics
// GET /_api/debug/collections - Collection metadata
```

### LuaOnBeans Testing

```lua
-- Framework test helpers
local Test = require("test")
local describe, it, expect = Test.describe, Test.it, Test.expect

describe("User Model", function()
    it("validates email format", function()
        local user = User:new({ email = "invalid" })
        expect.eq(user:valid(), false)
        expect(user.errors.email).to_exist()
    end)
end)
```

#### Debug Output
```lua
-- Global debug function
P("Debug info", variable, another_var)

-- Framework logging
Log(kLogInfo, "User action: " .. action)
Log(kLogError, "API call failed: " .. error)

-- Check debug.log file for output
```

### Cross-Project Testing

```bash
# Start SolidB in background
./target/release/solidb --port 6745 --data-dir /tmp/solidb-test &
SOLIDB_PID=$!

# Run LuaOnBeans tests against live SolidB
cd www && lua test/run.lua

# Cleanup
kill $SOLIDB_PID
```

## Best Practices & Gotchas

### Performance Considerations

#### SolidB Performance
- Use `spawn_blocking` for CPU-intensive operations
- Implement proper indexing for query performance
- Use connection pooling for client applications
- Monitor memory usage with large datasets

#### LuaOnBeans Performance
- Cache templates in production
- Use pagination for large datasets
- Minimize database queries in loops
- Leverage HTMX for partial updates

### Security Best Practices

#### Input Validation
```rust
// SolidB - Serde validation
#[derive(Deserialize)]
pub struct CreateUserRequest {
    #[serde(rename = "username")]
    pub username: String,
    // Add validation attributes
}
```

```lua
-- LuaOnBeans - Mass assignment protection
local User = Model.create("users", {
    permitted_fields = {"username", "email"},  -- Only allow these
    validations = {
        username = { presence = true },
        email = { format = "email" }
    }
})
```

#### Authentication & Authorization
```rust
// SolidB - Permission checks
if !claims.has_permission("admin") {
    return Err(DbError::PermissionDenied);
}
```

```lua
-- LuaOnBeans - Session validation
if not self.current_user then
    return self:redirect("/login")
end
```

### Common Pitfalls

#### Rust/Lua Interop Issues
- **Memory Management**: Understand Arc vs Clone in Rust
- **Async Boundaries**: Ensure Send bounds for tokio tasks
- **JSON Serialization**: Handle null/None/nil consistently
- **Error Propagation**: Map errors appropriately across boundaries

#### Web Development Gotchas
- **CSRF Protection**: Implement tokens for state-changing requests
- **Session Security**: Use secure, httpOnly cookies
- **XSS Prevention**: Escape user content in templates
- **Rate Limiting**: Implement for API endpoints

#### Lua-Specific Issues
- **1-based Indexing**: Arrays start at index 1
- **Nil vs False**: Both evaluate to false in conditionals
- **String Concatenation**: Use `..` not `+`
- **Global Variables**: Minimize use to avoid conflicts

## Deployment & Production

### SolidB Production Deployment

```bash
# Build optimized release
cargo build --release

# Configure for production
./target/release/solidb \
  --port 6745 \
  --data-dir /var/lib/solidb \
  --log-level info \
  --cluster-secret $(openssl rand -hex 32)
```

#### Production Configuration
- **Resource Limits**: Set appropriate memory/CPU limits
- **Backup Strategy**: Regular data backups with WAL
- **Monitoring**: Enable metrics collection
- **Security**: Use proper TLS certificates

### LuaOnBeans Production Deployment

```bash
# Set production environment
export BEANS_ENV=production

# Start optimized server
./luaonbeans.org -D . -s -d -p 8080 -P luaonbeans.pid
```

#### Production Optimizations
- **Asset Caching**: Long cache headers for static files
- **Template Caching**: Pre-compile templates
- **Database Connection**: Reuse connections efficiently
- **Error Handling**: Custom error pages and logging

### Full Stack Deployment

```bash
# 1. Deploy SolidB backend
systemctl start solidb

# 2. Update LuaOnBeans config
# config/database.lua - Point to production SolidB URL

# 3. Deploy web interface
systemctl start luaonbeans

# 4. Configure reverse proxy (nginx/caddy)
# Proxy /api/* to SolidB
# Proxy /* to LuaOnBeans
```

## Troubleshooting Guide

### Common Issues

#### Authentication Problems
```bash
# Check token validity
curl -H "Authorization: Bearer <token>" http://localhost:6745/_api/status

# Debug session issues
# Check browser cookies for sdb_token
# Verify JWT expiration
```

#### Database Connection Issues
```bash
# Test SolidB connectivity
curl http://localhost:6745/_api/status

# Check LuaOnBeans config
# config/database.lua should point to correct SolidB URL
```

#### Performance Issues
```bash
# SolidB metrics
curl http://localhost:6745/_api/stats

# Check slow queries
# Enable query logging in SolidB
```

#### Build Problems
```bash
# Clean and rebuild
cargo clean && cargo build

# Check Rust version
rustc --version  # Should be 1.70+

# Update dependencies
cargo update
```

### Debug Tools

#### SolidB Debugging
- **Logging**: `RUST_LOG=debug cargo run`
- **Metrics**: `GET /_api/stats` endpoint
- **Query Analysis**: Enable query logging
- **Memory Profiling**: Use `cargo flamegraph`

#### LuaOnBeans Debugging
- **Debug Function**: `P(variable)` for quick inspection
- **Log Files**: Check `debug.log` and server logs
- **Browser Tools**: Network tab for HTMX requests
- **Template Debugging**: Add debug prints in etlua templates

### Getting Help

1. **Check Logs**: Both SolidB and LuaOnBeans produce detailed logs
2. **Reproduce Locally**: Try to reproduce issues in development
3. **Isolate Components**: Test SolidB API directly, then LuaOnBeans
4. **Community Resources**: Check project issues and documentation

---

## Quick Reference

### SolidB Commands
```bash
# Development
cargo build              # Debug build
cargo build --release    # Optimized build
cargo test              # Run tests
cargo clippy            # Lint code
./target/release/solidb --help  # CLI options

# Key endpoints
GET  /_api/status       # Server status
GET  /_api/stats        # Performance metrics
GET  /_api/database/{db}/collections  # List collections
```

### LuaOnBeans Commands
```bash
# Development
./luaonbeans.org        # Start with hot reload
BEANS_ENV=production ./luaonbeans.org  # Production mode

# Key files
config/routes.lua       # Route definitions
config/database.lua     # DB configuration
app/controllers/        # Request handlers
app/views/             # Templates
```

### API Patterns
```rust
// SolidB handler pattern
pub async fn handler(
    State(state): State<AppState>,
    Path(params): Path<Params>,
    Json(req): Json<Request>
) -> Result<Json<Response>, DbError> {
    // Implementation
    Ok(Json(result))
}
```

```lua
-- LuaOnBeans controller pattern
function Controller:action()
    local data = self:fetch_api("/api/endpoint")
    self:render("template", { data = data })
end
```

This guide provides comprehensive coverage of both projects, enabling AI agents to effectively contribute to SolidB and LuaOnBeans development with proper understanding of architecture, patterns, and best practices.</content>
<parameter name="filePath">AI_AGENT_GUIDE.md