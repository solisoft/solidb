# SoliDB against PostgreSQL, MongoDB, ArangoDB and SurrealDB

The harness behind the blog post [SoliDB against PostgreSQL, MongoDB, ArangoDB and SurrealDB](https://solidb.solisoft.net/blog/solidb-against-postgresql-mongodb-arangodb-surrealdb). It sends the same documents and the same queries to each database, checks every answer against the generator, and writes what it measured as JSON.

A second harness, the bank (`bank.sh`, [below](#transactions-the-bank-banksh)), measures transactions against PostgreSQL: durable transfers, lost money, and `kill -9`.

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

With podman, set `DOCKER="sudo podman"`.

**Published results.** The blog post reports `results/2026-11-09/`:
- **The other databases:** one run of every configuration.
- **SoliDB:** the published 2.9.0 image (the digest `run.sh` pins), measured right after the release with the same harness binary.
- **`solidb-7d39337/`:** SoliDB in the same run as the others, on a build of `main` at `7d39337` before the release. It is within 5% of 2.9.0 on every series but the load of the 100,000 customers, which takes under a tenth of a second.
- **`results/2026-11-08/`:** the first run, on SoliDB 2.7.3, before the clients decoded rows into structs.

## Transactions: the bank (`bank.sh`)

A reply to the blog post asked the right question: "Speed don't matter. Correctness does. Is SoliDB ACID compliant?" The bank measures both at once against PostgreSQL. It moves money between accounts in transactions, with every commit fsynced, and checks that none appears or disappears. It also kills the database with `SIGKILL` in the middle of the transfers. The harness is `src/bank/` (`solidb-bank`), run by `bank.sh` on the same machine, limits and pinned images as the rest.

**A transfer** is one transaction:
- take 1 to 100 from a random account, only if it holds that much;
- give it to another account;
- insert a ledger row `w{worker}-{seq}`.

Every account starts at 1,000. A transfer refused by the concurrency control (a lock held, a serialization failure, a stale write) is retried with the same backoff on both databases. The first retry goes at once, then after a random wait of up to 0.2, 0.4 … 10 ms, at most 200 times.

**Series:**
- 10,000 accounts at 1 and 32 clients: little contention;
- 10 accounts at 32 clients: every transfer contends.

Each series gets 5 s of warm-up and 20 s measured, three times. The tables show the median run, with the range in parentheses.

| Configuration | Requests per transfer | How it moves money |
|---|---|---|
| `postgres-update` | 5: `BEGIN`, two `UPDATE`s, `INSERT`, `COMMIT` | READ COMMITTED. `UPDATE … SET balance = balance - $2 WHERE id = $1 AND balance >= $2`, then the credit. Both rows are updated in account order, so two transfers never deadlock. |
| `postgres-serializable` | 6 | SERIALIZABLE. Read both balances, decide in the client, write both in account order. |
| `postgres-naive` | 6 | The same read-then-write at READ COMMITTED. It is not safe, and is here as the control: it shows the checks catch lost updates. 10 accounts only. |
| `solidb-*-query` | 5 HTTP: begin, three queries, commit | `FOR a IN accounts FILTER a._key == @k FILTER a.balance >= @x UPDATE a WITH {balance: a.balance - @x} IN accounts`, the credit the same way, `INSERT` the ledger row. |
| `solidb-*-doc` | 7 HTTP | `GET` both accounts (shared locks), `PUT` both (exclusive locks), `POST` the ledger row. |

**Why SoliDB goes through HTTP.** The native driver's transactions take document writes by key and read-only queries, so a conditional debit cannot be written there. The Soli framework's model transactions use the same HTTP API.

**Durability.**
- **PostgreSQL:** `synchronous_commit=on`, its default.
- **SoliDB:** solistore with `SOLIDB_DURABILITY=full`.

Both fsync every commit before acknowledging it, and both let concurrent commits share an fsync.

**Checks**, after every run and after every restart:

| What | Holds when |
|---|---|
| Money | the balances add up to accounts × 1,000 |
| No overdraft | no balance is negative |
| Acknowledged transfers | each balance is 1,000 plus what the acknowledged transfers moved |
| Ledger | each balance is 1,000 plus what the stored ledger rows moved: no transfer is half applied |
| Durability | every acknowledged transfer has its ledger row, and no other row exists |
| Consistent reads | an auditor sums every balance in one statement every 10 ms while transfers run, and always sees the full total |

A transfer whose commit got no answer (the server was killed) is looked up after the restart and counted as landed or not. Either outcome is allowed.

### Results, 2026-11-09

`results/2026-11-09/bank/`. PostgreSQL 18.6; SoliDB 2.9.0 and 2.9.1, the published images.

| Configuration | Accounts | Clients | Transfers/s | p50 | p99 | Conflicts per commit | DB CPU per commit | Checks | Audits off |
|---|---:|---:|---:|---:|---:|---:|---:|---|---:|
| postgres-update | 10,000 | 1 | 10,555 (10,516–10,836) | 92 µs | 120 µs | 0.00 | 41 µs | ok | 0 of 6,819 |
| postgres-update | 10,000 | 32 | 67,383 (66,810–68,246) | 428 µs | 1,377 µs | 0.00 | 87 µs | ok | 0 of 6,524 |
| postgres-update | 10 | 32 | 8,339 (8,239–8,876) | 890 µs | 44.3 ms | 0.00 | 540 µs | ok | 0 of 6,817 |
| postgres-serializable | 10,000 | 1 | 8,361 (8,317–8,419) | 118 µs | 142 µs | 0.00 | 56 µs | ok | 0 of 6,819 |
| postgres-serializable | 10,000 | 32 | 46,138 (44,970–46,388) | 648 µs | 1,614 µs | 0.01 | 138 µs | ok | 0 of 6,475 |
| postgres-serializable | 10 | 32 | 7,663 (7,526–7,669) | 115 µs | 109.0 ms | 1.03 | 134 µs | ok | 0 of 6,816 |
| postgres-naive (control) | 10 | 32 | 7,477 (7,448–16,749) | 1,233 µs | 47.4 ms | 0.00 | 744 µs | **3 of 3 runs**; money +13,513, +26,399, +31,921 | 6,816 of 6,817 |
| solidb-2.9.1-query | 10,000 | 1 | 3,989 (3,750–4,091) | 241 µs | 352 µs | 0.00 | 227 µs | ok | 0 of 5,741 |
| solidb-2.9.1-query | 10,000 | 32 | 19,189 (19,164–19,236) | 1,606 µs | 3,143 µs | 0.01 | 357 µs | ok | 0 of 3,982 |
| solidb-2.9.1-query | 10 | 32 | 5,121 (5,117–5,124) | 554 µs | 103.9 ms | 2.19 | 647 µs | ok | 0 of 6,165 |
| solidb-2.9.1-doc | 10,000 | 1 | 3,458 (3,455–3,460) | 285 µs | 366 µs | 0.00 | 225 µs | ok | 0 of 6,028 |
| solidb-2.9.1-doc | 10,000 | 32 | 18,874 (18,778–18,889) | 1,619 µs | 3,295 µs | 0.02 | 345 µs | ok | 0 of 4,029 |
| solidb-2.9.1-doc | 10 | 32 | 2,865 (2,841–2,865) | 462 µs | 203.2 ms | 2.94 | 711 µs | ok | 0 of 6,627 |
| solidb-2.9.0-query | 10,000 | 1 | 4,068 (3,831–4,182) | 241 µs | 336 µs | 0.00 | 204 µs | ok | 0 of 5,882 |
| solidb-2.9.0-query | 10,000 | 32 | 19,433 (19,412–19,483) | 1,587 µs | 3,109 µs | 0.01 | 351 µs | **3 of 3 runs**; money −54, +48, +517; up to 13 accounts wrong | 3,778 of 4,058 |
| solidb-2.9.0-query | 10 | 32 | 5,199 (5,171–5,216) | 544 µs | 103.6 ms | 2.16 | 622 µs | **3 of 3 runs**; money +1,854, +901, +2,054; up to 10 accounts wrong | 6,099 of 6,151 |
| solidb-2.9.0-doc | 10,000 | 1 | 3,510 (3,494–3,510) | 281 µs | 361 µs | 0.00 | 220 µs | ok | 0 of 6,029 |
| solidb-2.9.0-doc | 10,000 | 32 | 18,927 (18,866–18,962) | 1,614 µs | 3,278 µs | 0.02 | 343 µs | ok | 0 of 4,010 |
| solidb-2.9.0-doc | 10 | 32 | 2,888 (2,868–2,920) | 450 µs | 193.3 ms | 2.91 | 699 µs | ok | 0 of 6,626 |

**Correctness:**
- **SoliDB 2.9.0 lost and invented money** in all six runs of its SDBQL path, even among 10,000 accounts. This is card #796: an `UPDATE` computed from the document a query read overwrote a write that landed in between. It is fixed in 2.9.1, which passed every check in all 18 of its runs.
- **The document-API path was correct on both versions**, and so were both PostgreSQL idioms.
- **The control** lost or invented money in every run, so the checks do catch a lost update.
- **Consistent reads:** the auditor never saw a wrong total on either database, except after money had actually been lost. Once a total is wrong it stays wrong.

**Speed: PostgreSQL is ahead.**

| Series | PostgreSQL ahead by |
|---|---|
| One client | 2.6× (10,555 against 3,989 transfers/s) |
| 32 clients | 3.5× |
| 10 hot accounts | 1.6× |

- **Where the cost is.** SoliDB spends 4 to 5 times more database CPU per transfer. A `perf` profile (of 2.9.0, at 32 clients) puts it in the five HTTP requests: the HTTP stack takes about 20% of the CPU, the kernel about 21% (TCP on loopback, and the fsyncs), and the authorization middleware about 8%. The locks and the transaction bookkeeping take under 1%.
- **Under contention.** SoliDB's transaction locks refuse at once rather than wait. At 10 accounts that makes 2.2 rolled-back attempts per committed transfer, and a p99 of 104 ms against PostgreSQL's 44 ms.
- **The follow-up** is card #797.

**Crash.** For each configuration, 32 clients transfer among 10,000 accounts. The container is killed with `SIGKILL` after a random 1 to 4 s (seed 1), then started again on the same data and checked. That is 20 times, about 42 s of transfers.

| Configuration | Kills | Transfers acknowledged | In flight at the kill (landed) | Acknowledged, then lost | Ready again (median) | Violations |
|---|---:|---:|---:|---:|---:|---:|
| `crash-postgres` (`synchronous_commit=on`) | 20 | 2,861,750 | 258 (129) | 0 | 408 ms | 0 |
| `crash-postgres-async` (`synchronous_commit=off`) | 20 | 3,657,389 | 110 (0) | 25,517 | 458 ms | 40 |
| `crash-solidb` (2.9.1, `SOLIDB_DURABILITY=full`) | 20 | 786,337 | 277 (137) | 0 | 271 ms | 0 |
| `crash-solidb-eventual` (2.9.1, `eventual`) | 20 | 787,233 | 298 (104) | 0 | 272 ms | 0 |

What the crash runs show:
- **No acknowledged transfer lost.** Durable PostgreSQL and SoliDB in both modes kept every acknowledged transfer. None was half applied, and money was conserved. A transfer in flight at the kill landed or did not, and both happened.
- **`synchronous_commit=off` loses commits, as documented.** PostgreSQL lost the last few hundred milliseconds of acknowledged transfers at each kill, but never part of one: its balances always matched its surviving ledger.
- **SoliDB's `eventual` mode lost nothing to a process kill.** It writes every commit to the operating system and leaves only the fsync to a background thread. Its window of loss is a power failure, which this test does not simulate. That test, LazyFS below the filesystem, is still to do in card #792.

### Running it

```sh
cargo build --release --bin solidb-bank
DOCKER="sudo podman" ./bank.sh all                        # about 45 minutes
WARM=1 SECS=3 RUNS=1 CYCLES=2 DOCKER="sudo podman" ./bank.sh all   # a quick pass
python3 bank_report.py ~/bench/results/<date>/bank
```

`bank.sh solidb-2.9.0` measures the 2.9.0 image. That version's transactional query reads only `bind_vars` (`bindVars` was ignored until 2.9.1), so the harness sends `bind_vars`, which both versions read.

## SoliDB 2.7.2, and why its image was rebuilt

The first run also measured SoliDB 2.7.2, before the fixes this benchmark led to in 2.7.3 (the two-sided range read as one interval, the full-scan COLLECT that streams). Those figures are in `results/2026-11-08/solidb-2.7.2/`.

The published `ghcr.io/solisoft/solidb:v2.5.0` through `v2.7.2` images do not start.
- **The cause:** the release binary is built on `ubuntu-latest` (glibc 2.39), and the Dockerfile's base, `debian:bookworm-slim`, has glibc 2.36.
- **The workaround:** `solidb-image.sh` builds the 2.7.2 run's image from the official release binary, checked against the SHA-256 GitHub records, on `debian:trixie-slim`.
- **Fixed in 2.7.3.** That release ships a working image, which `run.sh` uses by default.
