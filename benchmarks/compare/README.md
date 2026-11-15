# SoliDB against PostgreSQL, MongoDB, ArangoDB and SurrealDB

The harness behind the blog post [SoliDB against PostgreSQL, MongoDB, ArangoDB and SurrealDB](https://solidb.solisoft.net/blog/solidb-against-postgresql-mongodb-arangodb-surrealdb). It sends the same documents and the same queries to each database, checks every answer against the generator, and writes what it measured as JSON.

A second harness, the bank (`bank.sh`, [below](#transactions-the-bank-banksh)), measures transactions against PostgreSQL: durable transfers, lost money, and `kill -9`.

A third harness, the graph (`graph.sh`, [below](#the-graph-graphsh)), asks traversal and shortest-path questions of SoliDB, Neo4j, Memgraph and ArangoDB. Its first full-scale results, of 2026-11-14, are [below](#results-2026-11-14).

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

**Foreign keys.** `solidb-solistore-fk` and `postgres-typed-fk` declare that an order's `customer_id` refers to a customer (SoliDB: `create_foreign_key`, served by `idx_customer_id`; PostgreSQL: `FOREIGN KEY (customer_id) REFERENCES customers (key) ON DELETE RESTRICT`) and run only the writes: `load`, `insert`, `update`. They are not in `all`. The line to compare each with is the same configuration without `-fk`, restricted to the same phases: `PHASES=load,insert,update ./run.sh solidb-solistore postgres-typed`. `update` changes an order's status, not its customer, so it shows what a key costs a write that does not touch the reference.

`results/2026-11-13-fk/` holds that comparison: SoliDB on a build of the `feat/foreign-keys` branch (2.9.5 plus foreign keys, `--features engine-solistore`, the manifest's release profile), PostgreSQL 18.6, the four configurations one after the other on rocky-build2 with nothing else running. Medians of three runs:

| Operations per second | `solidb-solistore` | `-fk` | `postgres-typed` | `-fk` |
|---|---|---|---|---|
| load, 8 connections | 468,173 | 437,413 | 297,445 | 223,249 |
| insert, c=1 | 41,402 | 39,402 | 33,001 | 29,087 |
| insert, c=32 | 159,927 | 150,174 | 141,266 | 104,942 |
| update, c=1 | 39,671 | 39,254 | 25,489 | 25,050 |
| update, c=32 | 177,374 | 181,767 | 103,409 | 91,273 |

The runs of `insert` at c=32 spread widely on both engines (SoliDB: 131,537 to 172,579 without the key, 124,856 to 169,197 with), falling from the first run to the third; read that line as "no difference shown". Not measured: parent deletes running beside child inserts, which is where SoliDB's per-database gate would show.

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
| `postgres-function` | 1: `SELECT transfer(…)` | `postgres-update`'s statements in a PL/pgSQL function, one statement and its own transaction. Rows still updated in account order; a debit that finds too little money after the credit raises, which rolls the credit back. The counterpart of `run_transaction`. |
| `postgres-naive` | 6 | The same read-then-write at READ COMMITTED. It is not safe, and is here as the control: it shows the checks catch lost updates. 10 accounts only. |
| `solidb-*-query` | 5 HTTP: begin, three queries, commit | `FOR a IN accounts FILTER a._key == @k FILTER a.balance >= @x UPDATE a WITH {balance: a.balance - @x} IN accounts`, the credit the same way, `INSERT` the ledger row. |
| `solidb-*-doc` | 7 HTTP | `GET` both accounts (shared locks), `PUT` both (exclusive locks), `POST` the ledger row. |
| `solidb-main-run` | 1 over the native driver: `run_transaction` | The same three statements sent in one request: the server begins the transaction, runs them, commits. The debit and the credit are marked `require_rows`, so a debit that matches nothing rolls the transaction back. After 2.9.2, on a build of `main`. |
| `solidb-2.9.2-driver` | 5 over the native driver: begin, two queries, an insert, commit | The `query` statements (ending in `RETURN 1`, whose rows say whether the debit matched) through `solidb-client`'s MessagePack driver. Needs 2.9.2, the first version that runs a query that writes inside a driver transaction. |

**HTTP and the driver.** Up to 2.9.1 the native driver's transactions took document writes by key and read-only queries only, so a conditional debit could only go through HTTP, the API the Soli framework's model transactions also use. 2.9.2 stages a query that writes in a driver transaction, with the same lost-update guard, and the `driver` mode measures that.

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

`results/2026-11-09/bank/` (PostgreSQL 18.6; SoliDB 2.9.0 and 2.9.1) and `results/2026-11-09/bank-2.9.2/` (SoliDB 2.9.2), all on the published images. `bank-driver/` is the same driver mode on a build of `main` before the 2.9.2 release, within 2% of it. `bank-run/` is a build of `main` after 2.9.2, with `run_transaction` and the driver changes listed below, not yet released.

| Configuration | Accounts | Clients | Transfers/s | p50 | p99 | Conflicts per commit | DB CPU per commit | Checks | Audits off |
|---|---:|---:|---:|---:|---:|---:|---:|---|---:|
| postgres-update | 10,000 | 1 | 10,555 (10,516–10,836) | 92 µs | 120 µs | 0.00 | 41 µs | ok | 0 of 6,819 |
| postgres-update | 10,000 | 32 | 67,383 (66,810–68,246) | 428 µs | 1,377 µs | 0.00 | 87 µs | ok | 0 of 6,524 |
| postgres-update | 10 | 32 | 8,339 (8,239–8,876) | 890 µs | 44.3 ms | 0.00 | 540 µs | ok | 0 of 6,817 |
| postgres-serializable | 10,000 | 1 | 8,361 (8,317–8,419) | 118 µs | 142 µs | 0.00 | 56 µs | ok | 0 of 6,819 |
| postgres-serializable | 10,000 | 32 | 46,138 (44,970–46,388) | 648 µs | 1,614 µs | 0.01 | 138 µs | ok | 0 of 6,475 |
| postgres-serializable | 10 | 32 | 7,663 (7,526–7,669) | 115 µs | 109.0 ms | 1.03 | 134 µs | ok | 0 of 6,816 |
| postgres-function | 10,000 | 1 | 21,674 (21,671–22,656) | 44 µs | 65 µs | 0.00 | 24 µs | ok | 0 of 6,817 |
| postgres-function | 10,000 | 32 | 113,920 (112,588–114,407) | 262 µs | 556 µs | 0.00 | 54 µs | ok | 0 of 6,525 |
| postgres-function | 10 | 32 | 10,406 (9,770–11,465) | 831 µs | 34.4 ms | 0.00 | 476 µs | ok | 0 of 6,814 |
| postgres-naive (control) | 10 | 32 | 7,477 (7,448–16,749) | 1,233 µs | 47.4 ms | 0.00 | 744 µs | **3 of 3 runs**; money +13,513, +26,399, +31,921 | 6,816 of 6,817 |
| solidb-main-run | 10,000 | 1 | 9,919 (9,901–9,947) | 110 µs | 145 µs | 0.00 | 103 µs | ok | 0 of 5,357 |
| solidb-main-run | 10,000 | 32 | 61,828 (61,746–61,984) | 483 µs | 1,100 µs | 0.02 | 113 µs | ok | 0 of 3,671 |
| solidb-main-run | 10 | 32 | 15,648 (15,645–15,706) | 134 µs | 43.9 ms | 1.21 | 138 µs | ok | 0 of 5,706 |
| solidb-main-driver | 10,000 | 1 | 7,322 (6,547–7,603) | 124 µs | 185 µs | 0.00 | 110 µs | ok | 0 of 5,677 |
| solidb-main-driver | 10,000 | 32 | 44,825 (44,777–44,839) | 680 µs | 1,396 µs | 0.01 | 138 µs | ok | 0 of 3,708 |
| solidb-main-driver | 10 | 32 | 9,719 (9,685–9,722) | 177 µs | 73.3 ms | 1.31 | 167 µs | ok | 0 of 6,006 |
| solidb-2.9.2-driver | 10,000 | 1 | 7,563 (7,540–7,579) | 128 µs | 178 µs | 0.00 | 109 µs | ok | 0 of 5,629 |
| solidb-2.9.2-driver | 10,000 | 32 | 38,518 (37,767–38,597) | 798 µs | 1,424 µs | 0.01 | 161 µs | ok | 0 of 4,016 |
| solidb-2.9.2-driver | 10 | 32 | 9,369 (9,366–9,376) | 206 µs | 73.2 ms | 1.37 | 183 µs | ok | 0 of 6,189 |
| solidb-2.9.2-query | 10,000 | 1 | 4,061 (3,805–4,126) | 238 µs | 346 µs | 0.00 | 203 µs | ok | 0 of 5,927 |
| solidb-2.9.2-query | 10,000 | 32 | 19,561 (19,534–19,662) | 1,573 µs | 3,097 µs | 0.01 | 348 µs | ok | 0 of 4,016 |
| solidb-2.9.2-query | 10 | 32 | 5,189 (5,189–5,212) | 547 µs | 102.6 ms | 2.17 | 626 µs | ok | 0 of 6,111 |
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
- **SoliDB 2.9.0 lost and invented money** in all six runs of its SDBQL path, even among 10,000 accounts. This is card #796: an `UPDATE` computed from the document a query read overwrote a write that landed in between. It is fixed in 2.9.1, which passed every check in all 18 of its runs, and so did 2.9.2, over HTTP and over the driver.
- **The document-API path was correct on both versions**, and so were both PostgreSQL idioms.
- **The control** lost or invented money in every run, so the checks do catch a lost update.
- **Consistent reads:** the auditor never saw a wrong total on either database, except after money had actually been lost. Once a total is wrong it stays wrong.

**Speed.** Over HTTP (2.9.1), PostgreSQL is ahead on every series:

| Series | PostgreSQL ahead by |
|---|---|
| One client | 2.6× (10,555 against 3,989 transfers/s) |
| 32 clients | 3.5× |
| 10 hot accounts | 1.6× |

- **Where the cost is.** SoliDB spends 4 to 5 times more database CPU per transfer. A `perf` profile (of 2.9.0, at 32 clients) puts it in the five HTTP requests: the HTTP stack takes about 20% of the CPU, the kernel about 21% (TCP on loopback, and the fsyncs), and the authorization middleware about 8%. The locks and the transaction bookkeeping take under 1%.
- **Under contention.** SoliDB's transaction locks refuse at once rather than wait. At 10 accounts that makes 2.2 rolled-back attempts per committed transfer, and a p99 of 104 ms against PostgreSQL's 44 ms.

Over the driver (2.9.2), the same statements commit about twice as often:

| Series | SoliDB 2.9.2, driver | PostgreSQL | |
|---|---:|---:|---|
| One client | 7,563 (p50 128 µs) | 10,555 (p50 92 µs) | PostgreSQL 1.4× ahead |
| 32 clients | 38,518 | 67,383 | PostgreSQL 1.75× ahead |
| 10 hot accounts | **9,369** | 8,339 | SoliDB 1.12× ahead |

- **What is left.** A line-tables profile of the driver path puts half of the server's CPU at 32 clients in the commit. About 20–25% of it is solistore's group commit, whose waiters spin and yield while the leader fsyncs (card #798). The responses' `send` calls take 18%, and the queries 12%. At one client the commit is 70 µs of the 128.
- **The follow-up** is card #797: the group commit, running the driver's commit and key-bound staged queries off the async workers, skipping the commit-time re-read, and a one-request transaction.

In one request each, the transfer is one round trip instead of five: `run_transaction` for SoliDB (a build of `main` after 2.9.2), a PL/pgSQL function for PostgreSQL (`postgres-function`). Compared with PostgreSQL's five round trips SoliDB would look level, but that is the wrong comparison:

| Series | SoliDB, one request | PostgreSQL, one request | |
|---|---:|---:|---|
| One client | 9,919 (p50 110 µs) | 21,674 (p50 44 µs) | PostgreSQL 2.2× ahead |
| 32 clients | 61,828 (p99 1.1 ms) | 113,920 (p99 556 µs) | PostgreSQL 1.8× ahead |
| 10 hot accounts | **15,648** | 10,406 | SoliDB 1.5× ahead |
| DB CPU per transfer | 103–138 µs | 24–476 µs | |

With the round trips out of the way, what is left is the cost of a transaction itself: SoliDB spends 2 to 4 times PostgreSQL's CPU on one, most of it in the commit (70 µs of a single client's 110). Under contention SoliDB keeps the lead in throughput, its locks refusing at once and the transfer retried where PostgreSQL's wait in line, but PostgreSQL's tail is the shorter one (p99 34 ms against 44).

- **The same build without it** (`solidb-main-driver`, five round trips) is 16% faster than 2.9.2 at 32 clients (44,825 against 38,518). Its commit no longer holds the async worker during the fsync, and a staged query that reaches documents only by key runs inline instead of going through the blocking pool.
- **What a profile left out of reach:** the commit is still most of a transfer at one client. solistore's group commit spends CPU spinning while another commit fsyncs (card #798). Letting those waiters sleep on one shared condition variable cut 18% of the CPU per transfer but slowed one client and stalled one run. That needs per-group wake-ups instead, so it is not in this build.

**Crash.** For each configuration, 32 clients transfer among 10,000 accounts. The container is killed with `SIGKILL` after a random 1 to 4 s (seed 1), then started again on the same data and checked. That is 20 times, about 42 s of transfers.

| Configuration | Kills | Transfers acknowledged | In flight at the kill (landed) | Acknowledged, then lost | Ready again (median) | Violations |
|---|---:|---:|---:|---:|---:|---:|
| `crash-postgres` (`synchronous_commit=on`) | 20 | 2,861,750 | 258 (129) | 0 | 408 ms | 0 |
| `crash-postgres-async` (`synchronous_commit=off`) | 20 | 3,657,389 | 110 (0) | 25,517 | 458 ms | 40 |
| `crash-solidb` (2.9.1, `SOLIDB_DURABILITY=full`) | 20 | 786,337 | 277 (137) | 0 | 271 ms | 0 |
| `crash-solidb-eventual` (2.9.1, `eventual`, commit still fsynced) | 20 | 787,233 | 298 (104) | 0 | 272 ms | 0 |

What the crash runs show:
- **No acknowledged transfer lost.** Durable PostgreSQL and SoliDB in both modes kept every acknowledged transfer. None was half applied, and money was conserved. A transfer in flight at the kill landed or did not, and both happened.
- **`synchronous_commit=off` loses commits, as documented.** PostgreSQL lost the last few hundred milliseconds of acknowledged transfers at each kill, but never part of one: its balances always matched its surviving ledger.
- **`crash-solidb-eventual` measured the same thing as `crash-solidb`** (corrected 2026-11-10). We first read it as a test of commits acknowledged before their fsync. It is not: these transactions begin at `read_committed`, whose commit SoliDB fsyncs in either durability mode; `SOLIDB_DURABILITY` decides only for writes outside a transaction and for `read_uncommitted` commits. `crash-solidb-unsynced` (below, 2026-11-10) is the test that line was meant to be. A power failure, LazyFS below the filesystem, is still to do in card #792.

### Results, 2026-11-10: SoliDB 2.9.3

On the published 2.9.3 image (`ghcr.io/solisoft/solidb@sha256:c5de9e7b…`). `results/2026-11-10/bank/` is `bank.sh solidb-2.9.3`: the three modes one after another on one server. `bank-run/` is `run_transaction` alone on a fresh server, the way `postgres-function` was measured.

| Configuration | Accounts | Clients | Transfers/s | p50 | p99 | Conflicts per commit | DB CPU per commit | Checks |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| solidb-2.9.3-run, fresh server | 10,000 | 1 | 11,945 (10,945–11,955) | 68 µs | 129 µs | 0.00 | 80 µs | ok |
| solidb-2.9.3-run, fresh server | 10,000 | 32 | 68,903 (68,844–69,106) | 434 µs | 953 µs | 0.02 | 98 µs | ok |
| solidb-2.9.3-run, fresh server | 10 | 32 | 16,204 (16,138–16,207) | 129 µs | 41.9 ms | 1.18 | 124 µs | ok |
| solidb-2.9.3-run, after query and driver | 10,000 | 1 | 10,374 (10,347–10,393) | 109 µs | 139 µs | 0.00 | 100 µs | ok |
| solidb-2.9.3-run, after query and driver | 10,000 | 32 | 68,467 (68,325–68,765) | 436 µs | 978 µs | 0.02 | 98 µs | ok |
| solidb-2.9.3-run, after query and driver | 10 | 32 | 16,164 (16,115–16,200) | 130 µs | 42.0 ms | 1.19 | 125 µs | ok |
| solidb-2.9.3-driver | 10,000 | 1 | 7,036 (7,032–7,041) | 152 µs | 194 µs | 0.00 | 125 µs | ok |
| solidb-2.9.3-driver | 10,000 | 32 | 49,283 (49,179–49,340) | 616 µs | 1,283 µs | 0.01 | 121 µs | ok |
| solidb-2.9.3-driver | 10 | 32 | 10,160 (10,088–10,162) | 173 µs | 70.1 ms | 1.29 | 157 µs | ok |
| solidb-2.9.3-query | 10,000 | 1 | 4,133 (3,872–4,233) | 235 µs | 334 µs | 0.00 | 197 µs | ok |
| solidb-2.9.3-query | 10,000 | 32 | 20,135 (20,127–20,152) | 1,551 µs | 3,034 µs | 0.01 | 327 µs | ok |
| solidb-2.9.3-query | 10 | 32 | 5,303 (5,277–5,314) | 540 µs | 100.8 ms | 2.14 | 623 µs | ok |

- **One request each**, fresh servers on both sides: PostgreSQL's function commits 1.8× as many transfers at one client (21,674 against 11,945) and 1.65× at 32 (113,920 against 68,903); SoliDB commits 1.56× as many among 10 hot accounts (16,204 against 10,406). DB CPU per transfer: 80 / 98 / 124 µs against 24 / 54 / 476 µs.
- **The order matters at one client.** Run after the HTTP and driver modes on the same server, `run_transaction` commits 10,374/s (p50 109 µs) instead of 11,945 (p50 68 µs); the other series do not move. Cause not established.
- **Over the driver, 2.9.3 against 2.9.2:** 49,283 against 38,518 at 32 clients, 10,160 against 9,369 among hot accounts, but 7,036 against 7,563 at one client (p50 152 against 128 µs, CPU 125 against 109 µs). That regression is not explained yet.
- `ab-fresh/`: `run_transaction` on the published image and on the `798-2` build it was released from, alternating, fresh servers, 2 × 12 s: the published image is as fast or faster (12,702 against 11,818 at one client in steady state, 70,473 against 66,217 at 32).

**Crash on 2.9.3** (`results/2026-11-10/crash/`, `SOLIDB_CRASH` set to the 2.9.3 image). `crash-solidb-unsynced` begins its transactions at `read_uncommitted` on `SOLIDB_DURABILITY=eventual`: the only commit SoliDB acknowledges before its fsync (`bank.sh` passes `--isolation read_uncommitted`).

| Configuration | Kills | Transfers acknowledged | In flight at the kill (landed) | Acknowledged, then lost | Ready again (median) | Violations |
|---|---:|---:|---:|---:|---:|---:|
| `crash-solidb` (`full`) | 20 | 827,338 | 217 (93) | 0 | 272 ms | 0 |
| `crash-solidb-eventual` (commit still fsynced) | 20 | 824,962 | 272 (120) | 0 | 272 ms | 0 |
| `crash-solidb-unsynced` (`read_uncommitted`, `eventual`) | 20 | 943,758 | 163 (30) | 0 | 323 ms | 0 |

A commit acknowledged before its fsync lost nothing to a process kill: it is acknowledged once written to the kernel, where PostgreSQL's `synchronous_commit=off` acknowledges while the WAL record is still in its own memory (25,517 lost above). A power failure is still untested.

**Fsynced single writes** (`run.sh solidb-solistore-durable postgres-durable`, `results/2026-11-10/`, the same morning): inserts at 32 clients 86,890 (86,040–88,772) against PostgreSQL's 88,307 (76,449–102,017), p99 546 µs against 1,985; updates 88,243 against 49,506. `load-check/`: 2.9.0 and 2.9.3 alternately, one run each, to check a slower fsynced load seen once (317,598 docs/s with 8 loaders): 2.9.0 368,504 and 385,070, 2.9.3 381,557 and 385,815, so no regression; inserts at 32 clients 86,964 and 87,448 against 92,058 and 89,906.

**Write skew** (`write_skew.sh`, output in `results/2026-11-10/write-skew.txt`): two transactions at `serializable`, each taking one of two on-call doctors off call through a query if both are on call, both commit. Through documents read by key, the second write gets a `409` and one doctor stays on call.

**Locks that wait** (`results/2026-11-10/lock-wait/`; a build of `main` after 2.9.3, `797-e`, against the one before it, `797-b`, alternating, fresh servers, 2 × 12 s each, twice). Up to 2.9.3 a document lock another transaction holds is refused at once and the transfer retried after a backoff. After it, the transaction waits for the lock (500 ms at most, `SOLIDB_TX_LOCK_WAIT_MS`), a deadlock is refused at once, and at `read_committed` an `UPDATE` whose document changed while it waited is evaluated again on the current one. From this measurement on, the SoliDB harness writes the two accounts in account order, as the PostgreSQL harness always did. 10 accounts, 32 clients, every commit fsynced:

| Mode | Locks refuse at once | Locks wait | PostgreSQL |
|---|---:|---:|---:|
| `run` (one request) | 16,990/s, p99 42 ms, 1.1 refusals per commit | **27,330/s, p50 292 µs, p99 5.4 ms, none refused** | `postgres-function` 10,406/s, p99 34 ms |
| `driver` (five round trips) | 10,930/s, p99 68 ms, 1.17 | **17,440/s, p99 8.5 ms, none** | `postgres-update` 8,339/s |
| `query` (HTTP) | 5,715/s, p99 114 ms, 1.9 | **8,610/s, p99 17 ms, none** | |

Among 10,000 accounts nothing changes (`run` 77,400 against 77,500 at 32 clients, 13,530 against 13,500 at one; p99 at 32 clients 750 µs against 845).

The order matters, and only once locks wait (`hot-series/`, the 10-account series alone, 10 s, twice, with the harness of before and the ordered one). In either order a server that refuses at once does the same (`run` 11,860 and 11,840/s). A server that waits, given the two accounts in the transfer's own order, has two transfers between the same pair deadlock: `run` 11,440/s (0.72 refusals per commit, p99 14 ms), `driver` 7,260/s against 10,370, `query` 3,650/s against 5,430, so fewer transfers than refusing at once, with a tail two to four times shorter. In account order: 22,940, 17,400 and 9,100/s, none refused. (The series alone measures lower than after the two 10,000-account series, on both builds.)

**Commits in place on a quiet server** (`results/2026-11-11/commit-handoff/`; a build of `main`, `797-k`, against `797-e`, alternating, fresh servers, 2 × 12 s each, twice; the second run of each is given, the first of a fresh server writes a WAL file it has not written before and is slower on both: 10,230 and 14,150/s). Since 2.9.3 a driver commit leaves its async worker for its fsync (`block_in_place`), so the worker's other connections go on. With one client there are none, and leaving cost a thread wake-up per commit and the next request on another core: 14 of a transfer's 64 µs. The commit now stays on its worker while few driver connections are active and leaves from more than two per worker thread (counted per 4 ms, averaged; back under one and a half). The same build reads a stored document without bincode and plans a lone `FILTER d._key == …` without listing the indexes.

| Mode | Accounts | Clients | `797-e` | `797-k` | PostgreSQL |
|---|---:|---:|---:|---:|---:|
| `run` (one request) | 10,000 | 1 | 13,920/s, p50 58 µs, p99 119 µs | **18,460/s, p50 51 µs, p99 80 µs** | `postgres-function` 21,674/s, p50 44 µs |
| `run` | 10,000 | 32 | 77,720/s, p50 389 µs | 80,630/s, p50 376 µs | 113,920/s |
| `run` | 10 | 32 | 28,010/s | 28,630/s | 10,406/s |
| `driver` (five round trips) | 10,000 | 1 | 7,290/s, p50 147 µs, p99 188 µs | **9,730/s, p50 99 µs, p99 151 µs** | `postgres-update` 10,555/s |
| `driver` | 10,000 | 32 | 51,780/s | 54,050/s | 67,383/s |
| `driver` | 10 | 32 | 17,840/s | 18,170/s | 8,339/s |
| `query` (HTTP) | 10,000 | 1 | 4,080/s | 4,130/s | |
| `query` | 10,000 | 32 | 20,580/s | 21,270/s | |
| `query` | 10 | 32 | 8,760/s | 8,990/s | |

HTTP commits are not concerned (they run on the blocking pool). Where the two ways cross (`bare-metal.txt`: no container, server on 8 CPUs, `run`, 10,000 accounts; `f` always leaves, `k` decides): 1 client 13,280 → 19,410/s, 2: 21,920 → 24,260, 4: 31,990 → 33,650, 8: 53,000 → 55,210, 12: 62,680 → 70,230, 16: 68,770 → 69,740, then equal within the 1% between two passes of one binary (20: 74,260 and 74,210; 32: 85,490 and 85,430; 64: 97,050 and 96,940). A server where some commits stay and some leave does worse than either (25,000/s at four clients against 33,000 and 31,000), which is why the choice is made for the server, from an average, and not per commit. PostgreSQL remains 1.17× ahead at one client and 1.41× at 32 in one request.

**Commits in groups** (`results/2026-11-11/group-commit/`; a build of `main` with the group committer on a solistore with `WriteBatch::append` and the group insert below, `gci`, against the same solistore without the committer, `sgrp3`; alternating, fresh servers, 2 × 12 s each, twice). Under load a driver connection prepares its commit and hands it to one thread, which writes every commit waiting as one batch with one fsync; no connection thread sleeps through an fsync. The server also looks at its sockets every 4 tasks instead of tokio's 61 (`SOLIDB_IO_POLL_INTERVAL`): without that the grouped commits were slower over five round trips (56,000/s at 32 clients against 61,000), requests that had arrived waiting to be read.

| Mode | Accounts | Clients | Before | Grouped | PostgreSQL |
|---|---:|---:|---:|---:|---:|
| `run` | 10,000 | 1 | 18,290/s | 18,510/s | 21,674/s |
| `run` | 10,000 | 32 | 85,820/s, p50 350 µs, p99 636 µs | **108,620/s, p50 245 µs, p99 747 µs** | 113,920/s |
| `run` | 10 | 32 | 28,320/s, p99 5.3 ms | **35,070/s, p99 4.3 ms** | 10,406/s |
| `driver` | 10,000 | 1 | 9,670/s | 9,670/s | 10,555/s |
| `driver` | 10,000 | 32 | 57,170/s, p50 525 µs, p99 968 µs | **71,740/s, p50 406 µs, p99 1,031 µs** | 67,383/s |
| `driver` | 10 | 32 | 18,050/s | 19,780/s | 8,339/s |

`bare-metal.txt`: the committer and the socket interval apart, 1 to 64 clients. With tokio's interval the committer gives 103,500/s at 32 clients in one request (88,800 without) and loses over five round trips; at 4 or 2, 121,800–124,100 and 75,600–79,100. Without the committer the interval moves nothing by more than 2%. Reads and single updates at interval 4 against 61: point reads at 32 clients 746,000 → 843,000/s, index reads and sorted pages unchanged in throughput with a longer p99 (240 → 350 µs, 415 → 545 µs), one client within 1%.

`crash-run/`, `crash-driver/`: `CRASH_MODE=run` and `driver` (`bank.sh crash-solidb crash-solidb-eventual`), so that the killed commits are grouped ones: 80 `kill -9`, 14.8 million transfers acknowledged, no violation.

**The leader of a small fsynced group inserts it** (`results/2026-11-11/solistore-group-insert/`; solistore, same SoliDB): 32 clients 80,800 → 86,430/s in one request, 53,370 → 58,180 over five round trips. It is the `Before` column above.

**Sorted reads, planner** (`results/2026-11-10/sorted-read-planner/`, `797-b` against `797-c`, one client): the plan of an index-ordered read opens its collection once, clones only the index it chose, and does not look for an index to narrow a filter that is only a range on the sort field. The join 13,003 → 13,485/s (median 76–77 → 73–74 µs; PostgreSQL 71 µs), the 20 most recent 14,707 → 14,971/s, ten documents by index 24,980 → 25,175/s.

### Results, 2026-11-10: ArangoDB 3.12.4

`results/2026-11-10/bank-arango/`. The same transfer, checks and machine, on ArangoDB 3.12.4-3 (Community) through its HTTP API. Both collections are created with `waitForSync` and every transaction asks for it too (the two do not add up: the rates are the same without the collections' flag). Two transactions that write one document do not wait: the second is refused with error 1200 and retried.

| Configuration | Requests per transfer | How it moves money |
|---|---|---|
| `arango-stream` | 5 HTTP: begin, two queries, an insert, commit | A stream transaction. The debit and the credit are AQL under `x-arango-trx-id`, the debit `FILTER a.balance >= @x … RETURN 1`. |
| `arango-js` | 1 HTTP | The same three statements in a JavaScript transaction (`POST /_api/transaction` with an `action`). |

Transfers per second, median of three runs (range), every check clean on every run:

| Series | `arango-stream` | `arango-js` | DB CPU per transfer |
|---|---|---|---|
| 10,000 accounts, 1 client | 2,011 (1,999–2,184), p50 452 µs | 2,283 (2,282–2,310), p50 427 µs | 628 / 605 µs |
| 10,000 accounts, 32 clients | 9,296 (9,238–9,426), p99 6.9 ms | 8,347 (8,225–8,407), p99 8.5 ms | 612 / 700 µs |
| 10 accounts, 32 clients | 2,173 (2,143–2,225), 3.9 refusals per commit | 3,053 (3,047–3,157), 3.0 refusals per commit | 2,159 / 2,025 µs |

Against it, measured the same day on a build of `main` (f4e2f35e, `results/2026-11-10/transaction-cpu-1/`), not in the same pass: SoliDB's `run_transaction` does 13,983, 77,411 and 16,905 transfers a second on the three series (65 to 111 µs of DB CPU each), and its five HTTP requests about 4,000, 20,000 and 5,300.

**Crash** (`crash-arango`, stream transactions, 32 clients, SIGKILL of the container): four runs, 70 kills, 1,618,985 transfers acknowledged.
- In 69 of them every acknowledged transfer was there after the restart, none half applied, and the server answered again within about 0.4 s.
- Once (the first run, its tenth kill, about four seconds after the server had finished recovering from the ninth) ArangoDB did not start again: `FATAL … unable to initialize RocksDB engine: Corruption: SST file is ahead of WALs in CF default` (`crash-arango.server.log`). Three more runs of 20 kills each, one of them with the same seed, did not reproduce it. One occurrence, reported as seen: the kill and the restart are the commands used for PostgreSQL and SoliDB.

### Running it

```sh
cargo build --release --bin solidb-bank
DOCKER="sudo podman" ./bank.sh all                        # about 45 minutes
WARM=1 SECS=3 RUNS=1 CYCLES=2 DOCKER="sudo podman" ./bank.sh all   # a quick pass
python3 bank_report.py ~/bench/results/<date>/bank
```

`bank.sh solidb-2.9.2` measures the 2.9.2 image over HTTP and over the driver; `ONLY_MODES="driver run"` limits a configuration to those modes; `SOLIDB_MAIN=<image> bank.sh solidb-main` does the same for an unreleased build. For profiling, `BANK_STEPS=1` prints the mean time of each step of a driver transfer, and `--audit-ms 0` turns the auditor off. `bank.sh solidb-2.9.0` measures the 2.9.0 image. That version's transactional query reads only `bind_vars` (`bindVars` was ignored until 2.9.1), so the harness sends `bind_vars`, which both versions read.

## The graph (`graph.sh`)

A third harness asks graph questions: who is within one, two or three hops of a person, who among them matches a filter or bought a product, and how long the shortest path between two people is. It loads the same generated graph into each database, asks the same six questions in each database's own query language, and checks every answer against its own copy of the graph. The harness is `src/graph/` (`solidb-graph`), run by `graph.sh`.

### What runs

| Configuration | Image | Client | Query language |
|---|---|---|---|
| `solidb` | `ghcr.io/solisoft/solidb:v2.10.3`, `SOLIDB_ENGINE=solistore` | `solidb-client`, native MessagePack driver, query cache off | SDBQL traversals and `SHORTEST_PATH` |
| `neo4j` | `neo4j`, Community | `neo4rs` 0.8, Bolt | Cypher |
| `memgraph` | `memgraph/memgraph`, Community, `latest` on 2026-11-14 | `neo4rs` 0.8, Bolt | Cypher |
| `arango` | `arangodb/arangodb` 3.12.4-3 | HTTP API through `reqwest`, `/_api/cursor` | AQL traversals |
| `postgres` (by name only, not in `all`) | `postgres` 18.6 | `tokio-postgres`, prepared statements | SQL: joins, and a recursive CTE for the shortest path |

**PostgreSQL is not in `./graph.sh all`.** It has an adapter (`src/graph/engines/postgres.rs`), but it is not a graph database, and it was left out of the comparison on 2026-11-14. `./graph.sh postgres` still runs it.

**The rules are those of the document benchmark:**
- the same machine, rocky-build2, with one database running at a time;
- each database in a container limited to `--cpuset-cpus 0-3,16-19` and `--memory 16g` (`--memory-swap` the same, so no swap), the harness on `8-15,24-31` through `taskset`;
- each database through its own protocol and client, as in the table;
- the container's `inspect` output kept beside the result. The `docker run` command is printed to the terminal; unlike `run.sh`, `graph.sh` does not write it to a file.

**Pinning.** The Neo4j, Memgraph, ArangoDB and PostgreSQL images are pinned by digest in `graph.sh`; the versions in the table are the comments beside those digests. The SoliDB image is named by tag (`v2.10.3`), not by digest, unlike in `run.sh`. The Neo4j version behind its digest is not written in `graph.sh`; the result's `server_version` holds it. `Cargo.lock` as committed has no entry for `neo4rs`: the manifest asks for `0.8`, and the exact version is the one the first build resolves.

### The data

`src/graph/data.rs`. Everything is a function of the vertex numbers through a fixed hash, so two runs with the same sizes load the same graph, and the harness can hold all of it in memory.

| | Default | What |
|---|---|---|
| People | 1,000,000 (`PEOPLE`) | numbered from 0; each has `n`, a `country` among 20, an `age` from 18 to 77 |
| Products | 10,000 (`PRODUCTS`) | numbered from 0; each has `n` |
| `knows` | about ten per person | directed, person to person, with `since` (a year, 2000 to 2024) and `weight` (1 to 9) |
| `bought` | at most three per person | person to product, no attribute |

- **Degree.** Each person draws 2 to 18 targets, uniformly, so ten on average. A draw that falls on the person, or twice on the same target, is dropped: there is no self-loop and no parallel edge, and the out-degree is at most the number drawn.
- **Locality.** Four draws in five go to one of the 2,000 people whose numbers follow the person's own (wrapping at the end).
- **Hubs.** One draw in five goes to person `people × u³`, `u` uniform in [0, 1): low numbers are known by many, and the in-degree is skewed toward them.
- **Purchases.** Each person draws 0 to 3 products, uniformly among all, duplicates dropped.
- **Exact counts.** The number of edges and of purchases after the duplicates are dropped is printed at the start of a run and written to the result (`edges`, `purchases`). It is not quoted here for the default size, which has not been run.

`weight` and `age` are loaded and read by no scenario.

**The load.** People, then products, then `knows`, then `bought`, from eight connections in batches of 2,000 rows, one request per batch. The indexes a lookup by number needs exist before the load; so do the edge indexes, in every engine that has them (see [Settings and indexes](#settings-and-indexes)).

### The six scenarios

Every scenario but `shortest` returns one row, `(count, sum of n)`, over a set of **distinct** people that never includes the start. `shortest` returns a number of edges.

| Scenario | What it asks |
|---|---|
| `hop1` | The people the start knows. |
| `hop2` | The people within two hops of the start, following `knows` forward: at one hop or at two. |
| `hop3` | The same within three hops. |
| `filtered` | The people exactly two edges away whose second edge has `since >= @year` and who live in `@country`. A person who is also a direct neighbour counts. The year is drawn from 2005 to 2019, the country among the 20. |
| `fof_bought` | The people within two hops who bought a given product. For half of the parameter sets the product is one that somebody within two hops bought, when there is one; for the others it is drawn among all products, and the answer is often empty. |
| `shortest` | The number of edges on a shortest path, by hops, from one person to another, following `knows` forward. The target is chosen exactly 2, 3 or 4 hops away, in turn, so a path always exists. |

**Parameters.** For each scenario the harness prepares 2,000 parameter sets, deterministically, with the answer the graph gives for each. Every worker cycles through the same 2,000, starting at a different offset.

**Timing.** Each scenario runs at concurrency 1 and 8 (`CONCURRENCY`): 5 s of warm-up, then 3 measured runs of 20 s (`WARM`, `SECS`, `RUNS`). A request is timed from the call into the client to the decoded answer. All six scenarios are reads; nothing is written after the load.

**The query each database receives.** Copied from the adapters; the same text is written to the result under `queries`. SoliDB and ArangoDB receive a person as `"people/<n>"` and a product as `"products/<n>"`; the others receive the number.

`hop1`, `hop2`, `hop3` — `{d}` is 1, 2 or 3, written into the text:

| Engine | Query |
|---|---|
| SoliDB | `FOR v IN 1..{d} OUTBOUND @start knows COLLECT AGGREGATE c = COUNT(), s = SUM(v.n) RETURN [c, s]` |
| Neo4j, Memgraph | `MATCH (a:Person {n: $n})-[:KNOWS*1..{d}]->(b) WHERE b <> a WITH DISTINCT b RETURN count(b) AS c, coalesce(sum(b.n), 0) AS s` |
| ArangoDB | `FOR v IN 1..{d} OUTBOUND @start knows OPTIONS { uniqueVertices: "global", order: "bfs" } COLLECT AGGREGATE c = COUNT(1), s = SUM(v.n) RETURN [c, s]` |
| PostgreSQL, `hop1` | `SELECT count(*), coalesce(sum(dst), 0)::bigint FROM knows WHERE src = $1` |
| PostgreSQL, `hop2` | `WITH h1 AS (SELECT dst FROM knows WHERE src = $1), h2 AS (SELECT k.dst FROM knows k JOIN h1 ON k.src = h1.dst) SELECT count(*), coalesce(sum(n), 0)::bigint FROM (SELECT dst AS n FROM h1 UNION SELECT dst FROM h2) u WHERE n <> $1` |
| PostgreSQL, `hop3` | `WITH h1 AS (SELECT dst FROM knows WHERE src = $1), h2 AS (SELECT DISTINCT k.dst FROM knows k JOIN h1 ON k.src = h1.dst), h3 AS (SELECT k.dst FROM knows k JOIN h2 ON k.src = h2.dst) SELECT count(*), coalesce(sum(n), 0)::bigint FROM (SELECT dst AS n FROM h1 UNION SELECT dst FROM h2 UNION SELECT dst FROM h3) u WHERE n <> $1` |

`filtered`:

| Engine | Query |
|---|---|
| SoliDB | `LET ns = (FOR v, e IN 2..2 OUTBOUND @start knows OPTIONS { uniqueVertices: "path" } FILTER e.since >= @year AND v.country == @country RETURN DISTINCT v.n) RETURN [LENGTH(ns), SUM(ns)]` |
| Neo4j, Memgraph | `MATCH (a:Person {n: $n})-[:KNOWS]->(m)-[e:KNOWS]->(b) WHERE e.since >= $year AND b.country = $country AND b <> a WITH DISTINCT b RETURN count(b) AS c, coalesce(sum(b.n), 0) AS s` |
| ArangoDB | the same text as SoliDB |
| PostgreSQL | `SELECT count(*), coalesce(sum(n), 0)::bigint FROM ( SELECT DISTINCT p.n FROM knows a JOIN knows b ON b.src = a.dst JOIN people p ON p.n = b.dst WHERE a.src = $1 AND b.since >= $2 AND p.country = $3 AND b.dst <> $1) u` |

`fof_bought`:

| Engine | Query |
|---|---|
| SoliDB | `LET buyers = (FOR b IN INBOUND @product bought RETURN b.n) LET ns = (FOR v IN 1..2 OUTBOUND @start knows FILTER v.n IN buyers RETURN v.n) RETURN [LENGTH(ns), SUM(ns)]` |
| Neo4j, Memgraph | `MATCH (a:Person {n: $n})-[:KNOWS*1..2]->(b)-[:BOUGHT]->(:Product {n: $product}) WHERE b <> a WITH DISTINCT b RETURN count(b) AS c, coalesce(sum(b.n), 0) AS s` |
| ArangoDB | `LET buyers = (FOR b IN 1..1 INBOUND @product bought RETURN b.n) LET ns = (FOR v IN 1..2 OUTBOUND @start knows OPTIONS { uniqueVertices: "global", order: "bfs" } FILTER v.n IN buyers RETURN v.n) RETURN [LENGTH(ns), SUM(ns)]` |
| PostgreSQL | `WITH h1 AS (SELECT dst FROM knows WHERE src = $1), h2 AS (SELECT k.dst FROM knows k JOIN h1 ON k.src = h1.dst) SELECT count(*), coalesce(sum(u.n), 0)::bigint FROM (SELECT dst AS n FROM h1 UNION SELECT dst FROM h2) u JOIN bought b ON b.person = u.n AND b.product = $2 WHERE u.n <> $1` |

`shortest`:

| Engine | Query |
|---|---|
| SoliDB | `FOR v, e, p IN SHORTEST_PATH @from TO @to OUTBOUND knows RETURN LENGTH(p.edges)` |
| Neo4j | `MATCH (a:Person {n: $from}), (b:Person {n: $to}) MATCH p = shortestPath((a)-[:KNOWS*..15]->(b)) RETURN length(p) AS l` |
| Memgraph | `MATCH p = (a:Person {n: $from})-[:KNOWS *BFS ..15]->(b:Person {n: $to}) RETURN size(relationships(p)) AS l` |
| ArangoDB | `LET p = (FOR v IN OUTBOUND SHORTEST_PATH @from TO @to knows RETURN 1) RETURN LENGTH(p) - 1` |
| PostgreSQL | `WITH RECURSIVE r(id, d) AS ( SELECT $1::int, 0 UNION SELECT k.dst, r.d + 1 FROM r JOIN knows k ON k.src = r.id WHERE r.d < 6) SELECT min(d)::bigint FROM r WHERE id = $2` |

SoliDB's `hop` and `fof_bought` queries pass no `OPTIONS`: an SDBQL traversal is breadth-first with global vertex uniqueness by default (`TraversalOptions` in `src/sdbql/ast.rs`), which ArangoDB is asked for explicitly.

### What is checked

The harness holds its own copy of the graph and computes the answer to every prepared request itself (`within`, `filtered`, `fof_bought` and `someone_at` in `data.rs`). What it compares is `(count, sum of the people's numbers)`, or the path length.

- **Before timing.** The first 50 parameter sets of a scenario are asked once. A wrong answer or an error skips the scenario: the result holds `{"scenario": …, "skipped": "wrong answer: …"}` (or `"error: …"`) with the first mismatch, and no timing.
- **During timing.** Every answer of every run is compared the same way. Each run reports `wrong` and `errors`, and `first_problem` holds the first of either.
- **What that does not cover.** A wrong answer during a timed run is counted in `wrong`, and its latency still enters `ops_per_sec` and the percentiles: a run with `wrong` above zero is to be read as wrong, not as a throughput. An error is not counted as an operation, and is followed by a 10 ms pause. A request with no answer within 120 s counts as an error and ends its worker for that run.
- **A sum can hide a wrong set** only if another set of the same size has the same sum; the check is `(count, sum)`, not the list of people.

### Settings and indexes

What `graph.sh` passes beyond the limits above:

| Configuration | Settings |
|---|---|
| `solidb` | `SOLIDB_ENGINE=solistore`; everything else at its defaults, so `SOLIDB_DURABILITY` is `eventual` |
| `neo4j` | heap 4 GB, initial and maximum (`NEO4J_server_memory_heap_initial__size`, `_max__size`); page cache 8 GB (`NEO4J_server_memory_pagecache_size`). The two share the container's 16 GB. |
| `memgraph` | `--memory-limit=14000`, `--storage-snapshot-interval-sec=0`, `--storage-wal-enabled=false`, `--log-level=WARNING`; no volume |
| `arango` | `ARANGODB_OVERRIDE_DETECTED_TOTAL_MEMORY=16G`, `ARANGODB_OVERRIDE_DETECTED_NUMBER_OF_CORES=8` (the eight CPUs of the set). Without them it sizes itself from the host. |
| `postgres` | `shared_buffers=4GB`, `effective_cache_size=12GB`, `max_connections=200`, `synchronous_commit=off`, `--shm-size 1g`; `ANALYZE` after the load |

How each engine finds a start vertex and its edges:

| Engine | Start vertex | Edges |
|---|---|---|
| SoliDB | the document key, `people/<n>` | the indexes of the edge collection, below |
| Neo4j | `CREATE INDEX person_n … FOR (p:Person) ON (p.n)`, and the same on `Product`, awaited with `db.awaitIndexes` | the adapter creates no index on relationships |
| Memgraph | `CREATE INDEX ON :Person(n)`, `CREATE INDEX ON :Product(n)` | the adapter creates no index on relationships |
| ArangoDB | the document key | the edge index an edge collection (`type: 3`) carries from its creation |
| PostgreSQL | `people (n)` primary key | `knows`: primary key `(src, dst)` and an index on `(dst)`; `bought`: primary key `(person, product)` and an index on `(product)` |

**SoliDB's edge indexes.** `Database::create_edge_indexes` (`src/storage/database.rs`), called when an edge collection is created, gives it four persistent indexes: `_edge_from_idx` on `_from`, `_edge_to_idx` on `_to`, `_edge_out_idx` on `[_from, _to]` and `_edge_in_idx` on `[_to, _from]`. The entries of the two-field indexes say where an edge leads, so a traversal can read a vertex's neighbours from them without the edge documents. Three things to know:

- **The two-field indexes are not in 2.10.3.** That release creates only `_edge_from_idx` and `_edge_to_idx`. The two-field indexes, and the cache described under [Known limits](#known-limits-and-fairness), are on the `perf/traversal` branch, in no tagged release on 2026-11-14. The default image of `graph.sh` therefore runs without either; measuring them means building an image from that branch and passing it as `SOLIDB_IMAGE`.
- **The adapter adds two more.** `engines/soli.rs` also creates `idx_from` on `_from` and `idx_to` on `_to` on both edge collections. `create_index` refuses a name already taken, not a field already indexed, so as the code reads the collection carries a second single-field index on each end, and SoliDB's load maintains all of them. This was read in the code, not observed on a running server.
- **The server version is not in SoliDB's result.** The adapter's `version` returns the placeholder `see meta.server_version`, and `graph.sh` passes no `meta` to this harness. The image is in `solidb.container.json`.

### Running it

Linux with Docker or rootful podman, as for `run.sh`; adjust `DB_CPUS` and `CLIENT_CPUS` to the machine.

```sh
cargo build --release --bin solidb-graph
DOCKER="sudo podman" ./graph.sh all                 # solidb, neo4j, memgraph, arango, one after another
DOCKER="sudo podman" ./graph.sh solidb neo4j        # only those
DOCKER="sudo podman" ./graph.sh postgres            # PostgreSQL, by name
PEOPLE=20000 PRODUCTS=500 WARM=1 SECS=3 RUNS=1 ./graph.sh all   # a smoke pass
ONLY=hop3,shortest ./graph.sh solidb                # some scenarios only
```

| Variable | Default | What |
|---|---|---|
| `PEOPLE`, `PRODUCTS` | 1000000, 10000 | size of the graph |
| `CONCURRENCY` | `1,8` | connections per series |
| `WARM`, `SECS`, `RUNS` | 5, 20, 3 | warm-up, length of a measured run, number of runs |
| `ONLY` | all six | scenarios, comma-separated |
| `SOLIDB_IMAGE` | `ghcr.io/solisoft/solidb:v2.10.3` | the SoliDB image |
| `DB_CPUS`, `CLIENT_CPUS`, `DB_MEM` | `0-3,16-19`, `8-15,24-31`, `16g` | the container's limits and the harness's CPUs |
| `OUT` | `~/bench/results/graph-<date>` | where the results go (`BENCH_ROOT` moves `~/bench`) |

How long `./graph.sh all` takes at the default size is not established: it has not been run.

**What is written**, per configuration, in `OUT`:

| File | Holds |
|---|---|
| `<config>.json` | the result, below |
| `<config>.log` | the harness's output: the size of the graph, the load time, one line per run |
| `<config>.container.json` | `docker inspect` of the container, taken before the load |
| `<config>.server.log` | the database's log |
| `<config>.disk_bytes` | size of the data directory after the run. Memgraph has no volume, so its figure is that of an empty directory. |

The data directory is deleted after each configuration. A configuration that fails is named at the end and makes `graph.sh` exit non-zero; the others still run.

**The JSON:**
- `engine`, `flavor`, `server_version`;
- `people`, `products`, `edges`, `purchases`;
- `load`: `secs`, `vertices_secs`, `edges_per_sec` (edges and purchases over the time after the vertices);
- `warm_secs`, `measure_secs`, `runs`;
- `queries`: the text of each scenario's query, as sent;
- `series`: one entry per scenario, concurrency and run, with `ops`, `secs`, `ops_per_sec`, `p50_us`, `p90_us`, `p99_us`, `max_us`, `errors`, `wrong`, `first_problem` and `db_cpu_cores` (CPU the container's cgroup used, per second of the run); or the `skipped` entry of a scenario that failed its check.

There is no report script for this harness yet; `report.py` and `bank_report.py` do not read these files.

### Results, 2026-11-14

`results/2026-11-14-graph/`: the default size (1,000,000 people, 9,985,098 edges, 1,499,179 purchases among 10,000 products), three runs of 20 s per series after 5 s of warm-up, on rocky-build2 under the rules above. Neo4j Community 2026.10.2, Memgraph 3.14.0, ArangoDB 3.12.4-3. SoliDB is a build of the code that became 2.10.4 (image `localhost/solidb-perf:graph`, built on the machine from the `perf/traversal` branch before the release). The published image `ghcr.io/solisoft/solidb:v2.10.4` (`sha256:2e3f1a16…`) was measured afterwards, read-only: `published-2.10.4/`, within 4% of the figures below and ahead of the three others on the twelve series (`hop3` at one client: 2,968 a second, 11% over Memgraph; load 22.2 s, run with `SOLIDB_GRAPH_CACHE_MB=2048` and `SOLIDB_WRITE_BUFFER_SIZE=2GB`. Its files are those of its last pass, made after the three others; `first-pass/` holds its pass of the same run as the others, on an earlier commit of the branch, and `write-buffer-64mb/` the pass of the final commit with the default write buffer.

Requests per second, median of the three runs (p50 in parentheses). No request returned a wrong answer and no scenario was skipped, on any engine.

| One client | SoliDB | Memgraph | Neo4j | ArangoDB |
|---|---:|---:|---:|---:|
| `hop1` | 61,500 (16 µs) | 19,134 (52 µs) | 11,289 (83 µs) | 10,477 (95 µs) |
| `hop2` | 28,341 (35 µs) | 12,066 (82 µs) | 6,837 (144 µs) | 2,527 (396 µs) |
| `hop3` | 2,917 (339 µs) | 2,674 (367 µs) | 1,621 (621 µs) | 313 (3.2 ms) |
| `filtered` | 14,174 (69 µs) | 12,686 (78 µs) | 6,940 (141 µs) | 1,656 (594 µs) |
| `fof_bought` | 6,854 (146 µs) | 244 (4.0 ms) | 2,844 (349 µs) | 428 (2.3 ms) |
| `shortest` | 22,324 (36 µs) | 0.6 (1.8 s) | 8,309 (104 µs) | 7,578 (112 µs) |

| Eight clients | SoliDB | Memgraph | Neo4j | ArangoDB |
|---|---:|---:|---:|---:|
| `hop1` | 323,272 (23 µs) | 76,294 (101 µs) | 37,438 (207 µs) | 30,295 (249 µs) |
| `hop2` | 150,343 (50 µs) | 52,092 (147 µs) | 28,481 (272 µs) | 9,648 (809 µs) |
| `hop3` | 16,213 (485 µs) | 13,200 (598 µs) | 8,164 (966 µs) | 1,324 (6.0 ms) |
| `filtered` | 74,274 (103 µs) | 55,131 (140 µs) | 29,014 (267 µs) | 6,721 (1.2 ms) |
| `fof_bought` | 35,730 (215 µs) | 1,396 (5.6 ms) | 14,469 (534 µs) | 2,102 (3.8 ms) |
| `shortest` | 112,449 (56 µs) | 2.4 (3.3 s) | 31,701 (234 µs) | 25,957 (273 µs) |

Load, eight connections, indexes included: SoliDB 23.5 s, ArangoDB 26.9 s, Neo4j 49.6 s, Memgraph 101.5 s. SoliDB's figure is with a 2 GB write buffer (`SOLIDB_WRITE_BUFFER_SIZE`, set in `graph.sh`); with the server's default of 64 MB the same load takes 46.2 s (`write-buffer-64mb/`, whose query figures are within 4% of these) and with 1 GB 29.8 s.

How to read them:
- **SoliDB leads the twelve series and the load.** The narrowest leads are `hop3` at one client, 9% over Memgraph, and `filtered` at one client, 12%. The load leads only with the larger write buffer: at the default, ArangoDB loads in 27 s where SoliDB takes 46. Three earlier passes of that series on earlier commits of the branch had SoliDB 3 to 5% behind.
- **SoliDB answers from memory here.** After the warm-up, most of what a query needs was read by an earlier one and kept (see "Known limits"): the benchmark only reads, over 2,000 sets of parameters per scenario. With nothing kept (smoke passes on 20,000 people, before the kept maps existed), the same branch took 130 µs for `hop2` and 1.1 ms for `hop3` at one client. With writes arriving while it reads, see the mixed run below.
- **Memgraph's `shortest` and `fof_bought`** are the queries of `engines/bolt.rs`, which may not be the ones a Memgraph user would write: `*BFS` from one end for the first, a variable-length pattern followed by a purchase for the second.
- **Where the released version stood.** On smoke passes of 2.10.3 (20,000 people): `hop2` 560 µs, `hop3` 7.3 ms, `shortest` 10.8 ms at one client. On 2.10.2 a shortest path on 100,000 edges ran into the query time limit. Not measured at full scale.

### Results, 2026-11-14: with 1,000 edge updates a second

`results/2026-11-14-graph/writes-1000/`: the same run with `WRITES=1000`. One more connection sets the weight of 1,000 edges a second (a random person's edge each time) while every series is timed. No scenario reads the weight, so the answers are still checked: none was wrong, and every engine kept up with the 1,000 updates. SoliDB is the same branch, a few commits later: a single write now forgets only what it touches of the kept neighbourhoods, where it used to empty them.

| One client | SoliDB | Memgraph | Neo4j | ArangoDB |
|---|---:|---:|---:|---:|
| `hop1` | 59,642 (16 µs) | 17,152 (57 µs) | 11,054 (86 µs) | 8,659 (112 µs) |
| `hop2` | 26,397 (37 µs) | 10,155 (97 µs) | 6,506 (153 µs) | 2,103 (476 µs) |
| `hop3` | 2,648 (373 µs) | 2,540 (388 µs) | 1,482 (677 µs) | 280 (3.6 ms) |
| `filtered` | 13,240 (74 µs) | 12,346 (80 µs) | 6,592 (150 µs) | 1,291 (761 µs) |
| `fof_bought` | 6,400 (155 µs) | 227 (4.3 ms) | 2,459 (403 µs) | 396 (2.5 ms) |
| `shortest` | 16,890 (48 µs) | 0.6 (1.8 s) | 7,524 (117 µs) | 6,238 (140 µs) |

| Eight clients | SoliDB | Memgraph | Neo4j | ArangoDB |
|---|---:|---:|---:|---:|
| `hop1` | 265,213 (24 µs) | 69,676 (109 µs) | 38,668 (200 µs) | 28,509 (261 µs) |
| `hop2` | 128,942 (54 µs) | 47,753 (160 µs) | 28,382 (272 µs) | 8,682 (902 µs) |
| `hop3` | 14,861 (523 µs) | 12,314 (611 µs) | 7,735 (1.0 ms) | 1,171 (6.7 ms) |
| `filtered` | 70,840 (108 µs) | 52,380 (147 µs) | 27,704 (279 µs) | 5,611 (1.4 ms) |
| `fof_bought` | 33,754 (226 µs) | 1,260 (5.8 ms) | 13,376 (579 µs) | 1,927 (4.1 ms) |
| `shortest` | 82,860 (73 µs) | 2.3 (3.3 s) | 30,507 (243 µs) | 24,428 (288 µs) |

SoliDB still leads the twelve series, with narrower margins where they were narrow: `hop3` at one client by 4%, `filtered` at one client by 7%. Against its own read-only figures it loses 3 to 10% on most series, 18% on `hop1` at eight clients and 24 to 26% on `shortest`. What this run does not cover: more than 1,000 writes a second (on 20,000 people, 5,000 a second cost SoliDB 15 to 40%), inserts and deletes of edges rather than updates, writes in batches (a batch still empties the collection's kept maps), and writes to the vertices.

### Licences

The licence texts were read on 2026-11-14, to check that publishing benchmark results is allowed. None of those read restricts it.

| Software | Licence |
|---|---|
| Neo4j Community | GPLv3 |
| Memgraph Community | BSL 1.1 |
| ArangoDB | BSL 1.1. Its community licence PDF could not be read, so that document is not covered by the statement above. |
| PostgreSQL, the client crates | permissive (the PostgreSQL licence, Apache 2.0, MIT) |
| SoliDB | the O'Saasy licence (`LICENCE.md`); its Rust client is MIT |

### Known limits and fairness

- **Cypher enumerates paths; SoliDB and ArangoDB do not.** `[:KNOWS*1..3]` matches every path of one to three relationships that repeats no relationship, and `DISTINCT b` reduces them to people afterwards. SoliDB and ArangoDB walk breadth-first with global vertex uniqueness and reach each person once. The answers are the same; the work asked for is not, and it grows with the depth and around the hubs. This is the query a Cypher user would write for the question. Whether Neo4j's or Memgraph's planner avoids the enumeration was not examined, and no procedure-based alternative (APOC, Memgraph's own BFS expansion for the hop scenarios) was tried.
- **`fof_bought` is not the same plan everywhere.** SoliDB and ArangoDB first collect the product's buyers, then filter the traversal with `v.n IN buyers`; Cypher extends each path by a `BOUGHT` relationship; PostgreSQL joins. Each is the direct way to write it in that language.
- **Memgraph runs in memory, with snapshots and the WAL off.** Nothing it loads reaches the disk. The other three store the graph on disk and read it through a cache sized within the same 16 GB.
- **The shortest-path query differs between Neo4j and Memgraph.** Neo4j gets `shortestPath` with `*..15`; Memgraph gets its `*BFS` expansion with the same bound. They are different operators. ArangoDB's and SoliDB's `SHORTEST_PATH` are given no bound. PostgreSQL's recursive CTE stops at depth 6 and returns the least depth; the targets are at most four hops away, so the bounds do not change an answer.
- **SoliDB keeps what a traversal reads, and a read-only benchmark favours that.** On the `perf/traversal` branch (module `neighbourhoods` in `src/sdbql/executor/execution/graph.rs`), SoliDB keeps between queries, per collection:
  - where each expanded vertex's edges lead, one map per edge collection and direction (and a second kind that also holds each edge's key);
  - the vertex documents a traversal read, one map per set of fields read.

  A map is valid for one write generation of its collection: any committed write to that collection makes the whole map stale, and the next query starts an empty one. Maps are filled as queries expand vertices, not built ahead. Each map holds at most `SOLIDB_GRAPH_CACHE_MB` (default 256; `0` turns the cache off) by its own estimate of entry sizes; past that, new entries are not kept and nothing is evicted. The limit is per map, not for the whole server.

  This benchmark writes nothing after the load, and its 2,000 parameter sets repeat, so once the maps hold those neighbourhoods and vertices SoliDB can answer from them without reading storage. That is the position an in-memory graph database is in, and the result says nothing about SoliDB on a graph that is written while it is read. No write-mixed scenario exists in this harness, and no run with `SOLIDB_GRAPH_CACHE_MB=0` has been made; `graph.sh` does not pass that variable through, so such a run needs the `docker run` line edited.
- **The other engines' caches are warmed by the same repetition.** Every engine sees the same 2,000 requests again and again, and whatever it caches stays warm. How much each gains was not measured.
- **Concurrency stops at 8.** The document benchmark goes to 32; this one runs 1 and 8 by default.
- **Durability plays no part.** Nothing is written during the timed runs. The load is timed and reported, but the engines load under different guarantees (SoliDB's background fsync, Memgraph with no log, PostgreSQL with `synchronous_commit=off`, the others at their defaults), so `load` is not a like-for-like figure.
- **One client library for two servers.** Neo4j and Memgraph are both driven through the `neo4rs` crate. No other Bolt driver was tried.
- **One size.** The default is a million people with about ten `knows` edges each. Whether that fits each engine's cache within 16 GB was not checked, and nothing here measures a graph known to be larger than the cache.

## Time series (`ts.sh`)

The same points and the same five questions against SoliDB, QuestDB and
TimescaleDB, one database at a time, in a container with 4 cores (8 threads)
and 16 GB as everywhere on this page; the client is `solidb-ts`
(`src/ts/`), pinned to other cores.

### The data and the scenarios

1,000 series, one point every 10 seconds for 24 hours: 8,640,000 points
`{series, ts, v}`, loaded in time order from 8 connections in batches of
5,000. Then, at 1 and at 8 clients, 5 s of warm-up and three runs of 20 s
per scenario (the table gives the median run):

| Scenario | What one request asks |
|---|---|
| `range` | count and sum of one series over one hour (360 points) |
| `bucket` | the five-minute averages of one series over six hours (72 rows from 2,160 points) |
| `last` | the latest point of one series |
| `window_all` | count and sum of every series over ten minutes (60,000 points) |
| `insert` | one new point, acknowledged |

Every answer is compared with what the generator computes for it; a
scenario whose first answers are wrong is not timed. None was.

### How each database is used

- **SoliDB** (solistore engine): a `timeseries` collection, persistent
  indexes on `[series, ts]` and `[ts]`, column blocks on `ts`, `series`,
  `v` (`PUT …/blocks`), sealed before the readings (`POST …/blocks/seal`);
  SDBQL over the driver with bind variables. `SOLIDB_TS_BLOCKS=0` runs it
  without the blocks.
- **QuestDB** 10.2.0: `SYMBOL` series, designated timestamp, partitions by
  hour, WAL table; writes over the InfluxDB line protocol on HTTP, reads
  over the PostgreSQL wire with prepared statements, `SAMPLE BY` and
  `LATEST ON`. The harness waits for the WAL to be applied before reading.
- **TimescaleDB** 2.32.0 on PostgreSQL 17.11: a hypertable on `ts`, an index
  on `(series, ts DESC)`, prepared statements, `time_bucket`,
  `synchronous_commit = off`, 4 GB of shared buffers, `ANALYZE` after the
  load.

The exact statements are in each result file (`queries`).

### Results, 2026-11-15

SoliDB is a build of the `bench/timeseries` branch (2.10.4 plus column
blocks), not a published image. Requests per second, median of three runs;
in bold the highest of the row.

| scenario | clients | SoliDB | QuestDB | TimescaleDB | p99 µs (S / Q / T) | DB cores (S / Q / T) |
|---|---|---|---|---|---|---|
| range | 1 | **42,690** | 21,040 | 6,510 | 25 / 102 / 184 | 0.7 / 7.9 / 0.9 |
| range | 8 | **211,715** | 62,763 | 27,420 | 65 / 735 / 351 | 3.8 / 7.8 / 7.5 |
| bucket | 1 | **10,505** | 6,598 | 1,200 | 111 / 258 / 1,019 | 0.9 / 8.0 / 1.0 |
| bucket | 8 | **57,773** | 14,105 | 6,779 | 250 / 4,613 / 1,403 | 6.0 / 7.9 / 7.8 |
| last | 1 | **57,616** | 55,114 | 43,336 | 19 / 20 / 26 | 0.5 / 7.8 / 0.6 |
| last | 8 | 289,577 | **338,912** | 159,400 | 42 / 49 / 62 | 4.2 / 7.0 / 6.0 |
| window_all | 1 | **38,160** | 35,839 | 306 | 29 / 32 / 3,487 | 0.7 / 7.9 / 1.0 |
| window_all | 8 | **187,967** | 129,426 | 1,232 | 70 / 571 / 6,866 | 4.3 / 7.5 / 8.0 |
| insert | 1 | **57,050** | 45,147 | 25,437 | 20 / 29 / 64 | 0.7 / 7.7 / 0.8 |
| insert | 8 | 204,467 | **250,359** | 82,846 | 137 / 124 / 190 | 3.7 / 7.2 / 6.6 |

Load: QuestDB 6.65 M points/s (1.3 s), TimescaleDB 1.17 M (7.4 s), SoliDB
1.09 M (7.9 s, of which the sealing of blocks is about a fifth: 6.7 s
without them). On disk after the run, the points of the `insert` scenario
included: TimescaleDB 3.2 GB, QuestDB 4.0 GB, SoliDB 9.3 GB. After the
shorter run below, which inserts nothing: 2.1, 0.6 and 4.6 GB.

What the table says, and what it does not:

- SoliDB leads eight rows of ten, QuestDB two (`last` and `insert` at 8
  clients), and QuestDB loads six times faster and takes far less disk.
- `last` and `window_all` at one client are within 5 to 6%: near ties.
- Without column blocks SoliDB 2.10.4 answered `range` at 925/s, `bucket`
  at 163/s and `window_all` at 9.7/s (one 10 s run each, same data): the
  blocks are what this table measures on those three rows.
- QuestDB keeps its workers spinning: it uses its 8 threads at one client.
  The other two use one core there.
- A point inserted is in SoliDB's and TimescaleDB's answers at once; in
  QuestDB's once its WAL is applied.
- The readings follow a load and a seal: no point arrives while a reading
  runs, so nothing is outside the blocks. Under ingestion the newest points
  (up to about 1,500) are read as documents.

### With values that round (`TS_VALUES=thirds`)

The values above are whole numbers, whose sums are exact in floating point
whatever the method. SoliDB's `SUM` is exact by construction and the blocks
keep exact subtotals, so the fair question is what happens when sums round.
Same data with every value divided by three, the three aggregations, three
runs of 20 s:

| scenario | clients | SoliDB | QuestDB | TimescaleDB |
|---|---|---|---|---|
| range | 1 | **40,967** | 18,280 | 6,719 |
| range | 8 | **204,137** | 50,137 | 28,070 |
| bucket | 1 | **9,677** | 6,292 | 1,236 |
| bucket | 8 | **53,547** | 14,698 | 6,883 |
| window_all | 1 | **36,284** | 36,082 | 308 |
| window_all | 8 | **180,358** | 127,314 | 1,236 |

`window_all` at one client is a tie; the rest keeps its order. (A version of
the blocks that kept float subtotals, usable only when exact, answered
`window_all` at 3,732/s here.) Results in
`results/2026-11-15-timeseries/thirds/`.

### Running it

```bash
cargo build --release --bin solidb-ts
./ts.sh all                       # the three, one after another
TS_VALUES=thirds ./ts.sh all      # with values whose sums round
SERIES=100 HOURS=2 WARM=1 SECS=3 RUNS=1 ./ts.sh all    # a smoke pass
```

Results: `results/2026-11-15-timeseries/` (per database: the JSON with every
run, the client log, the container's description, the size on disk).
InfluxDB 3 Core, on the list of card #795, is not wired in.

## Columnar (`col.sh`)

One wide table of events and four analytical questions against SoliDB and
ClickHouse, one database at a time, in a container with 4 cores (8 threads)
and 16 GB; the client is `solidb-col` (`src/col/`), pinned to other cores.

### The data and the scenarios

10,000,000 rows `{ts, user, country, device, amount, bytes}`: one row every
100 ms, 100,000 users, 50 countries, 5 devices, amounts that are quarters of
whole numbers. Loaded from 8 connections, 20,000 rows at a time, as JSON
over HTTP on both sides. Then, at 1 and at 4 clients, 5 s of warm-up and
three runs of 20 s per scenario (the table gives the median run):

| Scenario | What one request asks |
|---|---|
| `filter_agg` | count and sum of the amounts of one country (200,000 rows) |
| `group_all` | per country, the count and the sum of bytes (every row, 50 groups) |
| `window_group` | per device, the count and the sum of bytes over a twentieth of the time span (500,000 rows) |
| `top_users` | the ten users with the largest total amount (every row, 100,000 groups) |

Every answer is compared with the generator's; none was wrong.

### How each database is used

- **SoliDB** (solistore engine): a columnar collection created over HTTP,
  rows inserted through `…/columnar/events/insert`, SDBQL over the driver
  with bind variables.
- **ClickHouse** 26.10.4: `MergeTree ORDER BY (country, ts)`, the two string
  columns `LowCardinality`, `INSERT … FORMAT JSONEachRow` over HTTP,
  `OPTIMIZE TABLE … FINAL` after the load, SQL over HTTP with the values
  written in it, defaults otherwise. The order suits `filter_agg`.

### Results, 2026-11-15

SoliDB is a build of `main` after 2.10.4 with columnar blocks, not a
published image. Requests per second, median of three runs.

| scenario | clients | SoliDB | ClickHouse | p50 ms (S / C) | DB cores (S / C) |
|---|---|---|---|---|---|
| filter_agg | 1 | **721** | 562 | 1.4 / 1.7 | 1.0 / 2.0 |
| filter_agg | 4 | **2,683** | 1,307 | 1.4 / 2.9 | 3.9 / 5.9 |
| group_all | 1 | **53.1** | 47.0 | 18.0 / 20.8 | 7.4 / 7.2 |
| group_all | 4 | **53.7** | 50.3 | 38.0 / 78.8 | 7.0 / 7.9 |
| window_group | 1 | **189** | 120 | 5.4 / 8.3 | 2.9 / 4.1 |
| window_group | 4 | **334** | 193 | 11.9 / 20.2 | 5.3 / 7.4 |
| top_users | 1 | **39.2** | 20.4 | 25.5 / 48.6 | 6.5 / 7.4 |
| top_users | 4 | **50.1** | 21.1 | 93.7 / 187.3 | 6.7 / 7.9 |

Load: SoliDB 7.0 s (1.42 M rows/s), ClickHouse 7.2 s (1.38 M rows/s).
SoliDB 2.10.4 answered each of the four at 0.4 requests/s on one million
rows (2.4 s a query, one 5 s run each) and loaded 344,000 rows/s.

What the table says, and what it does not:

- SoliDB leads the eight rows, by 7% (`group_all` at 4 clients) to 2.4 times
  (`top_users` at 4 clients).
- Disk is not compared: at the end of the run SoliDB's rows were still in
  its write buffer and log (its directory held 320 MiB of pre-allocated
  log), ClickHouse's directory 1.5 GB with its own log tables.
- SoliDB keeps every block decoded in memory, with the ids of its rows:
  1.4 GB of resident memory for these ten million rows, and no ceiling on
  it yet.
- Four questions, one table, one order for ClickHouse's table, its default
  settings. No join, no window function, no table larger than memory.
  DuckDB, on the list of card #795, is not wired in.

### With amounts that round (`COL_VALUES=thirds`)

Quarters add up exactly in floating point; thirds do not. One run of 10 s
each:


| scenario | clients | SoliDB | ClickHouse |
|---|---|---|---|
| filter_agg | 1 | **717** | 567 |
| filter_agg | 4 | **2,633** | 1,314 |
| group_all | 1 | **52.3** | 46.9 |
| group_all | 4 | **53.0** | 50.2 |
| window_group | 1 | **188** | 121 |
| window_group | 4 | **334** | 193 |
| top_users | 1 | **21.9** | 20.4 |
| top_users | 4 | **26.4** | 20.9 |

`top_users` is the row that depends on it: SoliDB keeps a plain sum and the
total of what its additions rounded away per user, which together are the
exact sum, and falls back to exact accumulators when that total itself
rounds. Results in `results/2026-11-15-columnar/thirds/`.

### Running it

```bash
cargo build --release --bin solidb-col
./col.sh all
COL_VALUES=thirds ./col.sh all
ROWS=1000000 WARM=1 SECS=5 RUNS=1 ./col.sh all    # a smaller pass
```

## SoliDB 2.7.2, and why its image was rebuilt

The first run also measured SoliDB 2.7.2, before the fixes this benchmark led to in 2.7.3 (the two-sided range read as one interval, the full-scan COLLECT that streams). Those figures are in `results/2026-11-08/solidb-2.7.2/`.

The published `ghcr.io/solisoft/solidb:v2.5.0` through `v2.7.2` images do not start.
- **The cause:** the release binary is built on `ubuntu-latest` (glibc 2.39), and the Dockerfile's base, `debian:bookworm-slim`, has glibc 2.36.
- **The workaround:** `solidb-image.sh` builds the 2.7.2 run's image from the official release binary, checked against the SHA-256 GitHub records, on `debian:trixie-slim`.
- **Fixed in 2.7.3.** That release ships a working image, which `run.sh` uses by default.
