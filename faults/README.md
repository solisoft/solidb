# Fault tests

What SoliDB keeps when it is killed in the middle of writing, and what its
transactions actually do. Bonfire card #792.

The harness starts a real `solidb` binary on a data directory of its own,
writes to it, kills it with `SIGKILL` (no shutdown path runs), starts it
again on the same directory and checks what is there. It is a workspace of
its own, like `benchmarks/compare`, and is not part of the server's build.

```bash
cd faults && cargo build --release
B=target/release/solidb-faults
$B crash --bin /path/to/solidb --engine solistore --durability full --cycles 20 --out crash.jsonl
$B probe-client-tx --bin /path/to/solidb
```

`--data DIR` (deleted first) and `--port N` (default 6900) place the server.
Its output is appended to `DIR.server.log`.

## `crash`

Each cycle starts the server, runs the writers for a random 0.5 to 4 seconds
(`--min-ms`, `--max-ms`, `--seed`), kills the server while they write,
restarts it and checks everything every cycle so far has written.

**Writers:**
- **Transactions** (`--tx-writers`, 4), over the HTTP transaction API. Each
  one inserts three documents into `txdocs` and sets its writer's counter in
  `counters` to its sequence number, then commits. `--tx-isolation` picks the
  level: `read_committed` (the default) fsyncs the commit, `read_uncommitted`
  does not.
- **Single inserts** (`--single-writers`, 4), over the driver, outside any
  transaction, `--single-rate` per second each (2,000), into `singles`. That
  collection has a persistent index on `client` and a unique index on `uid`.

Each writer is sequential and stops at its first error. The harness records
how far it was acknowledged, and which request was in flight at the kill.

**Checks after each restart:**

| What | Holds when |
|---|---|
| Atomicity of a transaction | Each transaction present has all three of its documents. |
| Atomicity across collections | A writer's counter equals the last of its transactions present. |
| No holes | What survives of a writer is exactly its first *n* requests, for transactions and inserts alike. |
| Durability | Every acknowledged transaction and insert is present. `SOLIDB_DURABILITY=eventual` only allows a loss on a power failure, and this is a process kill. |
| Nothing invented | Nothing is present beyond what was sent or in flight. |
| Refusals | A request the server refused is absent. |
| Persistent index | The keys the index on `client` finds for a writer are exactly the documents in its key range; the index on `tx` finds three documents for a transaction present and none for the next one. |
| Unique index | It finds a sample of documents, nothing for the next sequence number, and refuses a second document with an existing `uid`. |
| Counts | `COLLECT WITH COUNT` equals the documents the writers' key ranges hold. |

Documents are read by key range (`d._key >= @lo AND d._key < @hi`), from the
document store; indexes are only ever compared with that.

**Output:** one JSON line per cycle, with what was acknowledged and in
flight, what is present, the restart time and every violation. The exit
status is 1 when there is one.

## Results

`results/2026-11-09/`: the published v2.9.0 binary on rocky-build2, 20 cycles
per configuration, `--seed 7`, one JSON line per cycle.

| Configuration | Transactions acknowledged | Inserts acknowledged | In flight at the kills | Violations |
|---|---|---|---|---|
| RocksDB, commit fsynced (`read_committed`) | 360,672 | 306,340 | 110 | 0 |
| RocksDB, commit not fsynced (`read_uncommitted`) | 422,581 | 306,350 | 100 | 0 |
| solistore `eventual`, commit fsynced | 391,162 | 306,211 | 115 | 0 |
| solistore `eventual`, commit not fsynced | 459,572 | 306,283 | 100 | 0 |
| solistore `full` | 369,109 | 306,364 | 114 | 0 |

A process kill lost nothing acknowledged on either engine, in either
durability mode, and left no transaction half applied. The requests in flight
landed or not, never partly.

## `isolation`

Concurrent transactions of reads and appends on a few hot lists, recorded
as a history in Elle's list-append format (`--history`), then checked for
what needs no cycle search:

| Anomaly | Meaning |
|---|---|
| `lost-append` | an append whose transaction committed is not in the final list |
| `aborted-read` | a value whose transaction failed is read, or kept |
| `incompatible-order` | two reads of a list where neither is a prefix of the other |
| `duplicate-elements`, `garbage-read` | a value twice in one read, or never appended |
| `internal` | a read after the transaction's own append does not end with it |

`--mode` picks the path: `doc` (HTTP transaction, documents read with `GET`
and written with `PUT` by key), `sdbql` (HTTP transaction, `FOR d … UPDATE d
WITH {vals: PUSH(d.vals, @v)}` through `…/query`) or `cursor` (the same
statement through `/cursor`, no transaction). `--keys` lists are in use at a
time; each is retired after `--max-writes` appends (32), as Jepsen does, so
the history stays small enough for Elle (`elle-cli --model list-append`),
which finds the cycles (G0, G1c, G2, write skew).

`results/2026-11-09/isolation/`: 10 clients, 8 active keys, 10 s, seed 11,
the published v2.9.0 binary (`solidb-*`) and a build of `main` with the fix
for card #796 (`solidb-guard-*`).

