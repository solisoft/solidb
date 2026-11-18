# Vector search, 2026-11-18 (card #979; the blog post's figures)

- rocky-build2. pgvector in its podman container (4 cores 0-3 + SMT, 16 GB,
  image `pgvector/pgvector@sha256:0a547a6067fdea4eec4cfb0887c4ddae2e91b1932cecc071541c14ee318312ce`,
  PostgreSQL 17.12). SoliDB and LanceDB in a systemd scope with the same
  limits (`AllowedCPUs=0-3,16-19`, `MemoryMax=16G`, no swap), not a
  container. The harness on cores 8-15 + SMT. The host also runs a
  production SoliDB.
- SoliDB: `main` at `6308c887` (after 2.14.0), solistore. LanceDB 0.41.0 from
  Python (`vec_lance.py`), IVF_HNSW_SQ.
- 1,000,000 vectors of 128 dimensions (100 clusters, normalised), 500
  queries, top-10, cosine; HNSW M=16, ef_construction=200; medians of 3 runs
  of 10 s at each of ef 40/100/200 and 1/8 clients.
- `solidb.json` / `.memory`: the build with card #984 (a filtered search
  follows the filter in the graph's walk, `ef` counting the documents
  that pass), run alone afterwards on the same host and limits. These are
  the post's figures. `solidb-6308c887.json` is the run beside the two
  others, before #984; its memory reading was wrong (it held the `sudo`
  process that started SoliDB, fixed in `vec.sh`), so it is not kept.
  `pgvector.memory` is the container's.
- `solidb-986.json` / `.memory`: after cards #985 and #986 (an index
  created by streaming the documents; each vector held once, in the graph;
  the index saved in pieces), same settings: speed unchanged within noise,
  RSS 4.35 GB at the end instead of 5.3. `solidb-986-cache64.memory`: a
  short run with `SOLIDB_ARGS="--block-cache 64MB"`, 1.83 GB (2.63 before
  #986), what the index and the server hold without the storage cache.

