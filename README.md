# Rust-DB: AQL-Compatible JSON Document Database

A lightweight, high-performance database server written in Rust that implements a subset of ArangoDB's AQL (ArangoDB Query Language) for JSON document storage.

## Features

- 🚀 **Fast & Efficient**: Built with Rust for maximum performance
- 📄 **JSON Document Storage**: Store and query JSON documents with ease
- 🔍 **AQL Query Language**: Familiar query syntax inspired by ArangoDB
- 🌐 **HTTP REST API**: Simple and intuitive API endpoints
- 💾 **RocksDB Storage**: Production-grade persistence with automatic crash recovery
- 🔒 **Thread-Safe**: Concurrent request handling with Tokio
- 📦 **Collections**: Organize documents into collections
- 📊 **Indexing**: Hash and persistent indexes for fast queries
- 🌍 **Geo Queries**: Spatial indexes and distance functions

## Quick Start

### Installation

```bash
# Clone the repository
git clone <repository-url>
cd rust-db

# Build the project
cargo build --release

# Run the server
cargo run --release
```

The server will start on `http://localhost:6745`.

### Basic Usage

#### 1. Create a Collection

```bash
curl -X POST http://localhost:6745/_api/collection \
  -H "Content-Type: application/json" \
  -d '{"name": "users"}'
```

#### 2. Insert Documents

```bash
curl -X POST http://localhost:6745/_api/document/users \
  -H "Content-Type: application/json" \
  -d '{"name": "Alice", "age": 30, "active": true}'

curl -X POST http://localhost:6745/_api/document/users \
  -H "Content-Type: application/json" \
  -d '{"name": "Bob", "age": 25, "active": true}'

curl -X POST http://localhost:6745/_api/document/users \
  -H "Content-Type: application/json" \
  -d '{"name": "Charlie", "age": 35, "active": false}'
```

#### 3. Query with AQL

```bash
# Get all users
curl -X POST http://localhost:6745/_api/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR doc IN users RETURN doc"}'

# Filter by age
curl -X POST http://localhost:6745/_api/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR doc IN users FILTER doc.age > 25 RETURN doc"}'

# Sort and limit
curl -X POST http://localhost:6745/_api/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR doc IN users FILTER doc.active == true SORT doc.age DESC LIMIT 2 RETURN doc"}'

# Project specific fields
curl -X POST http://localhost:6745/_api/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR doc IN users RETURN {name: doc.name, age: doc.age}"}'
```

#### 4. Query with Bind Variables (Secure)

**⚠️ Always use bind variables for user input to prevent AQL injection!**

```bash
# Safe parameterized query
curl -X POST http://localhost:6745/_api/cursor \
  -H "Content-Type: application/json" \
  -d '{
    "query": "FOR doc IN users FILTER doc.name == @name AND doc.age >= @minAge RETURN doc",
    "bindVars": {
      "name": "Alice",
      "minAge": 25
    }
  }'

# Dynamic field access with bind variables
curl -X POST http://localhost:6745/_api/cursor \
  -H "Content-Type: application/json" \
  -d '{
    "query": "FOR doc IN users FILTER doc[@field] == @value RETURN doc",
    "bindVars": {
      "field": "name",
      "value": "Alice"
    }
  }'
```

## AQL Query Syntax

### Supported Clauses

- **LET**: Bind variables, supports subqueries
- **FOR**: Iterate over documents in a collection (multiple FOR = JOIN)
- **FILTER**: Filter documents based on conditions (multiple allowed)
- **SORT**: Sort results by field (ASC/DESC)
- **LIMIT**: Limit and offset results
- **RETURN**: Project and return results

### Query Examples

