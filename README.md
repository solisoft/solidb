# SoliDB: AQL-Compatible JSON Document Database

A lightweight, high-performance database server written in Rust that implements a subset of ArangoDB's AQL (ArangoDB Query Language) for JSON document storage.

https://github.com/user-attachments/assets/aa64e937-39b8-42ca-8ee5-beb7dac90c23

[![CI](https://github.com/solisoft/solidb/actions/workflows/ci.yml/badge.svg)](https://github.com/solisoft/solidb/actions/workflows/ci.yml)

## Features

- 🚀 **Fast & Efficient**: Built with Rust for maximum performance
- 📄 **JSON Document Storage**: Store and query JSON documents with ease
- 🔍 **AQL Query Language**: Familiar query syntax inspired by ArangoDB
- 🌐 **HTTP REST API**: Simple and intuitive API endpoints
- 💾 **RocksDB Storage**: Production-grade persistence with automatic crash recovery
- 🔒 **Thread-Safe**: Concurrent request handling with Tokio
- 🔐 **JWT Authentication**: Secure API access with Bearer tokens
- 🗄️ **Multi-Database**: Multiple isolated databases with collections
- 📦 **Collections**: Organize documents into collections
- 📊 **Indexing**: Hash and persistent indexes for fast queries
- 🌍 **Geo Queries**: Spatial indexes and distance functions
- 🔄 **Multi-Node Replication**: Peer-to-peer replication with automatic sync
- ⚡ **Hybrid Logical Clocks**: Consistent ordering across distributed nodes
- 🧩 **Sharding**: Horizontal data partitioning with configurable shard count
- ⚖️ **Auto-Rebalancing**: Automatic data redistribution when nodes change
- 💳 **Transactions**: ACID transactions via X-Transaction-ID header

## Quick Start

### Installation

```bash
# Clone the repository
git clone <repository-url>
cd solidb

# Build and install system-wide
cargo install --path .

# The solidb command is now available globally
solidb --help
```

**Alternative: Build without installing**

```bash
# Build the project
cargo build --release

# Run from the target directory
./target/release/solidb

# Or run with cargo
cargo run --release
```

The server will start on `http://localhost:6745`.

### Ubuntu/Debian Build Requirements

Before building on Ubuntu or Debian-based systems, install the required development libraries:

```bash
# Install all required dependencies
sudo apt-get update
sudo apt-get install -y \
    build-essential \
    clang \
    libclang-dev \
    pkg-config \
    libssl-dev \
    libzstd-dev
```

**Required packages:**
- `build-essential` - GCC compiler and build tools
- `clang` & `libclang-dev` - Clang compiler (required by RocksDB)
- `pkg-config` - Package configuration tool (required by openssl-sys)
- `libssl-dev` - OpenSSL development libraries (required by reqwest/HTTPS)
- `libzstd-dev` - Zstandard compression library (required by RocksDB)

**Note**: Without these packages, compilation will fail with errors like:
- `failed to run custom build command for zstd-sys` (missing libzstd-dev)
- `failed to run custom build command for openssl-sys` (missing pkg-config or libssl-dev)

### Arch Linux Build Requirements

On Arch Linux, install the required dependencies:

```bash
# Install all required dependencies
sudo pacman -Syu
sudo pacman -S base-devel clang gcc pkg-config openssl zstd
```

**Required packages:**
- `base-devel` - Development tools (includes make, etc.)
- `clang` - Clang compiler (required by RocksDB)
- `gcc` - GCC C++ compiler and standard library (required for C++ compilation)
- `pkg-config` - Package configuration tool
- `openssl` - OpenSSL library (includes development headers)
- `zstd` - Zstandard compression library

**Note**: Both `clang` and `gcc` are needed because RocksDB uses C++17 features that require the GCC C++ standard library (`libstdc++`).

### Command Line Options

```bash
solidb [OPTIONS]

Options:
  -p, --port <PORT>              Port to listen on [default: 6745]
      --node-id <NODE_ID>        Unique node identifier (auto-generated if not provided)
      --peer <PEER>              Peer nodes to replicate with (can be repeated)
      --replication-port <PORT>  Port for inter-node replication [default: 6746]
      --data-dir <PATH>          Data directory path [default: ./data]
  -d, --daemon                   Run as a daemon (background process)
      --pid-file <PATH>          PID file path (used in daemon mode) [default: ./solidb.pid]
      --log-file <PATH>          Log file path (used in daemon mode) [default: ./solidb.log]
```

### Daemon Mode

Run SoliDB as a background daemon process (Unix/Linux only):

```bash
# Start as daemon with default settings
solidb --daemon

# Start daemon with custom paths
solidb --daemon --pid-file /var/run/solidb.pid --log-file /var/log/solidb.log

# Start daemon with custom port and data directory
solidb --daemon --port 8080 --data-dir /var/lib/solidb
```

**Managing the daemon:**

```bash
# Check if daemon is running
ps aux | grep solidb

# View daemon logs
tail -f ./solidb.log

# Stop the daemon gracefully
kill -TERM $(cat ./solidb.pid)

# Force stop the daemon
kill -9 $(cat ./solidb.pid)
```

**Note**: The daemon mode uses PID file locking to prevent multiple instances from running simultaneously.


### Single Node Mode

```bash
# Run a single server (default)
solidb

# Or with cargo during development
cargo run --release
```

### Cluster Mode

```bash
# Node 1 (initial node)
solidb --data-dir ./data1 --port 6745 --replication-port 6746

# Node 2 (joins the cluster)
solidb --data-dir ./data2 --port 6755 --replication-port 6756 --peer 127.0.0.1:6746

# Node 3 (joins via any existing node)
solidb --data-dir ./data3 --port 6765 --replication-port 6766 --peer 127.0.0.1:6746
```

### Basic Usage

> **Note**: The server automatically creates a `_system` database on startup. A default admin user (`admin`/`admin`) is also created. All API endpoints under `/_api/` require authentication.

#### 1. Login (Get JWT Token)

```bash
# Get authentication token
curl -X POST http://localhost:6745/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username": "admin", "password": "admin"}'
# Response: {"token": "eyJ..."}

# Store token for subsequent requests
export TOKEN="your-jwt-token-here"
```

#### 2. Create a Collection

```bash
curl -X POST http://localhost:6745/_api/database/_system/collection \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"name": "users"}'
```

#### 3. Insert Documents

```bash
curl -X POST http://localhost:6745/_api/database/_system/document/users \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"name": "Alice", "age": 30, "active": true}'

curl -X POST http://localhost:6745/_api/database/_system/document/users \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"name": "Bob", "age": 25, "active": true}'

curl -X POST http://localhost:6745/_api/database/_system/document/users \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"name": "Charlie", "age": 35, "active": false}'
```

#### 4. Query with AQL

```bash
# Get all users
curl -X POST http://localhost:6745/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR doc IN users RETURN doc"}'

# Filter by age
curl -X POST http://localhost:6745/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR doc IN users FILTER doc.age > 25 RETURN doc"}'

# Sort and limit
curl -X POST http://localhost:6745/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR doc IN users FILTER doc.active == true SORT doc.age DESC LIMIT 2 RETURN doc"}'

# Project specific fields
curl -X POST http://localhost:6745/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR doc IN users RETURN {name: doc.name, age: doc.age}"}'
```

#### 4. Query with Bind Variables (Secure)

**⚠️ Always use bind variables for user input to prevent AQL injection!**

```bash
# Safe parameterized query
curl -X POST http://localhost:6745/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d '{
    "query": "FOR doc IN users FILTER doc.name == @name AND doc.age >= @minAge RETURN doc",
    "bindVars": {
      "name": "Alice",
      "minAge": 25
    }
  }'

# Dynamic field access with bind variables
curl -X POST http://localhost:6745/_api/database/_system/cursor \
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

```ruby
# Basic iteration
FOR doc IN users RETURN doc

# Filtering
FOR doc IN users FILTER doc.age > 18 RETURN doc
FOR doc IN users FILTER doc.active == true AND doc.age >= 21 RETURN doc

# Sorting
FOR doc IN users SORT doc.name ASC RETURN doc
FOR doc IN users SORT doc.age DESC RETURN doc

# Limiting
FOR doc IN users LIMIT 10 RETURN doc
FOR doc IN users LIMIT 5, 10 RETURN doc  -- offset 5, count 10

# Projection
FOR doc IN users RETURN {name: doc.name, email: doc.email}

# Complex queries
FOR doc IN users
  FILTER doc.age > 25 AND doc.active == true
  SORT doc.name ASC
  LIMIT 10
  RETURN {name: doc.name, age: doc.age}

# JOIN: Users with their Orders
FOR u IN users
  FOR o IN orders
    FILTER o.user_key == u._key
    RETURN {user: u.name, product: o.product, amount: o.amount}

# JOIN with multiple filters and sorting
FOR u IN users
  FOR o IN orders
    FILTER o.user_key == u._key
    FILTER o.amount > 50
    SORT o.amount DESC
    LIMIT 10
    RETURN {user: u.name, product: o.product, amount: o.amount}

# Cross Join (Cartesian product)
FOR u IN users
  FOR p IN products
    RETURN {user: u.name, product: p.name}

# LET with literal value
LET minAge = 25
FOR doc IN users
  FILTER doc.age >= minAge
  RETURN doc

# LET with subquery
LET activeUsers = (FOR u IN users FILTER u.active == true RETURN u)
FOR user IN activeUsers
  RETURN user.name

# Correlated subquery (LET inside FOR with access to outer variable)
FOR u IN users
  LET userOrders = (FOR o IN orders FILTER o.user == u.name RETURN o.product)
  RETURN { name: u.name, orders: userOrders }

# Correlated subquery with aggregation
FOR u IN users
  LET totalSpent = SUM((FOR o IN orders FILTER o.user == u.name RETURN o.amount))
  FILTER totalSpent > 100
  RETURN { name: u.name, spent: totalSpent }

# Multiple LET clauses
LET seniors = (FOR u IN users FILTER u.age > 30 RETURN u)
LET juniors = (FOR u IN users FILTER u.age <= 30 RETURN u)
FOR s IN seniors
  RETURN {name: s.name, category: "senior"}

# LET with array literal
LET items = [1, 2, 3]
FOR x IN items
  RETURN x * 2
```

### Supported Operators

**Comparison**: `==`, `!=`, `<`, `<=`, `>`, `>=`
**Logical**: `AND`, `OR`, `NOT`
**Arithmetic**: `+`, `-`, `*`, `/`

### Aggregation Functions

```aql
-- Basic aggregations (operate on arrays)
SUM(array)          -- Sum of numeric values
AVG(array)          -- Average of numeric values
MIN(array)          -- Minimum value
MAX(array)          -- Maximum value
COUNT(array)        -- Number of elements
COUNT_DISTINCT(array) -- Number of unique values

-- Statistical functions
MEDIAN(array)           -- Median value
VARIANCE(array)         -- Population variance
VARIANCE_SAMPLE(array)  -- Sample variance
STDDEV(array)           -- Sample standard deviation
STDDEV_POPULATION(array) -- Population standard deviation
PERCENTILE(array, p)    -- Percentile value (p: 0-100)

-- Example: Get average order amount per user
FOR u IN users
  LET orderAmounts = (FOR o IN orders FILTER o.user == u._key RETURN o.amount)
  RETURN {
    user: u.name,
    total: SUM(orderAmounts),
    avg: AVG(orderAmounts),
    count: COUNT(orderAmounts),
    median: MEDIAN(orderAmounts)
  }
```

### Array Functions

```aql
-- Access functions
FIRST(array)          -- First element
LAST(array)           -- Last element
NTH(array, n)         -- Element at index n (0-based)
SLICE(array, start, length?) -- Slice array

-- Transformation functions
UNIQUE(array)         -- Remove duplicates
SORTED(array)         -- Sort ascending
SORTED_UNIQUE(array)  -- Sort and remove duplicates
REVERSE(array)        -- Reverse array
FLATTEN(array, depth?) -- Flatten nested arrays

-- Combination functions
PUSH(array, element, unique?) -- Add element
APPEND(array1, array2, unique?) -- Concatenate arrays
UNION(array1, array2, ...)      -- Union (unique values)
MINUS(array1, array2)           -- Difference
INTERSECTION(array1, array2, ...) -- Common elements

-- Search functions
POSITION(array, element)    -- Find index (-1 if not found)
CONTAINS_ARRAY(array, element) -- Check if contains

-- Example: Combine and deduplicate tags
LET tags1 = ["rust", "database"]
LET tags2 = ["database", "nosql"]
RETURN UNION(tags1, tags2)  -- ["rust", "database", "nosql"]
```

## REST API Reference

> **Note**: All `/_api/*` endpoints require authentication. Include the header `Authorization: Bearer <token>` in your requests.

### Authentication

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/auth/login` | Login and get JWT token |
| PUT | `/_api/auth/password` | Change password (requires auth) |
| POST | `/_api/auth/api-keys` | Create API key (for server-to-server) |
| GET | `/_api/auth/api-keys` | List API keys |
| DELETE | `/_api/auth/api-keys/:id` | Revoke an API key |

#### Login

```bash
curl -X POST http://localhost:6745/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username": "admin", "password": "admin"}'
```

**Response:**
```json
{"token": "eyJhbGciOiJIUzI1NiJ9..."}
```

#### Change Password

```bash
curl -X PUT http://localhost:6745/_api/auth/password \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"current_password": "admin", "new_password": "newpassword"}'
```

#### API Keys (Server-to-Server)

API keys are non-expiring tokens ideal for backend services and automation.

```bash
# Create an API key
curl -X POST http://localhost:6745/_api/auth/api-keys \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name": "my-backend-service"}'
# Response: {"id": "...", "name": "my-backend-service", "key": "sk_abc123..."}

# Use the API key (either header works)
curl -H "X-API-Key: sk_abc123..." http://localhost:6745/_api/databases
# OR
curl -H "Authorization: ApiKey sk_abc123..." http://localhost:6745/_api/databases

# List keys (shows names only, not the actual keys)
curl http://localhost:6745/_api/auth/api-keys -H "Authorization: Bearer $TOKEN"

# Revoke a key
curl -X DELETE http://localhost:6745/_api/auth/api-keys/<key-id> \
  -H "Authorization: Bearer $TOKEN"
```

**Notes:**
- The default admin user (`admin`/`admin`) is created automatically on first startup
- JWT tokens expire after 24 hours; API keys never expire (unless revoked)
- If the `_admins` collection is deleted, a default admin is recreated on next login attempt

### Databases

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/_api/database` | Create a database |
| GET | `/_api/databases` | List all databases |
| DELETE | `/_api/database/:name` | Delete a database |

**Note**: The `_system` database is created automatically and cannot be deleted.

### Collections

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/_api/database/:db/collection` | Create a collection |
| GET | `/_api/database/:db/collection` | List all collections |
| DELETE | `/_api/database/:db/collection/:name` | Delete a collection |
| PUT | `/_api/database/:db/collection/:name/truncate` | Truncate a collection |

### Documents

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/_api/database/:db/document/:collection` | Insert a document |
| GET | `/_api/database/:db/document/:collection/:key` | Get a document |
| PUT | `/_api/database/:db/document/:collection/:key` | Update a document |
| DELETE | `/_api/database/:db/document/:collection/:key` | Delete a document |

### Queries

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/_api/database/:db/cursor` | Execute an AQL query |
| POST | `/_api/database/:db/explain` | Explain/profile an AQL query |
| PUT | `/_api/cursor/:id` | Get next batch from cursor |
| DELETE | `/_api/cursor/:id` | Delete cursor |

#### Explain Query

Get detailed execution plan with timing for each step:

```bash
curl -X POST http://localhost:6745/_api/database/_system/explain \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR u IN users FILTER u.age > 25 SORT u.name RETURN u"}'
```

Returns timing (in microseconds) for each step, index usage analysis, and optimization suggestions.

### Indexes

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/_api/database/:db/index/:collection` | Create an index |
| GET | `/_api/database/:db/index/:collection` | List indexes on collection |
| DELETE | `/_api/database/:db/index/:collection/:name` | Delete an index |

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
curl -X POST http://localhost:6745/_api/database/_system/index/users \
  -H "Content-Type: application/json" \
  -d '{"name": "idx_age", "field": "age", "type": "persistent"}'

# This query will now use the index automatically
curl -X POST http://localhost:6745/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR doc IN users FILTER doc.age > 25 RETURN doc"}'
```

### Geo Indexes

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/_api/database/:db/geo/:collection` | Create a geo index |
| GET | `/_api/database/:db/geo/:collection` | List geo indexes |
| DELETE | `/_api/database/:db/geo/:collection/:name` | Delete a geo index |
| POST | `/_api/database/:db/geo/:collection/:field/near` | Find documents near a point |
| POST | `/_api/database/:db/geo/:collection/:field/within` | Find documents within radius |

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
curl -X POST http://localhost:6745/_api/database/_system/geo/places/location/near \
  -H "Content-Type: application/json" \
  -d '{"lat": 48.8584, "lon": 2.2945, "limit": 5}'
```

#### Geo Within Query

```bash
# Find places within 2km of the Eiffel Tower
curl -X POST http://localhost:6745/_api/database/_system/geo/places/location/within \
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
curl -X POST http://localhost:6745/_api/database/_system/index/articles \
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

-- BM25 relevance scoring (NEW!)
-- BM25(field, query) - Returns relevance score for ranking
FOR doc IN articles
  SORT BM25(doc.content, "machine learning") DESC
  LIMIT 10
  RETURN {title: doc.title, score: BM25(doc.content, "machine learning")}

-- Combined with filters
FOR doc IN articles
  FILTER doc.published == true
  SORT BM25(doc.content, "rust database") DESC
  LIMIT 5
  RETURN doc
```

#### AQL Date Functions

```aql
-- Get current timestamp in milliseconds
DATE_NOW()

-- Convert timestamp to ISO 8601 string
DATE_ISO8601(timestamp)

-- Example: Get events from last 24 hours with formatted date
FOR doc IN events
  LET eventDate = DATE_ISO8601(doc.timestamp)
  FILTER doc.timestamp > DATE_NOW() - 86400000
  RETURN { event: doc.name, date: eventDate }
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
│    (Multi-Database + RocksDB)           │
└─────────────────────────────────────────┘
```

### Components

- **Storage Engine**: Multi-database architecture with RocksDB backend
- **Database Layer**: Isolated databases, each containing collections
- **Collections**: Column families in RocksDB with naming format `{database}:{collection}`
- **AQL Parser**: Lexer and parser for AQL query language
- **Query Executor**: Executes parsed queries with smart collection lookup
- **HTTP Server**: REST API built with Axum and Tokio
- **Indexes**: Hash, Persistent, Geo, and Fulltext indexes stored in RocksDB

## Data Persistence

Data is stored using **RocksDB**, a high-performance embedded key-value store developed by Meta/Facebook.

### Storage Structure

```
./data/
  ├── *.sst           # Sorted String Table files (actual data)
  ├── *.log           # Write-Ahead Log for crash recovery
  ├── MANIFEST-*      # Database manifest
  ├── CURRENT         # Current manifest pointer
  ├── OPTIONS-*       # RocksDB configuration
  └── Column Families:
      ├── _meta       # Database metadata
      ├── _system:users      # Collection in _system database
      ├── _system:products   # Another collection
      └── myapp:customers    # Collection in custom database
```

### Benefits of RocksDB

- **Automatic persistence**: Data is durably stored immediately
- **Crash recovery**: Write-Ahead Log ensures no data loss
- **Compression**: Built-in LZ4/Snappy/Zstd compression
- **Column Families**: Each collection is a separate column family (`{db}:{collection}`)
- **Multi-Database**: Isolated databases with separate namespaces
- **LSM Tree**: Optimized for write-heavy workloads
- **No manual saves**: Unlike JSON files, no explicit save needed

## Web UI Dashboard

SoliDB includes a web-based management interface for browsing databases, collections, documents, and running AQL queries.

### Running the Dashboard

```bash
# Navigate to the www directory
cd www/

# Start the web server
./luaonbeans.org -D .
```

The dashboard will be available at `http://localhost:8080`.

### Features

- 📊 **Database Browser**: View and manage all databases
- 📁 **Collection Management**: Create, delete, and truncate collections
- 📄 **Document Viewer**: Browse, search, and edit documents
- 🔍 **AQL Query Editor**: Write and execute AQL queries with syntax highlighting
- 📈 **Cluster Status**: Monitor cluster health and replication lag (in cluster mode)
- 📚 **Documentation**: Built-in API and AQL reference

## Benchmarking

### HTTP API Benchmarks (Recommended: oha)

For accurate HTTP load testing, use [oha](https://github.com/hatoo/oha):

```bash
# Install oha
cargo install oha

# Simple query benchmark (10K requests, 8 concurrent)
oha -n 10000 -c 8 -m POST \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR u IN users LIMIT 1 RETURN u"}' \
  http://localhost:6745/_api/database/_system/cursor

# Document GET benchmark
oha -n 10000 -c 8 \
  http://localhost:6745/_api/database/_system/document/users/user_1

# With bind variables
oha -n 10000 -c 8 -m POST \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR u IN users FILTER u.age > @minAge LIMIT @limit RETURN u", "bindVars": {"minAge": 30, "limit": 10}}' \
  http://localhost:6745/_api/database/_system/cursor
```

**Example Results:**
```
Success rate:  100.00%
Total:         168.65 ms
Requests/sec:  59,295.76
Average:       0.13 ms
```

### Built-in Benchmarks

```bash
# Storage layer benchmark (no HTTP overhead)
cargo run --release --bin benchmark

# HTTP API benchmark (with connection pooling)
cargo run --release --bin http-benchmark
```

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
# Install system-wide (recommended)
cargo install --path .
solidb

# Or build and run from target directory
cargo build --release
./target/release/solidb
```

## Cluster & Replication

SoliDB supports multi-node replication for high availability and horizontal scaling.

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Cluster Architecture                     │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────┐    TCP/JSON    ┌─────────────┐            │
│  │   Node 1    │◄──────────────►│   Node 2    │            │
│  │  :6745/:6746│                │  :6755/:6756│            │
│  └──────┬──────┘                └──────┬──────┘            │
│         │                              │                   │
│         │         TCP/JSON             │                   │
│         └──────────────┬───────────────┘                   │
│                        │                                   │
│                        ▼                                   │
│               ┌─────────────┐                              │
│               │   Node 3    │                              │
│               │  :6765/:6766│                              │
│               └─────────────┘                              │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### Features

- **Peer-to-Peer**: No single master - all nodes accept writes
- **Automatic Sync**: New nodes receive full data sync automatically
- **Peer Discovery**: Nodes share peer information for automatic mesh formation
- **Last-Write-Wins**: Conflict resolution using Hybrid Logical Clocks
- **Persistent Log**: Replication log survives restarts

### Cluster Setup

#### Step 1: Start the First Node

```bash
solidb --data-dir ./data1 --port 6745 --replication-port 6746 --node-id node1
```

#### Step 2: Start Additional Nodes

```bash
# Join via the first node
solidb --data-dir ./data2 --port 6755 --replication-port 6756 \
         --node-id node2 --peer 192.168.1.10:6746

# Or join via any existing node
solidb --data-dir ./data3 --port 6765 --replication-port 6766 \
         --node-id node3 --peer 192.168.1.11:6756
```

### Cluster Status API

```bash
curl http://localhost:6745/_api/cluster/status
```

**Response:**
```json
{
  "node_id": "node1",
  "is_cluster_mode": true,
  "current_sequence": 1234,
  "log_entries": 1234,
  "peers": [
    {
      "address": "192.168.1.11:6756",
      "is_connected": true,
      "last_seen_secs_ago": 2,
      "replication_lag": 0
    },
    {
      "address": "192.168.1.12:6766",
      "is_connected": true,
      "last_seen_secs_ago": 5,
      "replication_lag": 3
    }
  ]
}
```

### How Replication Works

1. **Write Operation**: When a document is inserted/updated/deleted, the operation is recorded in the local replication log with a Hybrid Logical Clock timestamp.

2. **Push to Peers**: The node pushes new entries to all connected peers.

3. **Conflict Resolution**: If two nodes modify the same document, Last-Write-Wins (LWW) based on HLC timestamps determines the winner.

4. **Full Sync**: New nodes joining the cluster receive a full snapshot of all databases, collections, and documents.

5. **Incremental Sync**: After initial sync, only new operations are replicated.

### Replicated Operations

All data mutations are replicated:
- ✅ Insert document
- ✅ Update document
- ✅ Delete document
- ✅ Create database
- ✅ Delete database
- ✅ Create collection
- ✅ Delete collection
- ✅ Truncate collection

### Best Practices

1. **Node IDs**: Use meaningful, unique node IDs for easier debugging
2. **Network**: Ensure replication ports are accessible between nodes
3. **Data Directories**: Each node must have its own data directory
4. **Monitoring**: Check cluster status regularly via the API

## Limitations

This is an initial implementation focusing on core functionality. Current limitations:

- No graph queries
- No complex aggregations (GROUP BY, etc.)

## Future Enhancements

- [x] ~~Indexing for faster queries~~ ✅ Implemented!
- [x] ~~JOIN operations~~ ✅ Implemented! (nested FOR loops)
- [x] ~~Geo indexing~~ ✅ Implemented! (DISTANCE, GEO_DISTANCE, near, within)
- [x] ~~LET clauses & Subqueries~~ ✅ Implemented! (LET x = (FOR ... RETURN ...))
- [x] ~~Built-in functions~~ ✅ Implemented! (LENGTH, ROUND, ABS, UPPER, LOWER, CONCAT, SUBSTRING, etc.)
- [x] ~~RocksDB storage backend~~ ✅ Implemented! (crash recovery, compression, LSM tree)
- [x] ~~Bind Variables~~ ✅ Implemented! (@variable for AQL injection prevention)
- [x] ~~Aggregation functions~~ ✅ (COUNT, SUM, AVG, etc.)
- [x] ~~Multi-Database Architecture~~ ✅ Implemented! (isolated databases with collections)
- [x] ~~Replication and clustering~~ ✅ Implemented! (peer-to-peer, LWW conflict resolution, HLC)
- [x] ~~Sharding~~ ✅ Implemented! (horizontal partitioning, auto-rebalancing, auto mode)
- [x] ~~Transactions~~ ✅ Implemented! (ACID via X-Transaction-ID header)
- [x] ~~CLI Tooling~~ ✅ Implemented! (dump/restore with JSONL, JSON Array, and CSV support)
- [x] ~~Authentication~~ ✅ Implemented! (JWT-based authentication with password management)
- [ ] Graph traversal queries
- [ ] Role-based authorization
- [x] ~~WebSocket support~~ ✅ Implemented! (Real-time Changefeeds)
- [ ] Query optimization

## Real-time Changefeeds

SoliDB supports real-time changefeeds via WebSockets, allowing applications to react instantly to data changes.

### Usage

Connect to the WebSocket endpoint:
`ws://localhost:6745/_api/ws/changefeed?token=<your-jwt-token>`

Send a subscription message:
```json
{
  "type": "subscribe",
  "collection": "users",
  "database": "_system", // Optional, defaults to finding collection globally
  "key": "user_123"      // Optional, filter by specific document key
}
```

Receive events:
```json
{
  "type": "insert", // or "update", "delete"
  "key": "user_123",
  "data": { ... },
  "old_data": { ... } // for update/delete
}
```

See [Changefeeds Documentation](http://localhost:8080/docs/changefeeds) for full details.

## License

MIT

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
