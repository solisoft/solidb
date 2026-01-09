# Server Module

## Purpose
HTTP API server built on Axum. Handles all REST endpoints, authentication, authorization, WebSocket connections, and request routing.

## Key Files

| File | Size | Description |
|------|------|-------------|
| `handlers.rs` | 248KB | Main API handlers (documents, collections, queries, indexes) |
| `ai_handlers.rs` | 50KB | AI contribution, task, and agent management endpoints |
| `auth.rs` | 43KB | JWT authentication, user management, login/logout |
| `authorization.rs` | 18KB | RBAC permission system, roles, scopes |
| `routes.rs` | 23KB | Route definitions and middleware setup |
| `script_handlers.rs` | 21KB | Lua script execution endpoints |
| `role_handlers.rs` | 28KB | Role and permission CRUD endpoints |
| `columnar_handlers.rs` | 21KB | Columnar storage API endpoints |
| `transaction_handlers.rs` | 18KB | ACID transaction endpoints |
| `queue_handlers.rs` | 10KB | Background job queue endpoints |
| `permission_cache.rs` | 10KB | In-memory permission caching |
| `repl_session.rs` | 7KB | REPL session state management |
| `multiplex.rs` | 6KB | Protocol multiplexing (HTTP + binary driver) |
| `sql_handlers.rs` | 4KB | SQL query translation endpoint |
| `env_handlers.rs` | 3KB | Environment variable storage |
| `managed_agent_template.rs` | 4KB | Lua template for managed AI agents |

## Architecture

### Route Organization
Routes are defined in `routes.rs` under prefixes:
- `/_api/database/{db}/...` - Database-scoped operations
- `/_api/blob/{db}/...` - Binary blob storage
- `/_api/cursor/{id}` - Query cursor pagination
- `/auth/...` - Authentication (login, logout, refresh)
- `/ws/...` - WebSocket connections (LiveQuery)

### AppState
Shared state passed to all handlers:
```rust
pub struct AppState {
    storage: Arc<StorageEngine>,
    cursor_store: CursorStore,
    cluster_manager: Option<Arc<ClusterManager>>,
    shard_coordinator: Option<Arc<ShardCoordinator>>,
    queue_worker: Option<Arc<QueueWorker>>,
    permission_cache: PermissionCache,
    repl_sessions: ReplSessionStore,
}
```

### Authentication Flow
1. `POST /auth/login` - Returns JWT token
2. Token passed in `Authorization: Bearer <token>` header
3. `auth::jwt_auth_middleware` validates token, extracts Claims
4. Claims available via `Extension<Claims>` in handlers

### Authorization (RBAC)
- Roles defined in `_roles` collection
- User-role mappings in `_user_roles` collection
- Built-in roles: `admin`, `developer`, `reader`
- Permissions: `PermissionAction` (Read, Write, Admin) + `PermissionScope` (Database, Collection)

## Common Tasks

### Adding a New Endpoint
1. Add handler function in appropriate `*_handlers.rs`
2. Add route in `routes.rs` under correct section
3. Apply auth middleware if needed: `.layer(middleware::from_fn(jwt_auth_middleware))`

### Adding a New Handler File
1. Create `new_handlers.rs`
2. Add `pub mod new_handlers;` to `mod.rs`
3. Import handlers in `routes.rs`

### Debugging Auth Issues
- Check `auth.rs` for JWT validation
- Check `permission_cache.rs` for cached permissions
- Verify role exists in `_system/_roles` collection

## Dependencies
- **Uses**: `storage::StorageEngine`, `sdbql::Executor`, `scripting` for Lua
- **Used by**: `main.rs` creates router via `create_router()`

## Gotchas
- `handlers.rs` is very large (248KB) - search for handler by route path
- JWT secret from `JWT_SECRET` env var or defaults to "secret" (warn in logs)
- System collections (`_users`, `_roles`, etc.) created on startup in `_system` db
- WebSocket LiveQuery uses separate auth token endpoint: `/_api/livequery/token`
