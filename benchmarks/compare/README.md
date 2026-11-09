# SoliDB against PostgreSQL, MongoDB, ArangoDB and SurrealDB

The harness behind the blog post [SoliDB against PostgreSQL, MongoDB, ArangoDB and SurrealDB](https://solidb.solisoft.net/blog/solidb-against-postgresql-mongodb-arangodb-surrealdb). It sends the same documents and the same queries to each database, checks every answer against the generator, and writes what it measured as JSON.

## What runs

| Configuration | Image | Client |
|---|---|---|
| `solidb-solistore` | `ghcr.io/solisoft/solidb:v2.7.3`, `SOLIDB_ENGINE=solistore` | `solidb-client`, native MessagePack driver |
| `solidb` (by name only, not in `all`) | the same image on its RocksDB engine | same |
| `postgres` | `postgres:18.6`, one `jsonb` column per document | `tokio-postgres`, prepared statements |
| `postgres-typed` | same server, real columns | same |
| `mongo` | `mongo:9.0.4` | official `mongodb` crate 3.10 |
| `arango` | `arangodb/arangodb:3.12` (3.12.4-3, Community) | HTTP API through `reqwest` |
| `surreal` | `surrealdb/surrealdb:v4.0.0`, SurrealKV on disk | official `surrealdb` crate 4.0, WebSocket |

- **Pinning:** every image is pinned by digest in `run.sh`.
- **Recorded with each result:**
  - the exact `docker run` command;
  - the digest;
  - the server's version;
  - the text of every query, in `queries`.

**Durability.** No configuration fsyncs on every commit, except the `-durable` ones, which repeat the writes with that guarantee.
- `solidb-solistore-durable`: `SOLIDB_DURABILITY=full`.
- `postgres-durable`: `synchronous_commit=on`, PostgreSQL's default.
- `mongo-durable`: `{w: 1, j: true}`.
- `arango-durable`: `waitForSync`.
- `surreal-durable`: `sync=every`, SurrealDB's default.
- SoliDB on RocksDB has no per-write fsync mode, so it has no durable line.

**Configuration beyond defaults:**
- **PostgreSQL:** `shared_buffers=4GB`, `effective_cache_size=12GB`, `max_connections=200`, `--shm-size 1g`.
- **ArangoDB:** told the container's memory and cores (`ARANGODB_OVERRIDE_DETECTED_*`). Without that, it sizes its caches from the host's.
- **Every other engine:** its defaults.

## Data and scenarios

**Data** (`src/data.rs`):
- 1,000,000 orders of about 500 bytes of JSON each:
  - a customer, a status, an amount, an ISO `created_at` over two years, a country;
  - three tags, three line items and an address.
- 100,000 customers.
- Indexes on `customer_id`, `created_at` and `status`, created before the load.

**Order of the run:**
1. **Load.** The first 10% of the orders come from one connection, the rest from eight, in batches of 1,000.
2. **Check.** Before anything is timed, every read is checked against the generator 50 times. A wrong answer stops the run instead of counting as a fast one.
3. **Series:**

| Scenario | What |
|---|---|
| `get` | one order by key |
| `by_customer` | every order of a customer (≈10) |
| `aggregate` | count and total amount per status, over the whole collection |
| `recent` | the 20 most recent orders in a one-day window |
| `join` | the same 20, each with its customer's name |
| `insert` | one new order |
| `update` | one order's status |

**What the client does with the rows.** In `by_customer`, `recent` and `join`, every adapter deserializes the rows into the same Rust structs (`Order`, `JoinRow` in `src/engines/mod.rs`), through the serde support its client documents:

| Engine | How |
|---|---|
| SoliDB | `query_as::<Order>` |
| PostgreSQL jsonb | `Json<Order>` |
| PostgreSQL typed | the columns, with `Json<T>` for the nested ones |
| MongoDB | a `Collection<Order>`, and `aggregate(…).with_type::<JoinRow>()` |
| ArangoDB | the cursor body as `{result: Vec<Order>}` |
| SurrealDB | `take::<Vec<Order>>` |

- **Why the same structs:** each engine pays for the same decoding, which a generic JSON tree would not give. The cost of building a `serde_json::Value` grows with every field an engine adds to a row: system fields, record ids.
- **Not decoded:** the identifier, which every engine names and types differently, is returned but skipped.
- **`get` is the exception:** it reads the whole document as `serde_json::Value` everywhere, because SoliDB's client has no typed `get`.
- **Before 2026-11-09:** the queries were decoded into `serde_json::Value` everywhere, except PostgreSQL typed, which read the columns.

**Timing and order:**
- **Concurrency:** each series runs at concurrency 1 and 32; `aggregate` runs at 1 only.
- **Duration:** 5 s of warm-up, then 3 measured runs of 20 s. The report takes the median.
- **Reads first:** every read runs before any write, so all engines read the same million documents.

**Failures.** If a series takes a database down (the container is limited to 16 GB, and is killed past it), the failure is recorded under `aborted`.
- `run.sh` records the container's state in `<config>.container.json`.
- It then restarts the database on the same data, and the harness resumes after that series.

## Running it

Linux with Docker or rootful podman. The CPU sets in `run.sh` assume 16 cores: four for the database and eight for the harness. Adjust `DB_CPUS` and `CLIENT_CPUS` to the machine (`lscpu -e=CPU,CORE`).

```sh
cargo build --release
./solidb-image.sh                       # the SoliDB image, from the release binary
./run.sh all                            # about three hours
DOCS=10000 CUSTOMERS=1000 WARM=1 SECS=2 RUNS=1 ./run.sh all   # a quick pass
python3 report.py ~/bench/results/<date>
```

With podman, set `DOCKER="sudo podman"`. `results/2026-11-08/` is the first run, on SoliDB 2.7.3, before the clients decoded rows into structs.

## SoliDB 2.7.2, and why its image was rebuilt

The first run also measured SoliDB 2.7.2, before the fixes this benchmark led to in 2.7.3 (the two-sided range read as one interval, the full-scan COLLECT that streams). Those figures are in `results/2026-11-08/solidb-2.7.2/`.

The published `ghcr.io/solisoft/solidb:v2.5.0` through `v2.7.2` images do not start.
- **The cause:** the release binary is built on `ubuntu-latest` (glibc 2.39), and the Dockerfile's base, `debian:bookworm-slim`, has glibc 2.36.
- **The workaround:** `solidb-image.sh` builds the 2.7.2 run's image from the official release binary, checked against the SHA-256 GitHub records, on `debian:trixie-slim`.
- **Fixed in 2.7.3.** That release ships a working image, which `run.sh` uses by default.