```aql
-- Basic iteration
FOR doc IN users RETURN doc

-- Filtering
FOR doc IN users FILTER doc.age > 18 RETURN doc
FOR doc IN users FILTER doc.active == true AND doc.age >= 21 RETURN doc

-- Sorting
FOR doc IN users SORT doc.name ASC RETURN doc
FOR doc IN users SORT doc.age DESC RETURN doc

-- Limiting
FOR doc IN users LIMIT 10 RETURN doc
FOR doc IN users LIMIT 5, 10 RETURN doc  -- offset 5, count 10

-- Projection
FOR doc IN users RETURN {name: doc.name, email: doc.email}

-- Complex queries
FOR doc IN users
  FILTER doc.age > 25 AND doc.active == true
  SORT doc.name ASC
  LIMIT 10
  RETURN {name: doc.name, age: doc.age}

-- JOIN: Users with their Orders
FOR u IN users
  FOR o IN orders
    FILTER o.user_key == u._key
    RETURN {user: u.name, product: o.product, amount: o.amount}

-- JOIN with multiple filters and sorting
FOR u IN users
  FOR o IN orders
    FILTER o.user_key == u._key
    FILTER o.amount > 50
    SORT o.amount DESC
    LIMIT 10
    RETURN {user: u.name, product: o.product, amount: o.amount}

-- Cross Join (Cartesian product)
FOR u IN users
  FOR p IN products
    RETURN {user: u.name, product: p.name}

-- LET with literal value
LET minAge = 25
FOR doc IN users
  FILTER doc.age >= minAge
  RETURN doc

-- LET with subquery
LET activeUsers = (FOR u IN users FILTER u.active == true RETURN u)
FOR user IN activeUsers
  RETURN user.name

-- Correlated subquery (LET inside FOR with access to outer variable)
FOR u IN users
  LET userOrders = (FOR o IN orders FILTER o.user == u.name RETURN o.product)
  RETURN { name: u.name, orders: userOrders }

-- Correlated subquery with aggregation
FOR u IN users
  LET totalSpent = SUM((FOR o IN orders FILTER o.user == u.name RETURN o.amount))
  FILTER totalSpent > 100
  RETURN { name: u.name, spent: totalSpent }

-- Multiple LET clauses
LET seniors = (FOR u IN users FILTER u.age > 30 RETURN u)
LET juniors = (FOR u IN users FILTER u.age <= 30 RETURN u)
FOR s IN seniors
  RETURN {name: s.name, category: "senior"}

-- LET with array literal
LET items = [1, 2, 3]
FOR x IN items
  RETURN x * 2
```

### Supported Operators

**Comparison**: `==`, `!=`, `<`, `<=`, `>`, `>=`
**Logical**: `AND`, `OR`, `NOT`
**Arithmetic**: `+`, `-`, `*`, `/`

## REST API Reference

### Collections

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/_api/collection` | Create a collection |
| GET | `/_api/collection` | List all collections |
| DELETE | `/_api/collection/:name` | Delete a collection |

### Documents

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/_api/document/:collection` | Insert a document |
| GET | `/_api/document/:collection/:key` | Get a document |
| PUT | `/_api/document/:collection/:key` | Update a document |
| DELETE | `/_api/document/:collection/:key` | Delete a document |

### Queries

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/_api/cursor` | Execute an AQL query |
| POST | `/_api/explain` | Explain/profile an AQL query |

#### Explain Query

Get detailed execution plan with timing for each step:

```bash
curl -X POST http://localhost:6745/_api/explain \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR u IN users FILTER u.age > 25 SORT u.name RETURN u"}'
```

Returns timing (in microseconds) for each step, index usage analysis, and optimization suggestions.

### Indexes

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/_api/index/:collection` | Create an index |
| GET | `/_api/index/:collection` | List indexes on collection |
| DELETE | `/_api/index/:collection/:name` | Delete an index |

#### Create Index Request

```json
{
  "name": "idx_age",
  "field": "age",
  "type": "persistent",
  "unique": false
}
```

**Index Types:**
- `hash` - Fast equality lookups (`field == value`)
- `persistent` - Range queries and sorting (`field > value`, `SORT field`)
- `geo` - Geographic/spatial queries (near, within radius)
- `fulltext` - N-gram based text search with fuzzy matching (Levenshtein distance)

#### Example: Using Indexes

```bash
# Create a persistent index on the 'age' field
curl -X POST http://localhost:6745/_api/index/users \
  -H "Content-Type: application/json" \
  -d '{"name": "idx_age", "field": "age", "type": "persistent"}'

# This query will now use the index automatically
curl -X POST http://localhost:6745/_api/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR doc IN users FILTER doc.age > 25 RETURN doc"}'
```

### Geo Indexes

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/_api/geo/:collection` | Create a geo index |
| GET | `/_api/geo/:collection` | List geo indexes |
| DELETE | `/_api/geo/:collection/:name` | Delete a geo index |
| POST | `/_api/geo/:collection/:field/near` | Find documents near a point |
| POST | `/_api/geo/:collection/:field/within` | Find documents within radius |

#### Create Geo Index Request

```json
{
  "name": "idx_location",
  "field": "location"
}
```

The field should contain coordinates in one of these formats:
- Object: `{"lat": 48.8584, "lon": 2.2945}`
- Array (GeoJSON): `[2.2945, 48.8584]` (longitude, latitude)

#### Geo Near Query

```bash
# Find 5 nearest places to the Eiffel Tower
curl -X POST http://localhost:6745/_api/geo/places/location/near \
  -H "Content-Type: application/json" \
  -d '{"lat": 48.8584, "lon": 2.2945, "limit": 5}'
```

#### Geo Within Query

```bash
# Find places within 2km of the Eiffel Tower
curl -X POST http://localhost:6745/_api/geo/places/location/within \
  -H "Content-Type: application/json" \
  -d '{"lat": 48.8584, "lon": 2.2945, "radius": 2000}'
