# Changelog

## [Unreleased]

### Performance

* **jemalloc now serves RocksDB too, not just Rust.** It was linked with its
  `_rjem_` prefix, so it backed Rust's `GlobalAlloc` and nothing else: every C++
  allocation in RocksDB — block cache, table readers, memtables, iterators,
  compaction buffers, i.e. the bulk of a large instance's memory — went to
  glibc, whose per-thread arena growth is the very thing this dependency was
  added to avoid. Enabling
  `tikv-jemallocator/unprefixed_malloc_on_supported_platforms` routes C and C++
  `malloc` through jemalloc as well, where the existing
  `background_thread` + 2s decay tuning finally applies to it.
  * Measured on the same 613-collection checkpoint, importing 400 000 documents
    across 200 collections under the prod profile, once per variant:

    | | glibc | jemalloc |
    |---|---|---|
    | peak RSS (`VmHWM`) | 1881 MB | 1356 MB |
    | glibc arenas (~64 MB each) | 4 → 20 | 4 → 2 |
    | `[heap]` (brk) | 1094 MB | absent |
    | still held afterwards | 1726 MB, swapped out and never returned | — |
  * This came out of a 613-collection instance being OOM-killed at 21.7 GB RSS
    while holding 6.3 GB of data. The tempting explanation — 613 collections
    times the 64 MB per-collection write buffer is 38.3 GB — is wrong:
    memtables measure 2.2 MB at idle and plateau around 202 MB during that
    import while RSS keeps climbing. The growth tracks bytes written, not
    collections open.
  * The tuning symbol is coupled to the prefix: with this feature it is plain
    `malloc_conf`, and `_rjem_malloc_conf` without. Changing one without the
    other loses the tuning silently, which is what `log_allocator_tuning`
    reports at startup — it prints `background_thread=true` when the conf
    arrived.
  * Not established: whether this alone accounts for the 21.7 GB. One run per
    variant, and 400 000 documents peak at 1.9 GB, not 21.7. The three
    unbounded storage knobs (`--memtable-budget`, `--bounded-index-cache`,
    `--max-open-files`) remain worth setting on large instances.

### Features

* **`/metrics` now attributes memory per component.** A 613-collection instance
  was OOM-killed at 21.7 GB RSS while holding 6.3 GB of data, and there was no
  way to tell which consumer grew — the arithmetic that *looks* like it explains
  that (613 collections times the 64 MB per-collection write buffer) is not
  evidence, because several other consumers are unbounded too. New gauges:
  `solidb_memtable_bytes` and `_total_bytes` (the gap between them is flush
  backlog), `solidb_table_readers_bytes` (index and filter blocks pinned per
  open SST — outside the block cache, and never evicted, whenever
  `--bounded-index-cache` is off and `--max-open-files` is -1, which are the
  prod defaults), `solidb_block_cache_bytes` and `_pinned_bytes`,
  `solidb_sst_files`, `solidb_column_families`,
  `solidb_cached_collection_handles`, and six `solidb_jemalloc_*_bytes` from the
  allocator's own accounting.
  * The jemalloc figures are what separate live data from fragmentation and from
    address space merely kept mapped: the killed process showed 113 GB virtual
    against 21.7 GB resident, and `stats.retained` is the difference between
    that being a leak and it being normal.
  * The block cache is one shared LRU for every column family, so its two
    gauges are read from a single handle — summing them per column family would
    multiply the shared cache by the collection count.
  * Computed on demand at scrape time, never on a timer: it is one FFI property
    read per column family, and this repository has already had to fix a stats
    collector that read ten of them per collection every five seconds.
  * Requires the `stats` feature on `tikv-jemalloc-ctl`, which builds the C
    library with its counters enabled.
  * `solidb_cached_collection_handles` counts the per-database caches, where the
    handles actually live — each carries a `tokio::sync::broadcast` ring, and on
    a 613-collection instance that is a memory figure rather than a statistic.


## [1.0.2](https://github.com/solisoft/solidb/compare/v1.0.1...v1.0.2) (2026-09-01)

### Features

* **`&&` now works as `AND` in SDBQL.** It used to be a syntax error: the lexer
  read `&` without looking ahead, so `a && b` produced two `Ampersand` tokens
  and the parser died on the second with `Unexpected token in expression:
  Ampersand`. `docs/SDBQL_REFERENCE.md` had documented `AND (&&)` all along, so
  the reference was describing something that had never worked. `&&` is a
  strict alias: same boolean result as `AND`, `null && 5` is `false` exactly as
  `null AND 5` is.
  * Single `&` is untouched and still bitwise — `6 & 3` is `2`, and `a &&& b`
    lexes as `&&` followed by a bitwise `&`.
  * Note the deliberate asymmetry with `||`, which is *not* a strict alias for
    `OR`: `OR` returns a boolean, `||` returns the value of whichever side it
    settles on (`null || "x"` is `"x"`, `null OR "x"` is `true`). The two are
    interchangeable inside a `FILTER`, which only reads truthiness, but not in
    a `RETURN` or `LET`. This is now written down in the reference.
  * `!` was already a working alias for `NOT`; verified, unchanged.
  * Applied to both lexers, `src/sdbql/lexer.rs` and `sdbql-core/src/lexer.rs`.

### Fixes

* **A background column-family drop could crash the process at exit.**
  `PendingCfDrops::spawn_dropper` detached its thread with a bare
  `std::thread::spawn` and kept no handle, so a `drop_cf` still inside RocksDB's
  `PersistRocksDBOptions` raced the static destructors that free the global
  option-type registry. The process died with `SIGSEGV` or
  `std::bad_alloc`/`SIGABRT` *after* reporting every operation successful —
  which is why `rbac_admin_endpoints_tests` aborted at exit with all its tests
  green, and the server had the same race on shutdown. The handles are now kept
  and joined from `StorageEngine`'s `Drop`, by whichever clone is the last one
  alive (`StorageEngine` is `Clone` and its `Drop` runs per clone, so a
  `liveness` token identifies the last one rather than blocking a request path).
  Measured on the same binaries, 60 runs each: `db_authorization_tests` 5/60
  failures → 0/60, `rbac_admin_endpoints_tests` → 0/60.

### Performance

* **CI's test job: ~100 minutes → an expected ~25.** The suite was never the
  cost — CI spent **95m 07s compiling and 18.4s running** 2597 tests. Each of
  the ~96 files in `tests/` links its own binary against all of `src/` plus
  RocksDB, and `[profile.release]`'s `lto = "thin"` re-ran ThinLTO over the
  whole dependency graph once per binary. A new `[profile.ci]` inherits release
  — dependencies stay at `opt-level 3`, so the tests still execute in seconds —
  but drops cross-crate LTO and builds the workspace crates at `opt-level 1`.
  `[profile.release]` is untouched: `build-binaries` and `docker` ship it.
