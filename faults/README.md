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

## `probe-client-tx`

Checks what the Rust client's transaction calls do against a real server: a
write between `begin_transaction` and `rollback`, and `begin_transaction` /
`commit` on a pool of four connections. Card #793 has what it found on
2.9.0.

## Not covered yet

- **Power loss:** what was written but not fsynced, dropped below the
  filesystem (LazyFS or dm-log-writes).
- **Isolation:** concurrent transactions checked by a history checker
  (Elle), per isolation level.

Both are in #792.
