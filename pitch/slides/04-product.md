## Product: Powerful & Developer-Friendly

### SDBQL — The query language developers love

```sdbql
FOR doc IN users
  FILTER doc.age > 25 AND doc.status == "active"
  SORT doc.created_at DESC
  LIMIT 20
  RETURN { name: doc.name, email: doc.email, score: doc.score }
```

### Built-in Features

- **Live Queries** — Real-time subscriptions via WebSocket
- **Embedded Lua 5.4** — Write API endpoints directly in the database
- **ACID Transactions** — Configurable isolation levels
- **HNSW Vector Search** — AI-ready embeddings storage
- **Native Sharding** — Horizontal scaling out of the box
- **8 Official Clients** — Rust, Go, Python, Node.js, PHP, Ruby, Elixir, Mobile

### See it in action at solidb.solisoft.net/dashboard