| Path | v2.9.0: committed appends lost | Fixed | Fixed: refused instead |
|---|---|---|---|
| `cursor` (no transaction) | 24,785 of 405,507 | 0 | — |
| `sdbql`, `read_committed` | 8,207 of 126,137 | 0 | 8,984 transactions (stale write) |
| `sdbql`, `serializable` | 8,112 of 125,047 | 0 | 8,844 transactions (stale write) |
| `doc`, `serializable` | 0 | 0 | — (about 70% of transactions refused on lock conflicts, both versions) |

The `internal` anomalies of the `sdbql` path (about 7,700 per run) are the
documented behaviour: a query in a transaction reads committed data, not the
transaction's own writes.

### Elle

The same histories through Elle's list-append checker (elle-cli 0.1.11,
`-c serializable`, run on 2026-11-10 in an `eclipse-temurin:21-jdk`
container; the JRE image lacks the `jdk.random` module and graphviz is absent,
hence `-b 0`):

```bash
java -Xmx40g -jar elle-cli-0.1.11-standalone.jar -m list-append -f json -v json \
  -c serializable -b 0 -d out/NAME NAME.history.json
```

Counts per anomaly, and the weakest models Elle rules out (`not`), are in
`results/2026-11-09/isolation/elle-summary.json`. On v2.9.0's `sdbql` runs
the cycle search timed out on about 100 components, so its counts are lower
bounds; the fixed runs had no timeout.

| Path | v2.9.0 | Fixed (#796) |
|---|---|---|
| `cursor` (no transaction) | not read committed: 1,509 `incompatible-order` (the lost appends) | **valid** |
| `doc`, `serializable` | **valid** | **valid** |
| `sdbql`, `read_committed` | not even read uncommitted: G0 504, G1c 1, lost-update 100, G-single 813, G2 262 | not cursor stability nor read atomic: lost-update 85, G-single 4,512, G2 3,541 |
| `sdbql`, `serializable` | not even read uncommitted: G0 560, G1c 1, lost-update 79, G-single 826, G2 235 | the same as `read_committed`: lost-update 88, G-single 4,512, G2 3,393 |

What it means:
- **Documents by key in a transaction** (`doc`: two-phase locks, no wait) are
  serializable in these histories, on both versions.
- **The #796 guard removed the dirty writes** (G0, G1c): the fixed `sdbql`
  path is read committed. It is no more than that, at any level:
  - a query in a transaction reads committed data without a lock, so the same
    key read twice in one transaction can return two values (G-single);
  - the guard checks the revision the `UPDATE` itself read, not one an
    earlier statement of the transaction read, so read-then-update is a lost
    update in Elle's sense (no append is lost: `PUSH` reads the latest list);
  - keys that are only read are never validated at commit (G2, write skew).
- `internal` stays the documented read-your-own-writes gap.

### After card #802 (2026-11-10)

`results/2026-11-10/isolation/`: a build of `main` with the read checks of
card #802, `--mode sdbql`, 10 clients, 8 active keys, 30 s, seed 11, run on a
16-core workstation; Elle as above.

| Level | Committed | Refused | Harness anomalies | Elle `-c serializable` |
|---|---|---|---|---|
| `serializable` | 215,631 | 248,356 | none | **valid** |
| `repeatable_read` | 184,666 | 213,246 | none | **valid** |
| `read_committed` | 264,470 | 166,646 | none | G-single 10,429, G2 5,856, lost-update 181, internal 4,760 |

At the two upper levels a transaction's reads are recorded and checked at
commit (`docs/transactions` on the site). The refusals are locks held by a
writer (about 70%), a read that changed (about 29%) and stale writes.
`read_committed` no longer shows the harness's own `internal` anomaly: a
query now reads by key what its transaction staged. Elle's `internal` there
is a key read twice at two versions.

### With locks that wait (2026-11-10, after 2.9.3)

A build of `main` whose transactions wait for a document lock (500 ms at
most; a deadlock is refused at once) and, at `read_committed`, evaluate an
`UPDATE` again on the current document once they hold it. Same workload,
15 s, same workstation.

| Mode, level | Committed | Refused | Harness anomalies | Elle `-c serializable` |
|---|---|---|---|---|
| `sdbql`, `serializable` | 107,295 | 122,316 | none | **valid** |
| `sdbql`, `repeatable_read` | 109,863 | 125,012 | none | **valid** |
| `doc`, `serializable` | 89,684 | 124,878 | none | **valid** |
| `sdbql`, `read_committed` | 144,008 | 3,426 | none | not run (the level allows anomalies) |

At `read_committed` 2.3% of the transactions are refused, all of them
deadlocks, where 39% were. The two upper levels do not wait for a write
lock, so their refusals are as before. In `doc` mode nearly every refusal
is two transactions that read a key and then both want to write it.

## `probe-client-tx`

Checks what the Rust client's transaction calls do against a real server: a
write between `begin_transaction` and `rollback`, and `begin_transaction` /
`commit` on a pool of four connections. Card #793 has what it found on
2.9.0.

## Not covered yet

- **Power loss:** what was written but not fsynced, dropped below the
  filesystem (LazyFS or dm-log-writes).

In #792.