* **The CI cargo cache was frozen and over quota.** `actions/cache` refuses to
  save on a primary-key hit, so `target/` stayed at whatever existed when the
  `Cargo.lock` hash was first seen; and caching `target/` whole put the repo at
  17.34 GB against GitHub's 10 GB limit, so the four job caches evicted each
  other in LRU (clippy's was simply gone) and run times swung between 95 and
  136 minutes depending on which one lost. All four jobs now use
  `Swatinem/rust-cache`, which saves on every key change, keys on rustc version
  and `RUSTFLAGS` too, and keeps only dependency artifacts — with `save-if` so
  that only `main` writes.
* **The five benchmarks in `tests/` no longer build by default.** Every test in
  `benchmark_sort`, `benchmark_perf_fixes`, `sdbql_compare_bench`,
  `sdbql_datetime_bench` and `sdbql_string_bench` is `#[test] #[ignore]`, so
  they were paying a full release link each and never running. They are behind a
  `bench-tests` feature now: `cargo test --profile ci --features bench-tests --
  --ignored`. They stay linted, because `cargo clippy --all-targets
  --all-features` still reaches them.

## [1.0.1](https://github.com/solisoft/solidb/compare/v1.0.0...v1.0.1) (2026-09-01)

### Features

* **`solidb-dump --all-databases`** dumps every database the credentials can
  read into one stream. Every record already carried its own `_database`, so
  the combined dump restores to the right place with no `-d`; what was missing
  was a way to ask for all of them without scripting a loop over
  `GET /_api/databases`. That endpoint filters by permission, so this captures
  exactly the databases the principal may read — `_system` included, with its
  credential collections skipped as everywhere else. Conflicts with `-d` and
  `-c`. A database that cannot be listed aborts the dump rather than leaving a
  silently partial file.
* **`solidb-restore --exclude-collection`** skips collections on the way in.
  Repeatable, comma-separated values accepted, and `*` matches any run of
  characters (`--exclude-collection 'events_*'`). The pattern is matched
  against the collection name *in the dump*, so it still selects the right
  records when `-c` is rewriting the target. Every record type is filtered —
  documents, the collection and index declarations, columnar rows and blob
  chunks — so an excluded collection is not created at all; blob framing stays
  intact because the chunk payload is consumed before the record is dropped.
  Excluded records are counted and named in the summary and do not affect the
  exit status, unlike an unroutable record. A pattern that matches nothing
  warns rather than reporting a clean run. Pairs with `--all-databases`:
  `--exclude-collection '_*'` restores the user data out of a whole-instance
  dump without replaying `_system` bookkeeping into a live server.

### Fixes

* **A database with an `_env` collection could not list its collections.**
  `GET /_api/database/{db}/collection` enumerates column families, so `_env`
  appeared in the listing for any database that had ever had an env var set.
  The credential guard added in 1.0.0 then refused to open it, and the `?` in
  the listing handler turned one unlistable collection into a `403` for the
  whole request — for every principal, admin included. Setting a single env
  var (or opening the admin Env page, which creates the collection) was
  enough to make the database's collection list unreachable. Credential
  collections are now skipped in the listing, keyed off the same
  `PROTECTED_COLLECTIONS` list the guard uses so the two cannot drift.
  `_env` is no longer listed at all — it was never readable through this API,
  and the listing publishes a document count and storage stats per entry —
  and the admin-only `/_api/database/{db}/env` endpoints are unaffected.
* **The driver protocol's `list_collections` hid credential collections too.**
  It returned the raw column-family list, so `_env` appeared over the binary
  protocol while the HTTP listing hid it. Names only — every driver read path
  already refused them — but the two listings disagreed, and listing `_env`
  discloses that a database holds provider keys.

## [1.0.0](https://github.com/solisoft/solidb/compare/v0.34.0...v1.0.0) (2026-08-31)

### Security

* **Native HTTPS termination** with `--tls-cert` / `--tls-key`. Previously
  the only way to serve TLS was a reverse proxy in front; the server now
  terminates TLS itself via rustls (no OpenSSL). In dual-port mode the API
  port serves HTTPS while replication stays as configured. On the
  multiplexed port the listener *sniffs* for a TLS ClientHello and only
  handshakes when one is offered, so HTTPS clients get the tunnel (HTTP and
  the native driver protocol both work inside it) while plaintext peers keep
  connecting — the shipped SDKs' driver protocol and the sync/cluster
  transports do not speak TLS yet, and handshaking unconditionally would cut
  every driver client and every inter-node connection off that port. Set
  `SOLIDB_TLS_REQUIRE=1` to refuse plaintext there anyway (safe only on a
  single node with no native-protocol clients). Both flags are required
  together — one without the other refuses to start rather than silently
  listening in plaintext.
* **Per-client API rate limiting.** Only `/auth/login` was throttled; every
  other endpoint could be hammered without bound. The whole router now has a
  per-client-IP sliding-window limiter (default 600 requests / 60s,
  ~10 req/s sustained — generous enough that normal traffic never trips it)
  answering `429 Too Many Requests` with `Retry-After` before any handler
  work runs. Internal cluster traffic (a valid `X-Cluster-Secret`) and CORS
  preflights are exempt — shard forwarding sends one request per document,
  well above any budget meant for external callers. Client identity follows
  the login limiter's rule: socket peer, unless
  `SOLIDB_TRUST_PROXY_HEADERS=1`; a caller that cannot be identified at all
  is not throttled rather than sharing one bucket with everyone else.
  Configure with `SOLIDB_API_RATE_LIMIT` (0 disables) and
  `SOLIDB_API_RATE_WINDOW_SECS`.
* **Driver queries now have a timeout.** HTTP query execution was capped at
  30s but the binary protocol ran `executor.execute` unbounded on a runtime
  thread — a long query over the driver port could pin it forever. Driver
  `Query` and `Explain` now run on the blocking pool under the same 30s cap,
  gated on the same "is this long-running?" predicate the HTTP handler uses
  so point reads keep running inline. A blocking task cannot be cancelled,
  so a mutation that overruns the timeout still commits: its cached
  collections are dropped both at the timeout and again when the write
  actually lands.
* **Tokens in `?token=` are restricted to WebSocket endpoints.** A JWT in
  the query string leaks into access logs, proxy logs and browser history,
  but browser WebSocket clients cannot send an `Authorization` header, so
  the parameter is still accepted for exactly `/_api/ws/changefeed`,
  `/_api/cluster/status/ws` and `/_api/monitoring/ws`, and refused
  everywhere else (`401`, logged) — including `/_api/livequery/token`, the
  REST endpoint that issues those tokens.
* **Panic-surface cleanup in SDBQL.** The lexer's string/template readers
  took the opening quote from `current_char.unwrap()` — unreachable today,
  but an invariant rather than a check — and now receive the quote char
  explicitly (both copies: `solidb` and `sdbql-core`). `DATE_TRUNC`'s year/
  month/week arms return `ExecutionError` instead of panicking when chrono
  arithmetic hits date bounds.

### Hardening

* **cargo-fuzz targets** for the adversarial-input surfaces:
  `sdbql_parse` (full lexer+parser pipeline), `sdbql_lex` (lexer alone),
  `driver_command_decode` (MessagePack command decoding off the TCP port),
  and `restore_jsonl_line` (the JSONL document parsing `solidb-restore`
  performs). See `fuzz/`; run with `cargo +nightly fuzz run <target>`.

* **Replication TCP fails closed without a keyfile.** The HTTP cluster bus
  already required a secret (0.34.0); the multiplexed sync socket still
  skipped HMAC when no keyfile existed. Unauthenticated replication is now
  refused unless `SOLIDB_ALLOW_UNAUTHENTICATED_SYNC=true` is set for local
  tests. `SOLIDB_REQUIRE_KEYFILE=true` still wins.
* **HTTP listeners default to loopback.** Bind address is `127.0.0.1` unless
  `--host` or `SOLIDB_HOST` says otherwise. Use `0.0.0.0` only when something
  in front of the process terminates TLS.
* **Installing Lua is Admin.** Creating or changing `_scripts` / `_services`
  required only Write, so a collection editor could publish unauthenticated
  `/api/{db}/{service}/…` handlers. Mutating those collections now needs
  Admin. New services default to `require_auth: true`. `solidb.env` secrets
  are injected only for authenticated admin scripts.
* **Cluster control-plane HTTP is Admin.** `remove-node`, `rebalance`,
  sync-log prune/stats, cluster info/status, blob rebalance, and the cluster
  status WebSocket accepted any authenticated principal (including `viewer`).
  They now require Admin.
* **Livequery JWTs are path-restricted on `?token=` as well as Bearer.** The
  query-string branch used to accept a livequery token on any authenticated
  route.
* **JWT roles for real `_admins` users are reloaded on each request**, so a
  revoke does not wait for the 24h token TTL.
* **API keys must declare at least one role.** Empty `roles` no longer
  default to `admin`. The admin UI rejects a blank role list the same way.
* **`/metrics` requires authentication** unless `SOLIDB_METRICS_PUBLIC=1`.
  Present `SOLIDB_METRICS_TOKEN` (`X-Metrics-Token` or `Bearer`) or an admin
  JWT.
* **Physical backups are jailed** under `SOLIDB_BACKUP_ROOT` (or
  `{data_dir}/backups`). `..` and paths outside that root are refused.
* **Webhook URLs are SSRF-checked** (loopback, RFC1918, link-local, metadata
  hosts). Permissive TLS for `*.test` / `*.local` requires
  `SOLIDB_ALLOW_INSECURE_WEBHOOK_TLS=1`.
* **`SOLIDB_DB_AUTHZ_MODE=warn` is ignored** unless
  `SOLIDB_DB_AUTHZ_ALLOW_WARN=1`.
* **`SOLIDB_LUA_FAST_MODE` is ignored** unless
  `SOLIDB_LUA_FAST_MODE_UNSAFE=1` (cross-request Lua state leak).
* Passwords must be at least 12 characters. Lua `crypto.jwt_decode` compares
  signatures in constant time. `crypto.random_bytes` / `random_string` cap at
  64 KiB.
* The admin app no longer falls back to `admin`/`admin`. `enable_trust_proxy`
  is off unless you turn it on.

### Features

* **SDBQL gains set operations, recursive CTEs, `RETURN DISTINCT`, `COLLECT
  … KEEP`, the `NONE` quantifier, and standalone `OFFSET`.** Query blocks can
  be combined with `UNION [ALL]`, `INTERSECT`, and `EXCEPT` (`FOR … RETURN …
  UNION ALL FOR … RETURN …`; either side may be parenthesized; duplicates are
  removed except for `UNION ALL`). Chains follow SQL precedence — `INTERSECT`
  binds tighter than `UNION`/`EXCEPT`, which chain left to right — and rows
  are compared by value, the same equality `UNION()`/`INTERSECTION()` use.
  `WITH RECURSIVE name AS (<anchor> UNION ALL <step>)` iterates the step until
  it stops producing rows — inside the step the CTE name is bound to the rows
  of the *previous* iteration (hierarchies, org charts, transitive closures),
  with safety caps at 1,000 iterations / 1M rows. `RETURN DISTINCT expr`
  deduplicates result rows (first occurrence wins). `COLLECT … INTO g KEEP v1,
  v2` restricts which in-scope variables are stored in the group arrays (an
  unknown name is an error). `NONE(x IN arr SATISFIES cond)` — and the
  function form `NONE(arr, x -> cond)` — is true when no element satisfies.
  `OFFSET n` works alone or as `LIMIT n OFFSET m`. A CTE declared before a set
  operation binds in every operand, and operands, CTE bodies and subqueries
  are all executed as full query blocks — their own `WITH`, pre-`FOR` `LET`s,
  `SORT`/`LIMIT` and nested set operations all apply. Permission checks, query
  caching, cache invalidation, the long-running-query gate and
  `has_mutations()` all see through set-operation operands and CTE bodies.
* **Query-driven auto-indexes (opt-in).** Collections with `autoIndex: true`
  (or unset + `SOLIDB_AUTO_INDEX=1`) create a persistent `_auto_{field}`
  index the first time a `FOR` + `FILTER` equality/range miss would have used
  one. Requires Write or Admin on every query route, and a query with no
  principal at all (internal view refreshes, queue jobs, stream tasks) never
  creates one. Cap 16. Explicit `autoIndex: false` overrides the env var.
  Nested field names keep dots. Null comparisons, `_key`/`_id`/`_rev`,
  sharded collections, SORT, filters a composite index already covers,
  collections above `SOLIDB_AUTO_INDEX_MAX_DOCS` (default 1,000,000, `0`
  disables) and fields no document carries are not auto-indexed.
  `EXPLAIN` reports `auto_index_candidate` without creating.

* **Driver queries carry the session principal.** The binary protocol built
  its query executor without one, so `CURRENT_USER`, `CURRENT_ROLES` and row
  policies were inert over the driver while they applied over HTTP. A row
  policy now filters driver reads too.

* **`--no-lua` / `SOLIDB_NO_LUA=1` skips the Lua VM pool.** Custom scripts,
  `/api/{db}/{service}/…`, and the REPL return 501; a trigger whose action is
  a Lua script fails its job immediately, without retries. Script documents
  can still be stored. Use this on nodes that never run Lua to drop the idle
  RSS of the pre-warmed VMs (at least four states).

* **SDBQL string functions are AQL-shaped and Unicode-correct.** Offsets and
  `LENGTH` on strings are Unicode scalar counts (`BYTE_LENGTH` is UTF-8
  bytes). `FIND_FIRST` / `FIND_LAST` take an optional start/end. `SUBSTRING`
  accepts a negative start. `CONTAINS` can return a character index.
  `LIKE(text, pattern, caseInsensitive?)` is a function as well as an
  operator. Added `REGEX_MATCHES`, `REGEX_SPLIT`, `REPEAT`, `LPAD`/`RPAD`,
  `JOIN`, `MASK`, `WORD_COUNT`, `TRUNCATE_TEXT`, `RANDOM_TOKEN`. `ENCODE_URI`
  percent-encodes UTF-8. Null string arguments propagate as null. Regexes
  go through `safe_regex` and a compile cache. `REPEAT` / pad results are
  capped at 1 MiB.
* **SDBQL array, math, and object functions that the reference already
  listed now exist.** `TAKE`, `DROP`, `CHUNK`, `ZIP`, `CONTAINS` on arrays;
  `MOD`, `CLAMP`, variadic `MIN`/`MAX`; `GET(obj, "a.b", default)`,
  `DEEP_MERGE`, `ENTRIES`, `FROM_ENTRIES`, `JSON_POINTER`. `NTH` accepts a
  negative index. `VAR_SAMP` / `STDDEV_SAMP` divide by `n-1`. `RANGE`
  refuses more than 1M elements. `SHIFT([])` no longer panics.
* **SDBQL gains time-series, sketches, semantic operators, and query HOFs.**
  `MAP`/`FILTER`/`FLAT_MAP`/`GROUP_BY`/`SORT_BY`/`WINDOW_BY` take lambdas
  without requiring `|>`. `<=>` is vector cosine distance or a three-way
  compare; binary `~` is trigram semantic match. Added `DELTA`, `RATE`,
  `FILL`, `RESAMPLE`, `ASOF JOIN … ASOF left, right [BACKWARD|FORWARD|NEAREST]
  TOLERANCE`, `APPROX_COUNT_DISTINCT`, `APPROX_PERCENTILE`, `APPROX_TOP_K`,
  `SKETCH_MERGE`, `MATCH_SEQ`, `SEMANTIC`, `REDACT`. Graph `FOR v, e, p`
  binds a path object; `PRUNE expr` stops expansion; `SHORTEST_PATH … OPTIONS
  { weight: "cost" }` is parsed. `FOR coll SYSTEM_TIME AS OF ts` and
  `SNAPSHOT_DIFF` read versioned history. `CURRENT_USER`/`CAN`/`ROW_POLICY`
  use the request principal. `EMBED`/`EXTRACT` call the configured LLM;
  `CITE`/`GROUNDED` have a lexical fallback.
* **SDBQL graph, search, GeoJSON, and identity helpers.** Weighted
  `SHORTEST_PATH OPTIONS { weight }` uses Dijkstra. Added
  `ALL_SHORTEST_PATHS`, `K_SHORTEST_PATHS`, `K_PATHS`, `GRAPH name` as the
  edge collection, and one-pattern `MATCH (a:coll {_key: k})-[:edge*1..n]->(b)`.
  `SEARCH` is a scored filter; `TOKENS`/`PHRASE`/`BOOST`/`SEARCH_SCORE` are
  in-memory analyzers (not Arango Views). GeoJSON constructors and
  `GEO_CONTAINS`/`GEO_INTERSECTS`/`GEO_IN_RANGE`/`GEO_AREA`.
  `PARSE_IDENTIFIER`/`PARSE_COLLECTION`/`PARSE_KEY`, `UNSET_RECURSIVE`/
  `KEEP_RECURSIVE`, `ZIP_OBJECT`, `DATE_ROUND`, `APPLY`/`CALL`, `MINHASH*`.
  `insert_batch`/`upsert_batch` and transactional write batches record
  version history. `VALID_TIME AS OF` /
  `FROM … TO` filters `valid_from`/`valid_to` fields.
* **Named graphs, search views, document ACL, and index-backed search.**
  `CREATE_GRAPH` / `DROP_GRAPH` / `GRAPH_INFO` store `{vertices, edges}`
  in `_graphs`; `GRAPH name` resolves the first edge collection from that
  catalog (bare edge-collection names still work). `CREATE_VIEW` /
  `DROP_VIEW` register a `type: "search"` alias in `_views` so
  `FOR d IN view` scans the backing collection. `SEARCH_INDEX(coll, field,
  query [, limit])` walks a fulltext index. `CAN("read", doc)` honours
  `owner` / `_owner` and `_acl.{action}` (user, role, or `*`) after the
  collection-level grant. `ZIP(keys, values)` returns an object when the
  first array is all strings. `K_SHORTEST_PATHS` uses Yen’s algorithm.

### Performance

* **SDBQL set and slice helpers no longer walk the array twice.** `UNIQUE`,
  `UNION`, `INTERSECTION`, `MINUS`, and `COUNT_DISTINCT` hash values (seahash,
  collision-checked) instead of `serde_json::to_string` or `Vec::contains`.
  `APPEND` / `UNSHIFT` / `FLATTEN` reserve; `SHIFT` copies the tail instead of
  `remove(0)`. `SORTED` uses `sort_unstable_by`. `KEEP` / `UNSET` copy only
  surviving keys. ASCII `SUBSTRING` slices bytes.

* **SDBQL date functions share one parser and no longer live in two modules.**
  `YYYY-MM-DD` and second-resolution epochs parse. `DATE_ADD` uses calendar
  months (31 Jan + 1 month → 28/29 Feb). Added `DATE_COMPARE`, `DATE_LEAPYEAR`,
  `DATE_MILLISECOND`, `DATE_ISOWEEKYEAR`. `DATE_DIFF` unit is optional.
  `DATE_TRUNC` accepts `week`. `HUMAN_TIME` says `in N minutes` for the future.
  Null date arguments propagate.

* **SDBQL function dispatch is prefix-routed.** `UPPER` no longer walks the
  `DATE_*` / `SQRT` match arms. Phonetic math shadows of `ABS`/`ROUND`/`SQRT`
  are gone. Added AQL-shaped `BIT_AND`/`BIT_OR`/`BIT_XOR`/`BIT_NEGATE`/
  `BIT_SHIFT_*`, `OUTERSECTION`, `IS_DATE`, `IS_KEY`, `GEO_EQUALS`. `COUNT`
  on an object is the key count. `TO_NUMBER(null)` is null. Crypto helpers
  no longer clone the input string.

### Fixes

* **A failed index backfill no longer leaves a half-built index behind.**
  `create_index` persisted the index metadata before writing the entries, so
  an interrupted build (disk full, shutdown) left an index that `get_index`
  reported as ready and that later lookups silently under-returned from. The
  metadata is now rolled back on failure. The backfill also streams the
  column family instead of collecting every decoded document first, and
  `IndexStats.indexed_documents` now counts the documents actually indexed
  rather than the documents scanned.
* **`LENGTH("users")` is no longer a collection count.** If the string
  happened to be a collection name, `LENGTH` returned the document count
  instead of the character length. Use `COLLECTION_COUNT("users")` or
  `LENGTH((FOR d IN users RETURN 1))`.
* Lua string functions no longer live in two modules with different
  semantics (phonetic vs builtins). One evaluator, one set of rules.

### Removed

* **The client-facing job and cron queue is gone.** Application background jobs
  now live in the Soli framework, which claims and runs them in its own process
  on whichever database it is configured for. SolidB no longer needs to schedule
  application work, so the queue it exposed for that purpose is removed:

  * **HTTP** (10 endpoints): `GET/PUT /_api/database/{db}/queues…`,
    `GET /…/queues/{name}/jobs`, `POST /…/queues/{name}/enqueue`,
    `DELETE /…/queues/jobs/{id}`, `POST /…/queues/jobs/{id}/run-now`, and all
    four `/_api/database/{db}/cron` routes.
  * **Driver protocol** (8 commands): `list_queues`, `list_jobs`, `enqueue_job`,
    `cancel_job`, `list_cron_jobs`, `create_cron_job`, `update_cron_job`,
    `delete_cron_job`.
  * **Lua**: the `db:enqueue(queue, script, params, options)` global.
  * **Rust API**: `queue::{CronJob, QueueConfig}`, `QueueWorker::check_cron_jobs`.
  * **Clients**: the jobs and cron sub-clients in the JS, Ruby, PHP, Python, Go,
    and Elixir SDKs, plus the queue command variants in the Rust client.
  * **UI**: the admin Queues and Cron pages, and the CLI TUI Jobs tab. Tab `5`
    is now Cluster.
  * Per-queue pause and concurrency settings (`_queue_config`), which were only
    reachable through the removed endpoint.

  **Triggers keep working.** A trigger still fires by inserting a row into
  `_jobs`, and the worker still claims that row and runs its Lua script or posts
  its signed webhook. That dispatcher, the HMAC signing, and the `Job` /
  `JobStatus` types all remain. Embedding generation and materialized-view
  refresh are untouched.

  **Action required.** Any caller of the endpoints, driver commands, SDK
  sub-clients, or `db:enqueue` above must move to Soli's job engine. Let SolidB's
  queue drain before you upgrade: a `pending` row that no trigger created will
  never run. Stored `_cron_jobs` and `_queue_config` collections become orphan
  data — no code reads them, and **a cron schedule you configured stops
  firing, with no error**. Drop those collections when you are ready.

## [0.34.0](https://github.com/solisoft/solidb/compare/v0.33.0...v0.34.0) (2026-08-05)

Mostly a clustering release. Multi-machine replication could not work at all, and each fault hid the next, so every fix below was found by deploying two real nodes rather than by a test.

**Two changes can stop a node that used to start:** the replication bus now requires a shared secret, and an unroutable `--advertise` address is refused when peers are configured. That is what makes this a minor release rather than a patch — read those two entries before upgrading a running cluster.

### Security

* **Credential collections are no longer readable at `Read`.** `_env` is where SoliDB tells you to put provider API keys, but it was an ordinary collection: any principal with `Read` on the database could dump every key through `GET /_api/database/{db}/env`, the document API, SDBQL (`FOR d IN _env`), or the driver protocol. The same held for `_admins` (argon2 password hashes) and `_api_keys`. These three are now refused by every path that takes a caller-supplied collection name, and the `/env` endpoints require `Admin` on the database — which is what the documentation already promised. Server-side readers (the LLM client, authentication, the Lua `solidb.env` binding) are unaffected. Because credentials are no longer reachable through the query API, `solidb-dump` skips them and says so; capture them with a physical backup (`POST /_api/backup`).
* **The cluster bus fails closed instead of open.** `seal_cluster_message` and `open_cluster_message` signed and verified *when a secret was configured*; with none, both arms fell through and sent unsigned / accepted unverified. So a cluster started without a keyfile had no authentication on its replication bus and looked identical to one that did. The asymmetry was the dangerous part: a node *with* a secret rejects unsigned messages, a node *without* one accepts both, so one misconfigured member silently downgraded what it would take from anyone. A shared secret is now required. **Action required:** a cluster without a keyfile will not start.

### Features

* **`--advertise` sets the address peers should use to reach a node.** Defaults to `--host`. The address a node *listens* on and the address it *advertises* are different questions and only one of them can be guessed; before this flag existed the advertised one was hardcoded to loopback (see the fix below). A hostname is accepted without resolving: resolving here would let a DNS answer decide whether a node may start, and that answer can differ from the one a peer gets.
* **`Command::Query` gains a `cache` flag.** Defaults to true so older clients keep the cached path; set it false to force a real execution, the same way HTTP `/cursor` takes `"cache": false`.
* **`solidb-client` gains `send_command_on(idx, …)` and `authenticate_pool()`.** Needed to authenticate every socket of a pool rather than one (see the fix below), and useful directly when a command must land on a chosen connection.

### Fixes

* **A cluster can replicate across machines.** The advertised replication address was hardcoded to `127.0.0.1`, with no flag and no override, so every node told every peer to reach it at an address each peer reads as itself: a multi-machine cluster could not replicate, structurally, while logging nothing unusual. New `--advertise` flag, defaulting to `--host`. **Action required:** with peers configured, an unroutable value (loopback, `0.0.0.0`, `::`, `localhost`) is now refused at startup and the error names both ways to fix it — a cluster that silently cannot replicate is worse than one that will not start. Loopback is refused only when at least one peer is elsewhere, so two nodes on one host still work. A hostname is accepted without resolving: resolving here would let a DNS answer decide whether a node may start, and that answer can differ from the one a peer gets.
* **A fresh cluster gets an admin account.** `is_joining_cluster` was only "peers is not empty", so in a cluster whose members list each other, every node skipped creating `_admins` — the first one included. Nothing created an admin, nothing errored, and the cluster answered `401` to everything with four INFO lines to explain it. The database cannot distinguish "I am joining an existing cluster" from "we are all starting at once" from a peer list, so the skip is now a WARN naming both ways out: start the first node with no `--peer`, or set `SOLIDB_ADMIN_PASSWORD`.
* **A joining node can ask for the data that predates it.** `_admins` never reached a node that joined, so the peer answered `401` forever and an application could only ever point at the seed. Three faults stacked: the sync worker's command sender was discarded at construction (`let (_tx, rx) = …`), so no `SyncCommand` could ever be sent and `RequestFullSync` was unreachable although defined, handled and implemented; the full-sync framing wrote `[length][bincode]` while its only reader expected `[compressed][length][bincode]`, shifting everything by a byte; and the document batch was `bincode`-encoded from `Vec<serde_json::Value>`, which serialises but can never deserialise, because reading a self-describing type means asking the format what comes next and bincode stores no type information to answer with. Full sync had therefore never once completed. The sender is kept, a joining node asks the seed it joined through for a full sync, and a failure to ask is logged as an error stating exactly what the node will and will not have.
* **A joined member knows the cluster it joined.** `--advertise 10.0.0.1:6746` became `10.0.0.1:6746:6746`, because the port was appended unconditionally to a value that already carried one — and the flag is written both ways in practice, since its help says "the address peers should use" while its default is `--host`, a bare host. That address does not resolve, and the one place it mattered — the seed answering a `JoinRequest` with the peer list — discarded its error. So a joining node was a member from the seed's point of view and knew nobody from its own: `solidb_cluster_healthy_nodes` read 2 on the seed and 1 on the member, with nothing logged on either side. `with_port` now attaches a port only when there is not one, bare IPv6 gets brackets first (appending to `::1` gives `::1:6746`, which parses as a different address entirely and fails only at connect time), and the send logs its failure, naming what the operator is left with.
* **Sync frame decoding is symmetric.** `encode` returned a frame and `decode` took a body, so every caller had to strip a header whose length only lived inside `encode`. Thirteen tests still sliced the old 4-byte header and failed with the same "expected variant index 0 <= i < 27" error the header change was made to fix — which read as a regression in the thing just repaired. Missed because only `--lib` was run. There is now `decode_frame` and `HEADER_LEN`, all fourteen call sites go through them, a short frame and a frame that lies about its length are refused by name rather than by a discriminant error, and a test fails if the constant and the writer ever disagree again.
* **The native driver authenticates its whole pool.** Authentication is per-socket state — the server records it against the connection, not against a client identity or token — but `auth()` sent its handshake through the round-robin `send_command`, so exactly one connection of the pool was authenticated and the rest stayed bare; the next command landed on a bare one and failed with `Authentication required`. Every `pool_size > 1` client was therefore unusable, including the Rust crate's own `benchmark` binary, which is why the TCP transport read as broken rather than merely unmeasured. Adds `send_command_on(idx, …)` and `authenticate_pool()`. With it fixed the benchmark completes for the first time: TCP does 21,606 sequential inserts/s against HTTP's 9,967 (2.2x) and 40,378 reads against 10,646 (3.8x).
* **Document writes over the driver invalidate the query cache.** Document insert/update/delete/bulk over the native protocol left the shared query-result cache serving stale rows to both the driver and the HTTP `/cursor` path. Each mutation now invalidates the collection, matching the HTTP handlers.

### Performance

* **The driver's query handler caches like `/cursor` does.** It called `parse` directly where the HTTP `/cursor` handler uses the prepared-statement cache, and it had no result cache at all, so it executed every query for real while HTTP replayed a memoized result. That measured as the driver being 16% *slower* on a 50-row projection, with server CPU per request nearly doubling (127 -> 216us) — which reads as "the binary protocol is bad at queries" and was really "one handler caches and the other does not". It now uses `parse_if_needed`, memoizes read-only results per (database, query, bind vars), and invalidates the collections a mutating query touches, on the same terms as the HTTP handler. Same cell after: 50,106 req/s against HTTP's 35,726, on 49-50us of server CPU against 106-123us — 1.40x the throughput on under half the CPU, because MessagePack costs less to produce than the HTTP JSON response and there is no HTTP framing to build.

## [0.33.0](https://github.com/solisoft/solidb/compare/v0.32.2...v0.33.0) (2026-07-27)

### Features

* **Columnar collections are a first-class SDBQL data source.** A columnar collection was reachable from SDBQL through exactly one shape, `FOR x IN c COLLECT AGGREGATE … RETURN …`. Adding a `FILTER`, `SORT`, `LIMIT` or join made the same collection report *CollectionNotFound*, because columnar rows are not stored under the document prefix the scanner walks. `FOR` now resolves columnar collections directly, so filters, sorts, limits, joins and subqueries work, and columnar data can be combined with documents, edges and vector search in one query. Filter and projection pushdown are not implemented yet, so a selective filter over a large collection still reads less through the `/columnar/…/query` endpoint.
* **Physical backup.** `POST /_api/backup` (admin) takes a RocksDB checkpoint of the whole instance: near-instant, hard-linked, point-in-time consistent across collections. Restore by pointing a server at the directory. `solidb-dump` remains the per-database and cross-version path. Because a checkpoint hard-links SSTs on the same filesystem, copy it elsewhere — it is not protection against losing that volume.

### Fixes

* **Columnar aggregates returned wrong results rather than errors.** Four bugs: the `RETURN` clause was ignored (`RETURN {sum: total}` came back as `{"total": …}`, and a scalar `RETURN` came back as an object); grouped queries dropped every aggregate after the first, so `AGGREGATE lo = MIN(…), hi = MAX(…)` lost `hi`; group and aggregate columns were reported under internal storage names instead of the `COLLECT` variables; and string group keys were double-encoded, so `a` came back as `"\"a\""`. The last of these was fixed in the storage layer, so the `/columnar` REST endpoint benefits too.
* **Index definitions replicate.** An index created on one node existed only on that node: there was no `CreateIndex` operation in the sync protocol, so peers ran unindexed scans and never enforced a unique index they did not have. Worst, the TTL sweep skips a collection with no TTL index, so documents expired on the node where the index was created and lived forever on every other node. Index creation is also shard-aware now — previously an index on a sharded collection was built on the logical collection, which holds no documents.
* **Offline sync writes are persisted.** `/_api/sync/push` validated the session, appended to the replication log and returned `{"accepted": N}` without ever writing to storage, so pushed documents never existed. `/_api/sync/conflicts` and `/_api/sync/resolve` now return `501` instead of an empty list and `{"success": true}`; real conflict detection needs per-document version vectors, which storage does not yet carry.
* **Blob under-replication repair.** Blob chunks replicate inline at upload and are absent from both recovery paths, so a chunk missed while a peer was down stayed missing. The rebalance worker now scans for under-replicated chunks and re-pushes them.
* **Safer `solidb-dump` / `solidb-restore`.** Columnar indexes go through `columnar_index` records rather than schema `indexed` flags, document dumps are enveloped to avoid field collisions, index-list failures surface, and a partial dump or restore exits non-zero. Adds `--scheme`, `--overwrite` (import `mode=upsert`), path encoding and auth validation.
* **Dropping a database requires typing its name.** The admin confirm modal takes a name confirmation, and the endpoint rejects the delete unless it matches.

### Performance

* **Small responses are no longer gzip-ed.** Compression had a 32-byte floor, so every client sending `Accept-Encoding` paid a fixed per-response CPU cost on replies too small to benefit: a ~300B cursor response spent ~216µs compressing (65% of the request envelope) and a 65B health response *grew* to 88B. The 1-doc query path measured 42.7k req/s with compression against 103k req/s without. The floor is now 4 KB, overridable with `SOLIDB_GZIP_MIN_BYTES` if you are bandwidth-bound rather than CPU-bound.

### Build

* **Supply-chain checks in CI.** `cargo-deny` (advisories, bans, licenses, sources), clippy widened to `--all-targets --all-features`, and a declared MSRV verified by a job that builds it. Two advisories were fixed by version bumps rather than ignored.
* **Releases are gated on the docs site matching `Cargo.toml`.** `scripts/check_docs_sync.sh` runs as the `docs-sync` CI job and at the top of `scripts/release.sh`, so a stale version pill or a missing changelog section stops the release before a tag or a crates.io publish exists.

## [0.32.2](https://github.com/solisoft/solidb/compare/v0.32.1...v0.32.2) (2026-07-25)

### Fixes

* **Blob collections are restorable from a whole-database dump.** `solidb-dump` streamed the server's single-collection `/export` output verbatim, and those records name neither the database nor the collection, so `solidb-restore` aborted on the first one with *"No collection specified in doc or args"*. The dump now injects the routing metadata, copying binary payloads through byte for byte.
* **Collections larger than 10,000 documents are no longer silently truncated.** The dump asked for `batchSize` 1,000,000 and read only the first response, but the server clamps it to 10,000. It now follows the cursor, and warns when the number of documents dumped differs from the reported count.
* **Empty collections survive a round trip.** A collection with no documents and no indexes wrote nothing at all and disappeared from the dump. Every collection now leads with a declaration record carrying its type, so edge, blob and timeseries collections come back as themselves rather than plain document collections.
* **Columnar collections are dumped and restored.** They live behind their own API and were skipped entirely, while their backing `_columnar_*` column family was exported as a phantom empty document collection.
* Restore skips an unroutable record instead of aborting the whole run, creates the database once per run rather than once per collection, and treats "already exists" index clashes as success.

## [0.32.1](https://github.com/solisoft/solidb/compare/v0.32.0...v0.32.1) (2026-07-24)

### Fixes

* **`solidb-restore` now honours `--database` / `--collection`.** Both flags are documented as overrides, but the target was resolved as "name embedded in the dump, falling back to the flag". Since every dump emits `_database` and `_collection`, the fallback was unreachable and both flags were silently ignored — `solidb-restore -d staging --input prod.dump` restored into `prod`. The CLI flag now wins and the dump's name is the fallback, across document, blob-chunk and index records. **Operators who relied on the old behaviour to restore a dump back into its original database can simply omit `-d`.**

## [0.32.0](https://github.com/solisoft/solidb/compare/v0.31.0...v0.32.0) (2026-07-19)

### Features

* **Windows x86_64 builds**: releases now include `solidb-windows-amd64.zip` (`solidb.exe`, `solidb-dump.exe`, `solidb-restore.exe`) alongside the Linux and macOS tarballs. Two caveats for operators:
  * `--daemon` is Unix-only and exits with an error. Run SoliDB in a console, or wrap it with a service manager such as NSSM or `sc.exe`.
  * The generated `.admin_password` file is written with default ACLs rather than the owner-only permissions used on Unix. Restrict the data directory yourself on a shared machine.
  * FUSE (`solidb-fuse`) remains Unix-only, and `solidb update` still does not support Windows.

### Changes

* **TLS moves from OpenSSL to rustls.** OpenSSL is no longer in the dependency graph at all. The Docker image no longer installs `libssl3`, and building from source no longer needs `libssl-dev` (or Perl/NASM on Windows). Certificate validation now uses the platform trust store via rustls rather than OpenSSL; `ca-certificates` is still required in the container image.

### Performance

* Vector-index persistence is throttled to at most once per 5s behind a dirty flag, with a shutdown flush, instead of re-serializing the whole index (all vectors + HNSW graph) after every write batch. Bulk loads into embedding-bearing collections are no longer O(batches × index size).
* Document updates that leave every vector index's embedding unchanged skip the delete+reinsert entirely, so an incremental sync that only rewrites metadata pays no HNSW churn.

## [0.26.4](https://github.com/solisoft/solidb/compare/v0.26.3...v0.26.4) (2026-06-11)

### Security

* **Per-database authorization on the data plane**: every `/_api/database/{db}/...` route (documents, queries, collections, indexes, blobs, scripts, queues, triggers, ...) now enforces the caller's role permissions and API-key database scope. Previously only authentication was checked — any valid token could read/write any database. Notes for operators:
  * Set `SOLIDB_DB_AUTHZ_MODE=warn` for a dry-run release: denials are logged on the `audit` target but allowed. Default is `enforce`.
  * Pre-RBAC users and API keys hold the `admin` role (existing migration) and are unaffected. JWTs minted without roles are now denied on data routes.
  * `truncate`, `DROP collection` and the db-scoped Lua REPL now require `Admin` (the built-in `editor` role loses them).
  * Database-scoped API keys can no longer perform global operations (create/delete database, role and key management) even when they carry the `admin` role, and `GET /_api/databases` only lists databases the caller can read.
  * Mutating SDBQL/SQL submitted through read endpoints (`/cursor`, `/sql`, transactional `/query`) is upgraded to a Write permission check after parsing.
  * WebSocket changefeed subscriptions and live queries check Read permission on the target database; livequery tokens inherit the requester's identity/roles.
  * The binary driver protocol resolves roles at auth time and checks every command against its `database` field (connections are no longer trusted to stay on the database they authenticated against).
* **Cluster control messages are HMAC-signed**: when a keyfile is configured, membership/heartbeat/rebalance messages on the multiplexed port are wrapped in a signed envelope (timestamp + nonce + HMAC-SHA256) and unsigned or stale messages are rejected. The read is also size-capped (1 MB) and time-bounded. **All nodes in a cluster must upgrade together.**
* **Keyfile required for clusters**: a node configured with `--peers` now refuses to start without a keyfile instead of silently running an unauthenticated cluster.
* **Replication slowloris protection**: sync protocol reads are time-bounded (partial headers/payloads can no longer hold connections open forever).
* **Lua resource limits**: scripts run under a 64 MB memory cap (`SOLIDB_LUA_MEMORY_LIMIT_MB`) and a 30 s execution deadline (`SOLIDB_LUA_TIMEOUT_SECS`).

### Fixes

* **Truncate now replicates from every path**: the binary driver and materialized-view refresh previously truncated locally without writing to the replication log, leaving replicas with stale data. Driver document/collection/database mutations and driver queries now also feed the replication log (they previously never replicated at all).
* Stream processors clear their buffered window when the source collection is truncated instead of aggregating over deleted data.
* Materialized-view refresh reports the number of removed documents in mutation stats.

### Performance

* JOINs scan the joined collection once instead of once per left-side row.
* Graph traversals without a `_from`/`_to` index build an in-memory adjacency map (one scan) instead of rescanning the edge collection for every visited vertex; `ANY`-direction traversals use the indexes when both exist.
* Shortest-path queries scan the edge collection once per search instead of once per visited vertex.
* Per-username role lookups are cached (30 s TTL, invalidated on grant/revoke) instead of scanning `_user_roles` on every request; driver API-key auth uses the shared O(1) key cache instead of scanning `_api_keys`.
* Cursor store is capped at 10 000 concurrent cursors (oldest evicted); shard healing backs off exponentially (up to 5 min) when peers are unreachable; the sync worker warns loudly when an unacknowledged peer blocks log pruning.

## [0.21.2](https://github.com/solisoft/solidb/compare/v0.21.1...v0.21.2) (2026-03-24)

### Improvements

* **SORT performance & correctness**: Stable sort preserves original order of equal elements; Index optimization for SORT without LIMIT; Pre-evaluation of sort expressions with proper error handling; Lexicographic array comparison element-by-element

## [0.9.0](https://github.com/solisoft/solidb/compare/v0.8.0...v0.9.0) (2026-02-08)


### Features

* add default service parameter to script creation in tests ([ed6de25](https://github.com/solisoft/solidb/commit/ed6de254b73b9701360d1ce9745132086b29a749))

## [0.8.0](https://github.com/solisoft/solidb/compare/v0.7.0...v0.8.0) (2026-02-08)


### Features

* ACID on node ([865d293](https://github.com/solisoft/solidb/commit/865d293de170c05289e92cf7e4aa89c8164ad86c))
* add authorization error handling to DbError enum; enhance authentication logic in server components and update expression attributes for improved consistency ([32f4246](https://github.com/solisoft/solidb/commit/32f42464175b4c7654ab2d79973b6a4270b056eb))
* add Belote game routes and remove Sdbql timing wrapper ([102670c](https://github.com/solisoft/solidb/commit/102670cdbc03f3c49785dc05b2cbe811fd542127))
* add columnar index management features including creation, deletion, and listing routes; enhance server handlers and UI components for improved user interaction ([41836b9](https://github.com/solisoft/solidb/commit/41836b9a990be9d8e0daab23fbf702ef2b6cdc90))
* add comprehensive unit tests for error handling, cluster configuration, health monitoring, node management, and storage operations; enhance documentation for new features and improve test coverage across various modules ([439b91d](https://github.com/solisoft/solidb/commit/439b91d757822f4aab00d9748cbbbc9e53f7b28d))
* add curve25519 key agreement function and ISO 8601 timestamp formatting ([90bcb21](https://github.com/solisoft/solidb/commit/90bcb219a701a469c0bcf95854401c6a06545a20))
* add debug panel to talks components for enhanced local stream diagnostics; implement helper functions for PWA mode and video reference status checks ([c08b6ea](https://github.com/solisoft/solidb/commit/c08b6ea80d2a430439c8870f11feff23176651db))
* add debug_data action to CalendarController for user event insights and update routes to include debug endpoint ([05018b6](https://github.com/solisoft/solidb/commit/05018b6a92cd6f68fad05eaefa0b72fe218c6966))
* add delete schema functionality to collections controller; update routes and enhance modal UI with documentation link ([e34b6ee](https://github.com/solisoft/solidb/commit/e34b6eed49a2cd0c27dcf0908595f608c85e86a5))
* add detailed documentation for Array Spread Operator, including usage examples and notes on behavior with missing fields ([6852c4b](https://github.com/solisoft/solidb/commit/6852c4b8fa7ac2bd5ce59d99da15301be2c8c2b0))
* add Docker cluster setup instructions to documentation, including commands for generating shared keyfile and starting a 3-node cluster ([8362a17](https://github.com/solisoft/solidb/commit/8362a17335cdc8c989a2f3d40a0061004e6f0a43))
* add environment variable management routes and handlers; enhance dashboard and sidebar UI components for improved user access to environment variables ([fadde74](https://github.com/solisoft/solidb/commit/fadde747b4b39f96f2a405a4b4a3762b36767deb))
* add function call handling for LEFT and RIGHT tokens in parser ([3cfd0fa](https://github.com/solisoft/solidb/commit/3cfd0fa14332a9c39ff6d0c2a2bcd1e9937b8d15))
* add functionality to manage channel members in talks component; implement add and remove member features with corresponding API routes; enhance UI for member management in talks-header and talks-sidebar components ([392d928](https://github.com/solisoft/solidb/commit/392d928190792c5ff734698cd4f859a131329fb2))
* add fuzzy string matching and similarity functions to enhance expression evaluation, introducing FuzzyEqual operator and new SIMILARITY and FUZZY_MATCH functions for improved text comparison capabilities ([163c50c](https://github.com/solisoft/solidb/commit/163c50c07899c6c31f262f69dc585364b735bdef))
* add Gemini provider support and update documentation ([77fbf91](https://github.com/solisoft/solidb/commit/77fbf91473e43dec77d88d9bbb4f17286dec4f27))
* add generic content generation endpoint and enhance LLMClient ([17c04b3](https://github.com/solisoft/solidb/commit/17c04b3000f5e1aa624908073f948585c7aa9f55))
* add HNSW support for vector search, including ef_search parameter for improved recall and performance, and update documentation to reflect new functionality ([cbab1db](https://github.com/solisoft/solidb/commit/cbab1dbacd89501d1a7447b2d0d35b675f1f5c30))
* add HUMAN_TIME and HIGHLIGHT functions to query executor ([00e1722](https://github.com/solisoft/solidb/commit/00e1722d009987b1214d5acc06c63dd851a96ba5))
* add JOIN operations support to SDBQL ([eca4a23](https://github.com/solisoft/solidb/commit/eca4a2361fe9127b1e398eebb566172d9a6f2b4e))
* add JSON functions documentation to scripting-utils ([440b6d2](https://github.com/solisoft/solidb/commit/440b6d286caca870bfea88eea642b6d9820c003e))
* add JSON schema validation support in collections; enhance error handling for schema validation and compilation; update API routes for schema management in collections ([3300b92](https://github.com/solisoft/solidb/commit/3300b9299c3a95263625f3efb70fa6e4bbfd7e1e))
* add missing migration ([0325075](https://github.com/solisoft/solidb/commit/03250757b3a1916a34c3cf9d050333df00cf24b7))
* add mutation statistics to query execution responses; enhance dashboard and scripts manager components for improved user feedback and interaction ([29c7755](https://github.com/solisoft/solidb/commit/29c7755eebb0f5ed4a7fea8a25f7bb0b75ecf513))
* add nanoid and ulid packages, enhance documentation with new functions ([30fba35](https://github.com/solisoft/solidb/commit/30fba351339662c17bf5c88b83b0c87b53e7efaf))
* add NotIn operator and enhance query parsing ([02feeac](https://github.com/solisoft/solidb/commit/02feeac4265bd0e80432bc2118c6ae7db4f60165))
* add null coalescing operator (??) to enhance expression evaluation, including parsing, execution, and comprehensive test coverage for various scenarios ([2a37af7](https://github.com/solisoft/solidb/commit/2a37af7509b630931f35db734a868d281de70dae))
* add OperationNotSupported error variant; refactor collection index creation to accept vector; enhance connection pool to flush stream before authentication; update tests to include ScriptStats in router creation ([4acf8da](https://github.com/solisoft/solidb/commit/4acf8daa4f77f7f8db41f5e4607e0048699e020c))
* add quoting functionality in talks components, allowing users to reply to messages with context, and implement sound activation popup for enhanced user experience during conversations ([906e89a](https://github.com/solisoft/solidb/commit/906e89a49c2cafe752a529a055cbfb42e476edaf))
* add Rust Internals section to documentation ([7e1b17b](https://github.com/solisoft/solidb/commit/7e1b17ba53b40fb2055df4a09f9c6bd4f22095a9))
* add Satisfies token and quantifier expression parsing ([efb5e41](https://github.com/solisoft/solidb/commit/efb5e4170c869a7f545f175e31a2d647cec90596))
* add security audit workflow to CI pipeline ([7ba8be2](https://github.com/solisoft/solidb/commit/7ba8be25895a9823030edda466560711855c4c81))
* add session authentication middleware and require it for CRUD routes to enhance security ([d970bc9](https://github.com/solisoft/solidb/commit/d970bc957899c3421963810bfb233b693fa4050c))
* add slides functionality to docs controller and corresponding route; enhance security check in show method and improve code structure ([5df4fff](https://github.com/solisoft/solidb/commit/5df4fffbb63cbca718472dc3750aa6e747c24b88))
* add solidb scripts section to documentation ([191fc91](https://github.com/solisoft/solidb/commit/191fc911a14ef7e5f1cc5ba60dd14e9bb902ebb3))
* add SQL compatibility section to API documentation, detailing SQL query execution, translation to SDBQL, and supported SQL features ([89aefb1](https://github.com/solisoft/solidb/commit/89aefb1211fede4a826c4a39558e9494b5047f48))
* add support for Materialized Views in SDBQL ([6a1dabc](https://github.com/solisoft/solidb/commit/6a1dabcccdbc02e82acd9d9926fa20ee6e36ed93))
* add support for URL-encoded query parameters in authentication middleware; integrate serde_urlencoded for improved token handling and validation ([9bc1013](https://github.com/solisoft/solidb/commit/9bc101386514c27d74bb481ff78eb3cd9a3000ad))
* add transaction document operations and enhance routing in server, improving database interaction capabilities ([98e463f](https://github.com/solisoft/solidb/commit/98e463f9b34882ea120e3b1f1b022d204f1503ac))
* add UUIDs methods ([052414a](https://github.com/solisoft/solidb/commit/052414af368988014fea438d612118d39be77884))
* add vector quantization support for memory-efficient storage, including new API endpoints for quantization and dequantization, and update documentation to reflect changes in vector index statistics ([23909c5](https://github.com/solisoft/solidb/commit/23909c5b796f405d8aaf51df5666b996ada9a813))
* add year view event fetching and Belote game routes ([299da26](https://github.com/solisoft/solidb/commit/299da267020fc2ae5e9b36f0a861269497a76139))
* allow channel creation ([1a4fb6d](https://github.com/solisoft/solidb/commit/1a4fb6d2d5951c0500c71e097e0286aa7a2e4a03))
* allow screen sharing for audio calls ([617fa65](https://github.com/solisoft/solidb/commit/617fa65c319ecd6dab14cb0e18eb5b09c7442bbe))
* array functions ([6c9de72](https://github.com/solisoft/solidb/commit/6c9de72a490ef3c781c3cfb2c8a42ab892d60d38))
* async index build ([8dcd1a6](https://github.com/solisoft/solidb/commit/8dcd1a68100990fd7cc76556d00f9d194307bb3d))
* async index build ([b6d4d23](https://github.com/solisoft/solidb/commit/b6d4d2305b973efed26bd339e17b631461f19ad1))
* audio + video + screensharing ([a5ffc54](https://github.com/solisoft/solidb/commit/a5ffc5418de138aeaedc9e3608c4ea33a1d58f91))
* authentification ([91c49ea](https://github.com/solisoft/solidb/commit/91c49ea239cd498a3d5f9905329be44571d4300d))
* basic graph features ([bdfc790](https://github.com/solisoft/solidb/commit/bdfc79059dd5d20c0d53ce13efb6e449a7a7f963))
* better documentation + daemon mode ([935b5dd](https://github.com/solisoft/solidb/commit/935b5dde43263b2d3d2501232fb214eee0a8c9e9))
* blob storage ([e0103ae](https://github.com/solisoft/solidb/commit/e0103ae4d7d0e3781ddfed481765876e2dcb97d4))
* check sharding / replicas ([624a915](https://github.com/solisoft/solidb/commit/624a915cf926098327aaead7e4b628c49fb57f53))
* daemonize solidb-fuse ([4c08c42](https://github.com/solisoft/solidb/commit/4c08c42b43a4c4bf1409c81a19c0e0fbae99ba0a))
* debug solidb driver ([edead6f](https://github.com/solisoft/solidb/commit/edead6fee3029a597f11b89ccbeaa7fc817a706d))
* debug solidb driver ([6d1b804](https://github.com/solisoft/solidb/commit/6d1b80492c153580533c0eaec861da3eb1013a98))
* debug solidb driver ([d9b5521](https://github.com/solisoft/solidb/commit/d9b55212a9176f1705f35f3605431bdce2c1e671))
* debug solidb driver ([bbf4c52](https://github.com/solisoft/solidb/commit/bbf4c521d66aecc2fde305092a1d80d904cbcd71))
* debug solidb driver ([9267210](https://github.com/solisoft/solidb/commit/92672108a68610637c740ccab534739bb5f2664d))
* debug solidb driver ([8cacfc8](https://github.com/solisoft/solidb/commit/8cacfc8d53217c161cb5af594d29f76605348d37))
* debug solidb driver ([6fb9216](https://github.com/solisoft/solidb/commit/6fb92169fe7bf23dce9cfd45be8477b675acc08d))
* debug solidb driver ([39feffb](https://github.com/solisoft/solidb/commit/39feffbdf8642032fceaebb65aaf5b1c8fabd5fb))
* debug solidb driver ([ca465ed](https://github.com/solisoft/solidb/commit/ca465ed6549a77efa976379e87eba8b30b1891ca))
* debug time spent ([3ee7c1a](https://github.com/solisoft/solidb/commit/3ee7c1a8d40a2f5cbe0a8a5ad6a552591f8d868d))
* draft fuse ([66712cb](https://github.com/solisoft/solidb/commit/66712cbcdc5ec450549cb9b0009c048e5cd259d2))
* enhance API key management with roles and scoped databases; update handlers and UI components for improved user experience and functionality ([d6fb853](https://github.com/solisoft/solidb/commit/d6fb8538518a3e32e32a85dec8cf08ed5051a9ab))
* enhance batch insert functionality with atomic operations and validation ([30c3dc1](https://github.com/solisoft/solidb/commit/30c3dc1dce819e85bacc52a8d2ef8ef3342b4b3c))
* enhance benchmark scripts to include read operations for all clients, updating output formatting to display insert and read performance separately ([7ad4a35](https://github.com/solisoft/solidb/commit/7ad4a35c2565156086a54df6f50de73c3510d206))
* enhance benchmark setup by adding installation steps for Go, Python, Node.js, Ruby, and PHP dependencies, improving client readiness for testing ([2fad96c](https://github.com/solisoft/solidb/commit/2fad96cbc73de7a5fa5251cba4d4b093ed57251d))
* enhance billing dashboard with monthly revenue statistics ([690b362](https://github.com/solisoft/solidb/commit/690b362179780180b83d8d90eeb05331043201c0))
* enhance CLI capabilities and optimize Lua scripting engine ([33263d2](https://github.com/solisoft/solidb/commit/33263d290b1f17941afc17da451ca581e430f14a))
* enhance client libraries and documentation for multiple languages ([d43bd92](https://github.com/solisoft/solidb/commit/d43bd92a9a6a78cd6bc5d0f73b7d72a5df7f4608))
* enhance cursor store functionality and update API response handling ([338e746](https://github.com/solisoft/solidb/commit/338e7469fbe8c489d41c2e412c47a409ce996724))
* enhance documentation with advanced syntax and functions ([2ef421c](https://github.com/solisoft/solidb/commit/2ef421cdc6d6a8ff9ec6736a6f9efd56322378ca))
* enhance documentation with Columnar Index and related features ([f92a869](https://github.com/solisoft/solidb/commit/f92a8690ca097e57475ec61c3673043265401ec3))
* enhance documentation with Docker deployment instructions and API reference updates, including new sections for Docker usage and detailed API categories in the sidebar ([66c5eaa](https://github.com/solisoft/solidb/commit/66c5eaa40e74c3eb92b27a2691b04ee4129ca876))
* enhance error handling and response formatting in DbError ([f9ae94d](https://github.com/solisoft/solidb/commit/f9ae94d94c2ea8974dce312f27838c1e4daa1692))
* enhance graph data fetching and UI for multiple vertex collections ([12780c1](https://github.com/solisoft/solidb/commit/12780c170f3c218787fa8ecd3036957e4d37aaa4))
* enhance local stream attachment logic in talks components; implement retry mechanism for video element readiness and improve logging for debugging ([9764465](https://github.com/solisoft/solidb/commit/9764465aa31528c07a63fe1ce7a9947a481b1ff5))
* enhance login flow by adding redirect support and refactoring authentication checks across controllers ([696a363](https://github.com/solisoft/solidb/commit/696a363c0ba76cfb627bf2ddc6cd5da5a4f631a7))
* enhance Mailbox and Calendar features by adding sidebar calendar invites, improving user experience with updated UI elements and new query methods ([c20c918](https://github.com/solisoft/solidb/commit/c20c9181bc5cb2f996d6ae5c3ba871e52ae74132))
* enhance mobile responsiveness and UI components in talks application; refactor sidebar, header, and message handling for improved user experience; update API integration for WebSocket connections ([3c333b3](https://github.com/solisoft/solidb/commit/3c333b3b4b8841c849091d851366a66fb30cd11e))
* enhance PWA support by updating service worker for improved caching and offline functionality; modify manifest for better app integration and icon support; update favicon and meta tags in index.html.etlua ([c5b3d71](https://github.com/solisoft/solidb/commit/c5b3d7143959ada2812f9524e1b58fc625ecd8c8))
* enhance query execution by adding range expression handling in FOR clauses; improve index usage checks for collection-based queries ([bff29d0](https://github.com/solisoft/solidb/commit/bff29d04687693e34e2622a551a94a4264bbb48d))
* enhance SDBQL with logical OR operator and autocomplete functionality ([101a122](https://github.com/solisoft/solidb/commit/101a122b0e743d7bcbfe65b984cdffd20cab4c21))
* enhance service worker communication and version management; update service worker to version 19 and implement version messaging for improved client synchronization ([8448af8](https://github.com/solisoft/solidb/commit/8448af8a5187b1b8a2817cb72d8b39e3824403b1))
* enhance session management and UI interactions ([bed3810](https://github.com/solisoft/solidb/commit/bed3810717ef7305cd38f4599ec9443d1f43fd75))
* enhance solidb-client with HTTP transport support and version bump to 0.7.0 ([7dca436](https://github.com/solisoft/solidb/commit/7dca436d04fe5326844edf254b030f7c31787323))
* enhance SQL parser and translator to support qualified column names and aggregate aliases with LET variables; improve HAVING clause handling and update documentation for clarity ([9e07a10](https://github.com/solisoft/solidb/commit/9e07a10dc3b0aaca1d1ee0f184220a7fb613c2b2))
* enhance stream documentation with windowing details and comparison diagram ([1a66165](https://github.com/solisoft/solidb/commit/1a66165a7250de28d0aa331b67b6395764e9bcf5))
* enhance user experience by adding avatar color generation and initials extraction for senders in talks components, improving visual representation and personalization in conversations ([4a64ab5](https://github.com/solisoft/solidb/commit/4a64ab56f23f5278f4992223e67e21241d3d426d))
* enhance user experience by improving the getInitials function for sender names and refactoring participant avatar color generation in talks components ([0c89084](https://github.com/solisoft/solidb/commit/0c8908405050baa9cae5656653d860a5c1d1bc3f))
* enhance vector index management by adding upsert and delete handling, including batch persistence and rebuilding of indexes ([4c0540d](https://github.com/solisoft/solidb/commit/4c0540d428b20320597728951249f536766de0cc))
* enhance WebSocket connection in system monitor and talks components by implementing token retrieval and improving call handling features ([193fd53](https://github.com/solisoft/solidb/commit/193fd539c2f9738a99c074049dbdc5a625d8a3f3))
* enhance z-index management in talks components for improved overlay visibility; update CSS classes for better responsiveness and consistency across mobile and desktop views ([1237fc6](https://github.com/solisoft/solidb/commit/1237fc65d3fd4fd76dc94bc4821b87ddd30467a6))
* expand benchmark suite to include Ruby, PHP, and Elixir tests, enhancing performance evaluation across multiple languages ([6f8517e](https://github.com/solisoft/solidb/commit/6f8517e778bca0c2d2ad0eae9e7fcda0b91ca3cc))
* expose crypto & time functions to lua ([d7fb473](https://github.com/solisoft/solidb/commit/d7fb473bba4b4759a5210e75e9a358f35648d0bb))
* fix compilation warnings ([fd92654](https://github.com/solisoft/solidb/commit/fd92654fff1f4d4a0df608b7cb7cfea74fa4b851))
* fix query button ([700483c](https://github.com/solisoft/solidb/commit/700483c4bbaf948282fb6cbd669249108b03c885))
* fix scheme wss:: ([8098860](https://github.com/solisoft/solidb/commit/80988608d5154e3ded3892bfb4aef884996d72ce))
* github actions ([0f95637](https://github.com/solisoft/solidb/commit/0f956377eb70142430f2892582de04952cff1036))
* global livefeed ([8afe5a2](https://github.com/solisoft/solidb/commit/8afe5a21f7e538428ca139793f468f9803e09536))
* if a daemon is running when launching a new server it kill the old first ([32fd9bc](https://github.com/solisoft/solidb/commit/32fd9bce1ae0c70b3b7062ab834d9595bfd92860))
* implement agent update functionality with new API endpoint; enhance agent structure with default values and improve error handling in agent management ([b3492d7](https://github.com/solisoft/solidb/commit/b3492d79ad55b0de4c9b190c47c21bbb0e7ebe6e))
* implement Blob Rebalance Worker for cluster mode ([4aeb41e](https://github.com/solisoft/solidb/commit/4aeb41ec0f8fbc080d70921a6ae9a0b85b607c68))
* implement block management features in PagesController and Page model ([8f3628d](https://github.com/solisoft/solidb/commit/8f3628d58c09037f248de08827b638ab15c8733a))
* implement Bloom filter index support in columnar storage; enhance collection and query handling for probabilistic membership testing, update UI components for index management ([f39c424](https://github.com/solisoft/solidb/commit/f39c42497cbbfe5e22961e08e0786767c7a0a70d))
* implement bulk operations for SDBQL ([fa9f86e](https://github.com/solisoft/solidb/commit/fa9f86e0ac98a9fc18f1cb6708603ece8a5b4b51))
* implement collaborator management for repositories, including adding, removing, and searching collaborators, along with admin user management routes ([19fde3a](https://github.com/solisoft/solidb/commit/19fde3a94d2513ee66c90cf3bbab286abff39d29))
* implement document pruning and enhance cuckoo filter methods ([36481cf](https://github.com/solisoft/solidb/commit/36481cfdfff075d4c6b45604f016db49f8833a34))
* implement drag-and-drop file upload functionality in talks components, enhancing user experience by allowing users to easily add files to conversations ([ba732b0](https://github.com/solisoft/solidb/commit/ba732b054afd64975ddb80e3e49e85ecb2411e82))
* implement hybrid search functionality combining vector similarity and fulltext search, including new API endpoint and documentation updates ([cfce308](https://github.com/solisoft/solidb/commit/cfce30822e91464191d5ed0a8d8407bffa5365ac))
* implement interactive Lua REPL with session management; add REPL endpoint, session persistence, and UI components for enhanced user experience ([a52df3e](https://github.com/solisoft/solidb/commit/a52df3e9d0dce682a5e47d6bef97318a3a799dae))
* implement LiveQuery token management in talks components; enhance WebSocket connection logic to utilize cached tokens and improve error handling for token fetching ([cf0a855](https://github.com/solisoft/solidb/commit/cf0a855c8a4708c5f5a1df2c33c41cb0e6cac10c))
* implement lua solidb.fetch method ([aea69a1](https://github.com/solisoft/solidb/commit/aea69a1b4d85fe0d0ef6c93f31d3a1772c982c2b))
* implement lua solidb.fetch method ([d4e1523](https://github.com/solisoft/solidb/commit/d4e1523b46f3192a7b433a2b8d6a521bf22444b8))
* implement natural language feedback and history tracking ([0607961](https://github.com/solisoft/solidb/commit/060796194a42dbf616038ed2358725f6c763b010))
* implement new caching mechanism to improve data retrieval performance; update related tests and documentation for clarity and coverage ([1cd0dea](https://github.com/solisoft/solidb/commit/1cd0dea1610580ff39e6f58b3c8fb8862d3bfed3))
* implement offline-first synchronization for Rust client ([01fd3ee](https://github.com/solisoft/solidb/commit/01fd3eea5e8ab904ac3bfdc3a832eed355031ef8))
* implement optional field access and window functions, enhancing expression evaluation with new CASE expressions and comprehensive support for SQL-style operations ([69accee](https://github.com/solisoft/solidb/commit/69acceede6e509cc6d4bb79da5490d69305adf19))
* implement pagination support for query results; enhance performance and user experience in data retrieval and display ([5ccd046](https://github.com/solisoft/solidb/commit/5ccd046c7e33ab7ef9ac2489314f89c2dfc42c69))
* implement pinning functionality for remote peers in talks-calls component, allowing users to pin up to two peers for better visibility during calls ([8e0b8e7](https://github.com/solisoft/solidb/commit/8e0b8e7761e6d1d2ba3b5c3cbedd80e6de887c92))
* implement pipeline operator and null coalescing functionality, enhancing expression evaluation with support for lambda expressions and improved parsing, along with comprehensive test coverage ([4913493](https://github.com/solisoft/solidb/commit/4913493be8760c0df7642efa8a88c74525a32df3))
* implement role and user management API handlers; enhance authorization service with permission checks and update routes for user role assignments ([06dcbab](https://github.com/solisoft/solidb/commit/06dcbab2f1d562e0f65cd79344f950983cea6c0a))
* implement schema validation caching in Collection ([7f1285c](https://github.com/solisoft/solidb/commit/7f1285cde6ec26aa245405355f41a4897202eb5c))
* implement serialization for DbError and enhance DriverHandler functionality ([c3a4a02](https://github.com/solisoft/solidb/commit/c3a4a0248b974b1354b42391fcb7a5fe66f6e10f))
* implement single-line comment support in lexer, enhancing SDBQL syntax with new comment handling and comprehensive test cases for various comment scenarios ([977504f](https://github.com/solisoft/solidb/commit/977504f2c699d6fa6801ebb450fdd168af1d95c3))
* implement slow query logging and natural language query support ([cfe8d9f](https://github.com/solisoft/solidb/commit/cfe8d9f6f381c42e521d799ebd556d5086ca9ba3))
* implement sorting for remote peers in talks components; enhance performance by caching sorted peers and updating expression attributes for consistency ([2a559cc](https://github.com/solisoft/solidb/commit/2a559ccf8e3b72b8a716e9e2596ba8657ab77f9c))
* implement threaded messaging functionality in talks components, allowing users to open, reply to, and manage message threads, enhancing conversation organization and user experience ([ab5f96c](https://github.com/solisoft/solidb/commit/ab5f96cb26f3b9b7fdf01c2362b26bffae48c094))
* implement TIME_BUCKET function for timestamp bucketing and add prune_collection API for managing time-series collections; enhance UI components for time-series collection support ([597133c](https://github.com/solisoft/solidb/commit/597133c4693c10c6208084cc040dfa061511221c))
* implement triggers functionality and associated routes ([110a4b0](https://github.com/solisoft/solidb/commit/110a4b083f08553fd45baedbe3562db02f487e3c))
* implement TTL expiry index and enhance atomic document/index operations ([31ddfd8](https://github.com/solisoft/solidb/commit/31ddfd8cdcbf322f349bc0a78509afd1985cb7f9))
* implement UPSERT functionality in query language; add support for bitwise operators and enhance parser and lexer for new syntax; update documentation and examples for clarity ([7975df8](https://github.com/solisoft/solidb/commit/7975df82ef3208c44a8b6bfa4a15f151ba40c50e))
* implement vector index functionality, including creation, listing, deletion, and searching of vector indexes, along with support for various distance metrics and vector normalization functions ([128c26f](https://github.com/solisoft/solidb/commit/128c26ff770f82ba4ac297a5049c9d94ecdc8d89))
* implement window close hangup and improve participant management in talks components, ensuring proper cleanup of peer connections and enhancing user experience during calls ([9683ee7](https://github.com/solisoft/solidb/commit/9683ee7219cab2d73ea8667fa08a22f1535915ec))
* improve bulk import on single node ([900c04f](https://github.com/solisoft/solidb/commit/900c04fbbb649f8ee97367c3994a10e627c713a5))
* improve cluster stats ([cfb9c34](https://github.com/solisoft/solidb/commit/cfb9c343c332fc57ca216031627fdd7012ff739e))
* improve endpoints for transatcions ([55f55fd](https://github.com/solisoft/solidb/commit/55f55fdec08052c738f8a08aaf2f6b26702aa413))
* improve import/export feature ([ffe427a](https://github.com/solisoft/solidb/commit/ffe427aa8dff9d030a85aec23c171e8a24518d8f))
* improve import/export feature ([7da543c](https://github.com/solisoft/solidb/commit/7da543c2c0292a52bd873a8dfad287b54bd34c3d))
* improve index performances ([db7699f](https://github.com/solisoft/solidb/commit/db7699f55627d64a9194963dc38369caf217cecc))
* improve security issues ([61140fe](https://github.com/solisoft/solidb/commit/61140fe816f6387d6910873101519b385a8739ce))
* improve security issues ([69ff125](https://github.com/solisoft/solidb/commit/69ff12566af2b3d1e87958ff2a642c264195be92))
* improve streams documentation ([715b291](https://github.com/solisoft/solidb/commit/715b291ac2643478abab29da0b81e93e5b66d769))
* improve talks login ([d246adc](https://github.com/solisoft/solidb/commit/d246adcf0f3d01fe66bda7092c5a1f1ba3625844))
* improve talks login page ([26b858f](https://github.com/solisoft/solidb/commit/26b858f1ecfca1e2187882d9a04454800ade5acb))
* improve talks login page ([ee7c6d3](https://github.com/solisoft/solidb/commit/ee7c6d3547e4b9913a583cd9b006b432466ccaab))
* improve talks login page ([d446c1c](https://github.com/solisoft/solidb/commit/d446c1c7d41a28bbf72cbe6f4f4b4852ba13849c))
* improve tooling page ([eb11ec6](https://github.com/solisoft/solidb/commit/eb11ec623383ca7f6b0d7c148e871b8262d17fcd))
* improve tooling page ([c85ab0e](https://github.com/solisoft/solidb/commit/c85ab0ebf7d3cc690e1f2f2de3cc80c6a6bb0f96))
* improve tooling page ([a8c238c](https://github.com/solisoft/solidb/commit/a8c238c2f4b33a1a1f983c3cb61cfa96cbff89b3))
* improve UI ([9b38660](https://github.com/solisoft/solidb/commit/9b3866059e2a2cacbec58a6c3abd91918cbfac8f))
* integrate channel management for WebSocket support ([b73d4e1](https://github.com/solisoft/solidb/commit/b73d4e1657d3948db7bc1c39ecd7d61b16f010d3))
* integrate MessagePack support for improved performance and add native driver protocol for enhanced data handling ([2420e06](https://github.com/solisoft/solidb/commit/2420e065e3026d3a45f2e439e1c0bc3726e7cdd1))
* integrate Prometheus metrics support, adding a new metrics endpoint and updating documentation to include available metrics and configuration examples ([24457bc](https://github.com/solisoft/solidb/commit/24457bc624ed5c6f16f36b03d6d65e81d6d9a081))
* integrate sdbql-core for local query execution in Rust client ([3774480](https://github.com/solisoft/solidb/commit/3774480183dd9d792c8617e60e6b1a5c24f8313b))
* integrate stream management into scripting engine and enhance documentation ([053ee99](https://github.com/solisoft/solidb/commit/053ee9945fa67b12001de4ca7d30781bafedd2b9))
* introduce AI features including contribution management routes, AI client integration in multiple languages, and enhancements to the dashboard for improved user interaction with AI capabilities ([67d5a4e](https://github.com/solisoft/solidb/commit/67d5a4e1cbb91596445b3f543793609cc0a8d251))
* introduce ArraySpreadAccess for enhanced array handling, allowing extraction of fields from all elements and supporting nested access in queries ([f7f60f9](https://github.com/solisoft/solidb/commit/f7f60f918a22ffd9188ed1e14b2319fcb54c0f31))
* introduce columnar storage support with new routes and UI components; enhance collection management and documentation for improved user experience ([ada38f3](https://github.com/solisoft/solidb/commit/ada38f3de8132bfb48ae5c8b4b3eb7e00d7b212f))
* introduce phonetic algorithms for enhanced name matching ([0e99675](https://github.com/solisoft/solidb/commit/0e99675181c820841f3b1227a86b95b40875ebd7))
* introduce service management and enhance script handling ([706b8ac](https://github.com/solisoft/solidb/commit/706b8ac685b31e5b3232e2f44464c9073034da7d))
* introduce stream processing capabilities with CREATE STREAM and WINDOW clauses, enhancing query syntax and execution in SDBQL ([705b282](https://github.com/solisoft/solidb/commit/705b2825bd09a8a19f0c62b1ed51f085b5e4e665))
* introduce template strings for cleaner string interpolation, enhancing expression evaluation with new syntax for dynamic content generation ([b24d76e](https://github.com/solisoft/solidb/commit/b24d76eab26a7c6e4335f17b49c155b3031215c8))
* keyfile auth for cluster ([22868d5](https://github.com/solisoft/solidb/commit/22868d57f607412d9679d705e273dd7007e8e354))
* keyfile auth for cluster ([9651ac1](https://github.com/solisoft/solidb/commit/9651ac1fc6ee8302ad6c675604b7f047bc98f334))
* lazy initialize AudioContext in talks components to optimize performance; update expression attributes for improved consistency across components ([83d5ec8](https://github.com/solisoft/solidb/commit/83d5ec81b8ef26e6dcca160ff59fa33acfa4e2b4))
* licence ([5995422](https://github.com/solisoft/solidb/commit/5995422fd77434ccec0cc5b484f23ad66275e5fb))
* live feed view update ([0002463](https://github.com/solisoft/solidb/commit/0002463eeff8e224bb7ea27fced233cde1ad8d44))
* live query ([f1ebcea](https://github.com/solisoft/solidb/commit/f1ebceaf7ef527c5cb5208213404abee762e104f))
* live query: enhance WebSocket connection by implementing token retrieval via authenticatedFetch ([b8ddffd](https://github.com/solisoft/solidb/commit/b8ddffdd99b936308de00dd56e53739124d9d30a))
* lua scripting ([f44d76d](https://github.com/solisoft/solidb/commit/f44d76d98d949434b80e181ea0c28c8629b8b189))
* lua scripts supports transactions ([0f82a3f](https://github.com/solisoft/solidb/commit/0f82a3fb1fc5c2546a449419c98c25dd5bc5ead9))
* master&lt;-&gt;master cluster mode ([dfb7a5c](https://github.com/solisoft/solidb/commit/dfb7a5c0315ca3482a36bee2c468cfedc988ea89))
* ms precision for jobs ([5f7099d](https://github.com/solisoft/solidb/commit/5f7099ddd670c6b8090b314ae57e534ed24f6606))
* multi-database ([1d062bc](https://github.com/solisoft/solidb/commit/1d062bc275a62e2ddc1c50590843fdd02e1e6649))
* multi-upload ([cd60b53](https://github.com/solisoft/solidb/commit/cd60b537204054d60dc268852704a3421bc870fb))
* new AQL methods ([ec64455](https://github.com/solisoft/solidb/commit/ec64455a7a8500df2a6d4a8bd75414ad1bdcb3df))
* new coumound indexes + doc ([d6e86fa](https://github.com/solisoft/solidb/commit/d6e86fa54c9691df4d19f12efa81ebe49a24748c))
* new coumound indexes + doc ([5aece57](https://github.com/solisoft/solidb/commit/5aece57227f90de48fb53cb55975b12d0812f942))
* optimize bulk import ([5049e12](https://github.com/solisoft/solidb/commit/5049e12c966192dc8ea3d1821ef5ba2bbb32ad99))
* queues & jobs ([a58024e](https://github.com/solisoft/solidb/commit/a58024eef07ca4b9c211ea643ec85611ff51086c))
* refactor benchmark suite to use async operations with SoliDBClient, enhancing performance and database interaction ([da1ffa8](https://github.com/solisoft/solidb/commit/da1ffa82c47fded23e02711747c58b7a1dc33719))
* refactor CRUD data editing forms to unify field and relation handling, enhancing structure management and user experience ([ed792ed](https://github.com/solisoft/solidb/commit/ed792ed33ed2887e06550af71d465b7c2e0cf98e))
* refactor MailboxController to improve message handling and user experience, including enhanced folder management and message operations ([62eea34](https://github.com/solisoft/solidb/commit/62eea3494f1c4526f05714f7f9d8e442c6f46951))
* refactor talks components to utilize getLocalStream method for local stream management; update expression attributes for improved consistency and functionality across components ([c7ddd48](https://github.com/solisoft/solidb/commit/c7ddd4898ff052ed324c12a3d8a0bcebf50f7f5d))
* refactoring cluster replication & sharding ([1f5e41a](https://github.com/solisoft/solidb/commit/1f5e41aac956d0fc245158f849b61e15d9f2d9f4))
* remove col:all() ([0b22e03](https://github.com/solisoft/solidb/commit/0b22e0326b67fb04ad5498809459e792249984e2))
* remove debug messages ([cc8aaa2](https://github.com/solisoft/solidb/commit/cc8aaa23cb4ed5c310b09627b0bd4040617a931e))
* remove debug panel and helper functions from talks components; update expression attributes for improved consistency and functionality across components ([24a6d39](https://github.com/solisoft/solidb/commit/24a6d394b3d5724a5f7a5894f4945c60c089cc58))
* remove debug time spent ([ea5b0ed](https://github.com/solisoft/solidb/commit/ea5b0ed752ab9aa3097b4d19799c804521cdf58d))
* remove deprecated components from the app, including cluster dashboard, cluster table, and various modals, to streamline the codebase and improve maintainability ([137cd72](https://github.com/solisoft/solidb/commit/137cd728c46ea868da66d740dc0c4813cf5a6846))
* remove deprecated files and components from the project; streamline codebase for improved maintainability and performance ([4199d84](https://github.com/solisoft/solidb/commit/4199d849dd0bec997e2ec630bb26314162da62c9))
* remove useless icons in editor toolbar ([e784159](https://github.com/solisoft/solidb/commit/e7841593f8e157566e1d93bd0cee886f2b5326fc))
* REPL, VSCODE plugin, improvement for query editor ([55348e7](https://github.com/solisoft/solidb/commit/55348e7cf5c1e0a4164beb2378fc2b28b6660982))
* request notification permission for DMs and mentions in talks components; enhance user experience by delaying permission request to avoid overwhelming users ([93e657c](https://github.com/solisoft/solidb/commit/93e657ca3085f08df51b73ecad6f28400890befd))
* scripting: add logging functionality for Lua scripts with a dedicated logs view in the UI ([a6eea75](https://github.com/solisoft/solidb/commit/a6eea75a718b3b94fe8ceeadf5c132bcaeee4835))
* scripting: implement runtime statistics tracking for scripts and WebSocket connections ([b511ce4](https://github.com/solisoft/solidb/commit/b511ce41fe009fcdeac3c930fca906539e3aa490))
* sdbql: add modulus and exponentiation binary operators with error handling for non-numeric values ([984b505](https://github.com/solisoft/solidb/commit/984b5051b4a1d27c2f62e6297f1f3133e6328c39))
* sdbql: add new binary operators (LIKE, REGEX) and hashing functions (MD5, SHA256) with enhanced query capabilities ([19957d5](https://github.com/solisoft/solidb/commit/19957d5e0e6516b82f898fa37508bc677923ee20))
* sharding + replicas ([8d11232](https://github.com/solisoft/solidb/commit/8d1123248b5905e5e83e6dde23777db8a27e0848))
* solidb client for luaonbeans ([a9f4db4](https://github.com/solisoft/solidb/commit/a9f4db464558a58442546b7413df6de9873c3e54))
* solidb-fuse doc ([9feedcb](https://github.com/solisoft/solidb/commit/9feedcbba7955667370c90d9c466c062329a3269))
* sort optimizations ([57a6b50](https://github.com/solisoft/solidb/commit/57a6b50fd3e45bd9e0b4a0c8c278a496540c2e4f))
* string.regex* new methods ([763ad6b](https://github.com/solisoft/solidb/commit/763ad6b6e75b65ab9a28f0b4b2eefec436a406b9))
* talks : update status ([94cf51f](https://github.com/solisoft/solidb/commit/94cf51f921907c147ceeadcb9d564d21ecffef6d))
* talks basics ([b7a3c56](https://github.com/solisoft/solidb/commit/b7a3c56a8857a44b5d21c54994d0376d3e935aa5))
* talks: add favourites + keep WS opened ([ed908d2](https://github.com/solisoft/solidb/commit/ed908d2abc4aceadc183a51ff6291fdccd0daa6c))
* talks: enhance environment variable retrieval for DB_HOST with fallback support ([188acf4](https://github.com/solisoft/solidb/commit/188acf48b66d56df87b6c7efaa2bfff4f5c786ed))
* talks: fix db_host ([4d43330](https://github.com/solisoft/solidb/commit/4d4333014fab852a5f94505379d22b6f401281b9))
* talks: implement search functionality with fulltext indexing ([7295d97](https://github.com/solisoft/solidb/commit/7295d97e446b45edadda1e36e56caf9e8999f925))
* talks: improve UI ([05098d9](https://github.com/solisoft/solidb/commit/05098d9c42ce7ca6d97e7388bb0414fd15e73d57))
* talks: make frontend code more modular ([7eb0260](https://github.com/solisoft/solidb/commit/7eb02607119dd48eafa17540f8f4afc69f1fcfa9))
* talks: make frontend code more modular ([1453728](https://github.com/solisoft/solidb/commit/1453728d81ab03f41d8d5696dfafb554f89bcf54))
* talks: make frontend code more modular ([a460aae](https://github.com/solisoft/solidb/commit/a460aae13c396c64a15ffbd6eec81582c333623f))
* talks: private rooms ([a2ccf5f](https://github.com/solisoft/solidb/commit/a2ccf5fd7bea04998724581dfddad12547d142aa))
* talks: update service worker ([ffe1b91](https://github.com/solisoft/solidb/commit/ffe1b911f2e1eb2174214843394e1d4cbbd0eade))
* ternary + typename methods ([5634f60](https://github.com/solisoft/solidb/commit/5634f60f304da3f908bf5d63f82eb92b1310e8b1))
* update benchmark script to increase worker count to 16 for improved parallel testing, add Rust and JS benchmarks, and adjust output formatting for clarity ([f474050](https://github.com/solisoft/solidb/commit/f47405030086661efdde1b9aa717c4a62a5f28b4))
* update benchmark scripts to use environment variables for configuration, add parallel benchmarking for Python and Go, and enhance error handling across all client implementations ([d50f578](https://github.com/solisoft/solidb/commit/d50f5781c489d51099d06f825e168a05d2607d95))
* update dependencies and enhance CLI with TUI support ([8a4602b](https://github.com/solisoft/solidb/commit/8a4602bee8f5ccda37f8c03d02483746b49b88e4))
* update dependencies and improve CI configuration ([678404a](https://github.com/solisoft/solidb/commit/678404a738934d90baf707cf1f7f799c242320c3))
* update dependencies and improve index handling with batch operations ([83b8842](https://github.com/solisoft/solidb/commit/83b88420f8b9812f1dc58764b6864f76db79806c))
* update documentation with new API reference for common endpoints and JWT/API key authentication details, and add Kubernetes deployment instructions for single-node and cluster setups ([3f81b9f](https://github.com/solisoft/solidb/commit/3f81b9fb2818a69e0393e84f86a574b04fd4c5b2))
* update number of shards per collection ([d579941](https://github.com/solisoft/solidb/commit/d57994183bd22cc2751a5586e3091d6a87255f76))
* update SDBQL documentation and add Gemini option to dashboard ([ba7537d](https://github.com/solisoft/solidb/commit/ba7537debabe4a23b2c4c7f4bd2f7764bd805742))
* update service worker to version 13; simplify fetch event handling and improve caching strategy for static assets ([84d464e](https://github.com/solisoft/solidb/commit/84d464e63b38dd20433e0adf2ebe8f6ccee2c309))
* update service worker to version 9 for improved caching strategy; ensure responses are cloned before caching to prevent potential issues ([8b370bc](https://github.com/solisoft/solidb/commit/8b370bcd265a5bac3df81f18c72064b5cd3619b8))
* update solidb and solidb-client versions, add API key authentication ([4aa9184](https://github.com/solisoft/solidb/commit/4aa91849e359a2087e2b0fe100edf3c42a9519bf))
* update talks components to include local stream support and enhance expression attributes for improved functionality; increment service worker version for better caching strategy ([df826b1](https://github.com/solisoft/solidb/commit/df826b1066ddba47e7da24b20b75933dbbc9e68c))
* use websockets to refresh cluster status ([4ab6f81](https://github.com/solisoft/solidb/commit/4ab6f815d422b4596ab381602664b76be7d05be7))
* websocket support for live changefeeds ([99f8ca0](https://github.com/solisoft/solidb/commit/99f8ca0e7823c4a85fb7ad14aea9bd14ffa98285))


### Bug Fixes

* adjust positioning in talks-calls component for better UI alignment; update architecture documentation with new metrics and built-in functions reference; enhance CSS properties for consistency ([47888f1](https://github.com/solisoft/solidb/commit/47888f166cb081cc0c650bdaf154acbebba1fced))
* admin UI duplicated buttons ([f2f9382](https://github.com/solisoft/solidb/commit/f2f9382d1c823c77a3a5b9abe7e1ab1974076873))
* cargo fmt ([08a1ca6](https://github.com/solisoft/solidb/commit/08a1ca6067df2572b2be0524ec11bb369196376a))
* cli tools needs auth ([68f0e9e](https://github.com/solisoft/solidb/commit/68f0e9e40150a927198b9ca4b4eb5e813d9851eb))
* cli tools needs auth - test ([f607290](https://github.com/solisoft/solidb/commit/f607290a450a0b8afb5ff3c5a899d3ec47733d80))
* cluster sync ([53c46d4](https://github.com/solisoft/solidb/commit/53c46d4007ceda541c01caf7d5d22e3e5ee98fe5))
* clustering check integrity ([42fd7a7](https://github.com/solisoft/solidb/commit/42fd7a7615257f57b441c1729011b116b7501db9))
* clustering check integrity &gt; DEBUG ([7c99195](https://github.com/solisoft/solidb/commit/7c9919588411b8fe8a1a5dafff790eb14bb3cd21))
* clustering check integrity &gt; DEBUG ([07da359](https://github.com/solisoft/solidb/commit/07da359e96a820262792e7709ecce58e4c1276c6))
* clustering check integrity &gt; DEBUG ([904675d](https://github.com/solisoft/solidb/commit/904675dff6da594875baf88ea1509db6addc3941))
* clustering issue ([2b78d4d](https://github.com/solisoft/solidb/commit/2b78d4d6ffbdee47353156fa0c00baf1040f9d95))
* clustering sync &gt; DEBUG ([66f4af4](https://github.com/solisoft/solidb/commit/66f4af448fb5b2c523a7b15fb547951940c6919a))
* clustering sync &gt; DEBUG ([2e117d5](https://github.com/solisoft/solidb/commit/2e117d5a48f26469972f4105254ba5bd3390cfb9))
* clustering sync &gt; DEBUG ([4b41b11](https://github.com/solisoft/solidb/commit/4b41b117aa0ef613b91301ffd3dcfcd1f3531c41))
* conflict ([513783d](https://github.com/solisoft/solidb/commit/513783d4232fb0681205d8e396fd6ff7bfdda337))
* display ms spent on jobs ([6af6921](https://github.com/solisoft/solidb/commit/6af692188f0ee749efa7cc1dc9e723afcf150c96))
* failing specs ([8a7efa9](https://github.com/solisoft/solidb/commit/8a7efa9e34ec0498007321c85a39ea03a498bcdb))
* fix cpu usage ([a479d0f](https://github.com/solisoft/solidb/commit/a479d0f5255e300ddf327bce31609aa967c0030c))
* fix cpu usage + indexes missing bearer ([4600753](https://github.com/solisoft/solidb/commit/4600753fb30cb57ad1ec11de12dd5f4cede30956))
* fuse is optional ([4362ea3](https://github.com/solisoft/solidb/commit/4362ea3c05e15ea1b9673ecab88dc5eccb4a1e01))
* fuse more verbose ([02a957b](https://github.com/solisoft/solidb/commit/02a957b2c64c926e586ac7a37cdd4655756ca1a6))
* fuse more verbose ([0d017c5](https://github.com/solisoft/solidb/commit/0d017c5a7a29972631f14f3ca1c88586c1a09d54))
* github actions (failing for now) ([8864e29](https://github.com/solisoft/solidb/commit/8864e29d86e4cb0ca3e58a4d126b6db51b2c7e52))
* improve sharding performances ([bb50871](https://github.com/solisoft/solidb/commit/bb5087191fdbe9f2145bd2d6d8717a2df36d1789))
* layouts.css ([50d929a](https://github.com/solisoft/solidb/commit/50d929a96e4579e95b50b36f5ded5a45a8fc4a02))
* readme install section ([b010db7](https://github.com/solisoft/solidb/commit/b010db7d1dabc62bffa58c90303fedfc1714f911))
* remove orphaned .db lines causing syntax error ([a31ff2e](https://github.com/solisoft/solidb/commit/a31ff2ebb0056a7cf7402e3162e6abdb6d4f2afb))
* restore csv file ([00b9d3b](https://github.com/solisoft/solidb/commit/00b9d3b3f84e0f615ed069559d22e95ef68a72ac))
* rmove custom jwt support ([f1df794](https://github.com/solisoft/solidb/commit/f1df794eea5a258eae1b81c6bc49058db3f1da23))
* sharding &gt; better compute node managnement ([a8567d7](https://github.com/solisoft/solidb/commit/a8567d72e89cbf2bab46f176c8af92dcf3ca577c))
* sharding issue debug ([81fd9f4](https://github.com/solisoft/solidb/commit/81fd9f449a8d4897227650e065cb399c4835a32b))
* sharding issue debug ([8370689](https://github.com/solisoft/solidb/commit/837068937933498ae8467ecef08a8887b608874b))
* sharding issue debug ([1196812](https://github.com/solisoft/solidb/commit/1196812b4200ad7b6e9383b09e4f9bd3226fdd90))
* sharding issue debug ([5534041](https://github.com/solisoft/solidb/commit/55340414f8125f1b5c370fd0266a2e8eab6ff538))
* sharding issue debug ([0fb1f12](https://github.com/solisoft/solidb/commit/0fb1f127e1829a8693c82dc4ef1ffaef2e6ce7d3))
* sharding issue debug ([26cb749](https://github.com/solisoft/solidb/commit/26cb74928452b7263b0af5c3b927582ef1c1e0b5))
* sharding issue debug ([ca93f87](https://github.com/solisoft/solidb/commit/ca93f8731bb88cf478056dc64582d033f64c5cb1))
* sorting improvement ([39edfa4](https://github.com/solisoft/solidb/commit/39edfa400dbc96c4275a3666105938f79a9f370e))
* sorting improvement ([da947f4](https://github.com/solisoft/solidb/commit/da947f453579acea3fc17198fc4bfe3c625b932e))
* sorting improvement - step 3 ([9928c63](https://github.com/solisoft/solidb/commit/9928c63db9d925008d3989a5b6ccca5f352dad5b))
* streamline slow queries table rendering and time classification ([5cc8fa9](https://github.com/solisoft/solidb/commit/5cc8fa9f1b512a3c14f678aad6328d91fa23e18e))
* talks: group calls (beta) + emoji reactions fixes ([65de1de](https://github.com/solisoft/solidb/commit/65de1dec25c13e9e314510a6d71c28b2192d4e76))
* toml file ([8dba11a](https://github.com/solisoft/solidb/commit/8dba11a389ef69127834da9bf60a1e6f9026b10c))
* unmount folder properly ([ca0b77c](https://github.com/solisoft/solidb/commit/ca0b77c2561bb4beda75f426baf97dea7457e847))
* update benchmark client configurations to use port 9998 and change authentication credentials for improved security and consistency across clients ([09cf4a3](https://github.com/solisoft/solidb/commit/09cf4a36d282f08f039a9f0cbe9aced6e78d906d))
* update benchmark results for all clients in clients.etlua to reflect new performance metrics, ensuring accuracy in sequential, read, and parallel operations ([9765063](https://github.com/solisoft/solidb/commit/9765063de3629c503f5ccdd49f33babaf00fe80a))
* update billing queries to handle missing owner_key ([2570aac](https://github.com/solisoft/solidb/commit/2570aac7ca8690fc6c64e190fd8de64d5f5fc05d))
* update CORS ([5bcfb45](https://github.com/solisoft/solidb/commit/5bcfb45c77e1818a4cad3e9adf0b95bc7ddc6f9f))
* update header styling in documentation layout for improved visibility and aesthetics ([bfd904e](https://github.com/solisoft/solidb/commit/bfd904effa32799e6e98946635afe9ef65784ef8))
* update prune command handling in DriverHandler ([4884a26](https://github.com/solisoft/solidb/commit/4884a2683fe8a98b19b9819d4608e23dad1ee57d))
* work on clustering issus ... wip ([f9b60f4](https://github.com/solisoft/solidb/commit/f9b60f4c879bddc9ecf9b839c0d2b13e19969cca))
