# Vector search, preliminary run (2026-11-18, card #979)

Not the published protocol: no containers for SoliDB or LanceDB, and the host
also runs a production SoliDB. Kept to show where each engine stood; the
published run and its blog post come after cards #983 and #984.

- rocky-build2. Servers (SoliDB, pgvector in a podman container) on cores
  0-3 + SMT, 16 GB for the container; the harness on 8-15 + SMT. LanceDB is a
  library in the client's process: `vec_lance.py`, pinned to 0-3 + SMT.
- 300,000 vectors of 128 dimensions (100 clusters, normalised), 500 queries,
  top-10, cosine; HNSW M=16, ef_construction=200 everywhere (LanceDB:
  IVF_HNSW_SQ, default partitions and nprobes). Medians of 3 runs of 10 s.
- SoliDB: `main` at `1269a727` (the HNSW and filtered-search fixes), started
  from its binary. pgvector: PostgreSQL 17, image
  `pgvector/pgvector@sha256:0a547a6067fdea4eec4cfb0887c4ddae2e91b1932cecc071541c14ee318312ce`.
  LanceDB 0.41.0 from PyPI.
- For about a minute during SoliDB's index build, a LanceDB smoke test ran on
  the same host, unpinned: SoliDB's build time may be a little high.
- LanceDB's searches go through Python: at 8 threads its throughput reflects
  the client as much as the engine.

| | SoliDB | pgvector | LanceDB |
|---|---|---|---|
| index build | 651.7 s | 24.2 s | 8.7 s |
| memory | 1.55 GB RSS | 568 MB (container) | in the client |
| plain, ef 40, 1 client | recall 0.876, 1,557/s | 0.875, 2,168/s | 0.832, 829/s |
| plain, ef 200, 1 client | 0.998, 560/s | 0.990, 890/s | 0.949, 737/s |
| plain, ef 40, 8 clients | 10,358/s | 10,957/s | 1,428/s |
| filtered, ef 40, 1 client | 0.999, 297/s | 0.957, 1,039/s | 0.961, 295/s |
| filtered, ef 40, 8 clients | 1,635/s | 5,700/s | 1,018/s |
