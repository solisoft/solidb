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

### Running it

```sh
cargo build --release --bin solidb-bank
DOCKER="sudo podman" ./bank.sh all                        # about 45 minutes
WARM=1 SECS=3 RUNS=1 CYCLES=2 DOCKER="sudo podman" ./bank.sh all   # a quick pass
python3 bank_report.py ~/bench/results/<date>/bank
```

`bank.sh solidb-2.9.2` measures the 2.9.2 image over HTTP and over the driver; `ONLY_MODES="driver run"` limits a configuration to those modes; `SOLIDB_MAIN=<image> bank.sh solidb-main` does the same for an unreleased build. For profiling, `BANK_STEPS=1` prints the mean time of each step of a driver transfer, and `--audit-ms 0` turns the auditor off. `bank.sh solidb-2.9.0` measures the 2.9.0 image. That version's transactional query reads only `bind_vars` (`bindVars` was ignored until 2.9.1), so the harness sends `bind_vars`, which both versions read.

## SoliDB 2.7.2, and why its image was rebuilt

The first run also measured SoliDB 2.7.2, before the fixes this benchmark led to in 2.7.3 (the two-sided range read as one interval, the full-scan COLLECT that streams). Those figures are in `results/2026-11-08/solidb-2.7.2/`.

The published `ghcr.io/solisoft/solidb:v2.5.0` through `v2.7.2` images do not start.
- **The cause:** the release binary is built on `ubuntu-latest` (glibc 2.39), and the Dockerfile's base, `debian:bookworm-slim`, has glibc 2.36.
- **The workaround:** `solidb-image.sh` builds the 2.7.2 run's image from the official release binary, checked against the SHA-256 GitHub records, on `debian:trixie-slim`.
- **Fixed in 2.7.3.** That release ships a working image, which `run.sh` uses by default.
