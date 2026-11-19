# Vector search, quantized indexes, 2026-11-19 (cards #996, #999, #1000, #1006)

- rocky-build2, the settings of `2026-11-18-vec-1m`: SoliDB in a systemd
  scope (`AllowedCPUs=0-3,16-19`, `MemoryMax=16G`, no swap), the harness on
  cores 8-15 + SMT; 1,000,000 vectors of 128 dimensions (100 clusters,
  normalised), 500 queries, top-10, cosine; HNSW M=16, ef_construction=200;
  medians of 3 runs of 10 s at ef 40/100/200 and 1/8 clients. The host also
  runs a production SoliDB.
- `baseline/`: the 2.15.0 vector code (release binary of 2026-11-18 18:27,
  after the last vector commit). `head/`: `main` with #994-#1002 (f32).
  `scalar/`, `binary/`, `pq/`: the same build, `QUANT=scalar|binary|pq`
  (the index created with `quantization`).
- `fix1006/`: `baseline` and the build with #1006, run back to back later.

| mode | RSS | search ef 100, 1 client | ef 200, 1 client | filtered ef 40, 1 client |
|---|---|---|---|---|
| f32 2.15.0 | 3.97 GB | 2,781/s r 0.901 | 1,780/s r 0.962 | 738/s r 0.975 |
| f32 head | 3.88 GB | 2,572/s r 0.899 | 1,673/s r 0.965 | 727/s r 0.972 |
| scalar | 3.70 GB | 1,391/s r 0.899 | 979/s r 0.960 | 356/s r 0.991 |
| binary | 3.50 GB | 494/s r 0.385 | 493/s r 0.385 | 132/s r 0.753 |
| pq | 3.53 GB | 506/s r 0.598 | 504/s r 0.598 | 138/s r 0.889 |
| f32 2.15.0 (fix run) | 4.12 GB | 2,769/s r 0.900 | 1,779/s r 0.968 | 747/s r 0.975 |
| f32 with #1006 | 3.95 GB | 2,748/s r 0.894 | 1,776/s r 0.964 | 752/s r 0.971 |

Readings:

- f32 lost 6-8% at one client with the quantization cards (`head`); #1006
  brings it back (`fix1006/`). Eight clients vary by about 8% between two
  runs of the same build here (baseline ef 200: 7,554 and 8,212/s).
- Scalar: the same recall as f32 at half the speed (#1007: the re-scoring
  reads 4·k documents, the code distance does not vectorise). At 128
  dimensions the vectors are 512 MB of a ~4 GB process: −0.27 GB.
- Binary and PQ do not hold recall at this scale (0.385 and 0.598 whatever
  `ef`; 0.99 and 0.975+ on 10,500 vectors in the unit tests): #1008. Not to
  be published or shipped as they are.
