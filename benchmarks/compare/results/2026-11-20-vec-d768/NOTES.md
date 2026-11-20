# Vector search at 1M x 768: f32, scalar, PQ (card #1007)

`vec.sh` with `DIM=768`, everything else as 2026-11-19-vec-quant: 1,000,000
vectors in 100 clusters, 500 queries, k 10, ef 40/100/200, 1 and 8 clients,
3 runs of 10 s, the server on 8 cores with `MemoryMax=16G`, rocky-build2,
2026-11-20. Server memory is the RSS after the run, in GiB. `summary.txt`
has every row.

- `f32/`: 278371a0 (no quantization).
- `oom-scalar/`, `oom-pq/`: the same build, quantized: **killed by the
  16 GiB limit while building the index**. Quantizing copied every vector
  to train on a sample of 10,000 to 25,000 (3 GB a copy at 768 dims; PQ
  made three and held the residuals beside the arena), and the vectors by
  key were still held while it ran.
- `scalar/`, `pq/`: with the fix (only the sample is copied, residuals in
  place, vectors by key freed before quantizing). Same codebooks.

| mode | RSS | index build | search ef 100, c1 / c8 | recall@10 | filtered ef 40, c1 | filtered recall |
|---|---|---|---|---|---|---|
| f32 | 10.95 GiB | 463 s | 860 / 3,350 | 0.785 (0.899 at ef 200: 604/s) | 342 | 0.921 |
| scalar | 8.38 GiB (−23%) | 451 s | 385 / 2,081 | 0.791 | 152 | 0.973 |
| pq | 8.10 GiB (−26%) | 477 s | 137 / 725 | 0.924 (flat in ef) | 67 | 0.993 |

- The vectors weigh 2.9 GiB in f32: scalar saves 2.6 GiB of them and PQ 2.85,
  as expected. What remains (8 GiB) is mostly the documents, which keep
  their own copy of each vector, and the storage cache.
- At the same recall scalar answers 45% of f32's searches at one client
  (62% at eight). PQ answers 23% of f32's at ef 200 for a higher recall;
  its speed is set by re-scoring 200 candidates a query from the documents.
- At 128 dims the saving was 0.27 (scalar) and 0.43 GiB (PQ) of 3.97.