```

#### AQL Geo Functions

```aql
-- Calculate distance between two points (in meters)
DISTANCE(lat1, lon1, lat2, lon2)

-- Calculate distance between two geo points
GEO_DISTANCE(point1, point2)

-- Example: Get places with distance from Eiffel Tower
FOR p IN places
  RETURN {
    name: p.name,
    distance: ROUND(DISTANCE(p.location.lat, p.location.lon, 48.8584, 2.2945))
  }
```

### Fulltext Indexes

Fulltext indexes enable fuzzy text search using n-gram indexing and Levenshtein distance.

#### Create Fulltext Index

```bash
curl -X POST http://localhost:6745/_api/index/articles \
  -H "Content-Type: application/json" \
  -d '{"name": "ft_title", "field": "title", "type": "fulltext"}'
```

#### AQL Fulltext Functions

```aql
-- Search for documents matching a query with fuzzy matching
-- FULLTEXT(collection, field, query, maxDistance?)
LET matches = FULLTEXT("articles", "title", "rust programming")
FOR m IN matches
  RETURN { doc: m.doc, score: m.score }

-- With custom Levenshtein distance (default is 2)
LET results = FULLTEXT("articles", "title", "pythn", 3)
FOR r IN results
  RETURN r.doc

-- Calculate Levenshtein distance between two strings
LEVENSHTEIN("hello", "hallo")  -- Returns 1
LEVENSHTEIN("rust", "rest")    -- Returns 1
LEVENSHTEIN("test", "test")    -- Returns 0
```

**Fulltext Search Response:**
Each match includes:
- `doc`: The full document
- `score`: Relevance score (higher is better)
- `matched`: Array of matched terms

## Architecture

```
┌─────────────────────────────────────────┐
│           HTTP REST API                 │
│         (Axum + Tokio)                  │
└─────────────────┬───────────────────────┘
                  │
┌─────────────────▼───────────────────────┐
│         Query Executor                  │
│    (AQL Parser + Evaluator)             │
└─────────────────┬───────────────────────┘
                  │
┌─────────────────▼───────────────────────┐
│        Storage Engine                   │
│           (RocksDB)                     │
└─────────────────────────────────────────┘
```

### Components

- **Storage Engine**: RocksDB-backed storage with column families per collection
- **AQL Parser**: Lexer and parser for AQL query language
- **Query Executor**: Executes parsed queries against the storage engine
- **HTTP Server**: REST API built with Axum and Tokio
- **Indexes**: Hash and Persistent indexes stored in RocksDB

## Data Persistence

Data is stored using **RocksDB**, a high-performance embedded key-value store developed by Meta/Facebook.

### Storage Structure

```
./data/
  ├── *.sst           # Sorted String Table files (actual data)
  ├── *.log           # Write-Ahead Log for crash recovery
  ├── MANIFEST-*      # Database manifest
  ├── CURRENT         # Current manifest pointer
  └── OPTIONS-*       # RocksDB configuration
```

### Benefits of RocksDB

- **Automatic persistence**: Data is durably stored immediately
- **Crash recovery**: Write-Ahead Log ensures no data loss
- **Compression**: Built-in LZ4/Snappy/Zstd compression
- **Column Families**: Each collection is a separate column family
- **LSM Tree**: Optimized for write-heavy workloads
- **No manual saves**: Unlike JSON files, no explicit save needed

## Development

### Running Tests

```bash
cargo test
```

### Running with Debug Logs

```bash
RUST_LOG=debug cargo run
```

### Building for Production

```bash
cargo build --release
./target/release/rust-db
```

## Limitations

This is an initial implementation focusing on core functionality. Current limitations:

- No graph queries
- No complex aggregations (GROUP BY, etc.)
- No authentication/authorization
- Single-node only (no clustering)

## Future Enhancements

- [x] ~~Indexing for faster queries~~ ✅ Implemented!
- [x] ~~JOIN operations~~ ✅ Implemented! (nested FOR loops)
- [x] ~~Geo indexing~~ ✅ Implemented! (DISTANCE, GEO_DISTANCE, near, within)
- [x] ~~LET clauses & Subqueries~~ ✅ Implemented! (LET x = (FOR ... RETURN ...))
- [x] ~~Built-in functions~~ ✅ Implemented! (LENGTH, ROUND, ABS, UPPER, LOWER, CONCAT, SUBSTRING, etc.)
- [x] ~~RocksDB storage backend~~ ✅ Implemented! (crash recovery, compression, LSM tree)
- [x] ~~Bind Variables~~ ✅ Implemented! (@variable for AQL injection prevention)
- [ ] Aggregation functions (COUNT, SUM, AVG, etc.)
- [ ] Graph traversal queries
- [ ] Authentication and authorization
- [ ] WebSocket support
- [ ] Query optimization
- [ ] Replication and clustering

## License

MIT

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
