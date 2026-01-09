# Scripting Module

## Purpose
Embedded Lua 5.4 runtime for custom endpoints, stored procedures, and AI agent scripts. Provides safe sandboxed execution with access to database operations.

## Key Files

| File | Lines | Description |
|------|-------|-------------|
| `mod.rs` | 2,362 | Main Lua VM setup, script execution, REPL sessions |
| `ai_bindings.rs` | 1,001 | AI/ML bindings (vector ops, embeddings, chat) |
| `file_handling.rs` | 655 | File I/O operations (restricted paths) |
| `dev_tools.rs` | 462 | Development utilities (debugging, profiling) |
| `string_utils.rs` | 356 | String manipulation extensions |
| `http_helpers.rs` | 322 | HTTP client bindings (fetch, request) |
| `auth.rs` | 324 | Authentication helpers (JWT, hashing) |
| `error_handling.rs` | 317 | Error capture and formatting |
| `validation.rs` | 284 | Input validation utilities |

## Architecture

### Script Execution Flow
```
Script Code → Lua VM → SoliDB Bindings → Storage/Query → Result
```

### Global Objects Available in Lua

#### `solidb` - Database Operations
```lua
solidb.query("FOR doc IN users RETURN doc")
solidb.get("users", "key123")
solidb.insert("users", { name = "John" })
solidb.update("users", "key123", { active = true })
solidb.delete("users", "key123")
solidb.count("users")
```

#### `db` - Collection Handle Factory
```lua
local users = db:collection("users")
users:insert({ name = "John" })
users:get("key123")
users:update("key123", { active = true })
users:delete("key123")
users:count()
```

#### `request` - HTTP Request Context
```lua
request.method    -- "GET", "POST", etc.
request.path      -- "/api/users"
request.headers   -- { ["Content-Type"] = "application/json" }
request.params    -- URL parameters
request.body      -- Request body (string or parsed JSON)
```

#### `response` - HTTP Response Builder
```lua
response:status(200)
response:header("X-Custom", "value")
response:json({ success = true })
response:send("plain text")
```

#### `time` - Time Operations
```lua
time.now()           -- Current Unix timestamp
time.millis()        -- Milliseconds since epoch
time.date(ts)        -- Format timestamp
time.parse(str)      -- Parse date string
time.iso(ts)         -- ISO 8601 format
time.diff(t1, t2)    -- Difference in seconds
time.add(ts, secs)   -- Add seconds
```

#### `table` - Extended Table Functions
```lua
table.keys(t)        -- Get all keys
table.values(t)      -- Get all values
table.merge(t1, t2)  -- Merge tables
table.filter(t, fn)  -- Filter by predicate
table.map(t, fn)     -- Transform values
table.find(t, fn)    -- Find first match
table.contains(t, v) -- Check membership
table.sorted(t)      -- Return sorted copy
```

## REPL Sessions (mod.rs)

Interactive Lua REPL with session persistence:
```rust
pub struct ReplSession {
    pub id: String,
    pub variables: HashMap<String, JsonValue>,  // Persisted variables
    pub history: Vec<String>,                   // Command history
}
```

Key behaviors:
- Variables persist across executions (except `local`)
- Functions replayed from history (can't serialize Lua functions)
- `db` object recreated each execution (not captured in session)
- Collection handles marked with `_solidb_handle` for recreation

## Common Tasks

### Adding a New Lua Function
1. Add function in appropriate helper file (e.g., `string_utils.rs`)
2. Register in `mod.rs` under the relevant global object
3. Use `lua.create_function()` with proper error handling

### Adding a Global Object
1. Create table: `let obj = lua.create_table()?`
2. Add methods: `obj.set("method", lua.create_function(...))?`
3. Set global: `globals.set("objname", obj)?`

### Debugging Script Execution
1. Check `execute_script()` in `mod.rs` for entry point
2. Use `print()` in Lua - captured in response output
3. Check `error_handling.rs` for error formatting

## Dependencies
- **Uses**: `mlua` crate, `storage::StorageEngine`, `sdbql::Executor`
- **Used by**: `server::script_handlers`, AI agent workers

## Gotchas
- `mod.rs` is 2,362 lines - main execution at top, bindings below
- Skip globals list prevents capturing system objects in REPL sessions
- Collection handles need special handling (can't serialize functions)
- `os` library disabled for security (use `time` instead)
- Scripts have 30-second default timeout
- AI bindings require external API keys in `_env` collection
