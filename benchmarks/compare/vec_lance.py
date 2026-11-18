#!/usr/bin/env python3
"""The vector harness's searches on LanceDB embedded (card #979).

LanceDB is a library that runs in its client's process, so it is measured as
such, with no server and no network: this script is the client. It reads what
`solidb-vec --engine dump --out DIR` wrote (the same vectors, queries and exact
answers as every other engine), loads them into a local LanceDB table, builds
an index, and measures what `solidb-vec` measures: recall@k over every query,
then requests/s and latency with 1 and more threads, plain and filtered on
`cat`.

    python3 vec_lance.py DIR [--index IVF_HNSW_SQ] [--ef 40,100,200]
        [--concurrency 1,8] [--warm 3] [--secs 10] [--runs 3] [--out FILE]

The index is IVF_HNSW_SQ with the HNSW settings of the other engines (m 16,
ef_construction 200) unless `--index` names another of LanceDB's kinds;
`--nprobes` sets the IVF partitions searched (LanceDB's default otherwise).
A filtered search is `.where("cat = c", prefilter=True)`.
"""

import argparse
import json
import os
import shutil
import tempfile
import threading
import time

import lancedb
import numpy as np
import pyarrow as pa


def main():
    p = argparse.ArgumentParser()
    p.add_argument("dir")
    p.add_argument("--index", default="IVF_HNSW_SQ")
    p.add_argument("--m", type=int, default=16)
    p.add_argument("--ef-construction", type=int, default=200)
    p.add_argument("--nprobes", type=int, default=None)
    p.add_argument("--ef", default="40,100,200")
    p.add_argument("--concurrency", default="1,8")
    p.add_argument("--warm", type=float, default=3)
    p.add_argument("--secs", type=float, default=10)
    p.add_argument("--runs", type=int, default=3)
    p.add_argument("--table-dir", default=None, help="where the table lives (a temporary directory otherwise)")
    p.add_argument("--out", default=None)
    a = p.parse_args()

    truth = json.load(open(os.path.join(a.dir, "truth.json")))
    n, dim, k = truth["n"], truth["dim"], truth["k"]
    data = np.fromfile(os.path.join(a.dir, "data.f32"), dtype="<f4").reshape(n, dim)
    queries = np.fromfile(os.path.join(a.dir, "queries.f32"), dtype="<f4").reshape(-1, dim)
    cats = truth["cats"]
    print(f"[vec] lancedb {lancedb.__version__}: {n} vectors of {dim} dims, {len(queries)} queries", flush=True)

    where = a.table_dir or tempfile.mkdtemp(prefix="lance979-")
    shutil.rmtree(where, ignore_errors=True)
    db = lancedb.connect(where)
    ids = np.arange(n, dtype=np.int64)
    table_data = pa.table({
        "id": pa.array(ids),
        "cat": pa.array(ids % truth["categories"]),
        "vector": pa.FixedSizeListArray.from_arrays(pa.array(data.reshape(-1)), dim),
    })
    began = time.time()
    tbl = db.create_table("items", table_data)
    load_secs = time.time() - began
    began = time.time()
    if a.index.startswith("IVF_HNSW"):
        tbl.create_index(metric="cosine", vector_column_name="vector", index_type=a.index,
                         m=a.m, ef_construction=a.ef_construction)
    else:
        tbl.create_index(metric="cosine", vector_column_name="vector", index_type=a.index)
    tbl.create_scalar_index("cat")
    index_secs = time.time() - began
    print(f"[vec] loaded in {load_secs:.1f} s, index ready {index_secs:.1f} s later", flush=True)

    def search(q, ef, cat):
        b = tbl.search(q, vector_column_name="vector").distance_type("cosine").limit(k).ef(ef).select(["id"])
        if a.nprobes:
            b = b.nprobes(a.nprobes)
        if cat is not None:
            b = b.where(f"cat = {cat}", prefilter=True)
        return [int(i) for i in b.to_arrow().column("id").to_pylist()]

    def recall(ef, filtered):
        want = truth["filtered"] if filtered else truth["plain"]
        found = 0
        for i, q in enumerate(queries):
            got = search(q, ef, cats[i] if filtered else None)
            assert len(got) <= k, f"{len(got)} ids for a top-{k}"
            found += len(set(want[i]) & set(got))
        return found / (k * len(queries))

    def timed(ef, filtered, threads, secs):
        stop = threading.Event()
        lats, counts = [[] for _ in range(threads)], [0] * threads

        def worker(w):
            i = w * 7919
            while not stop.is_set():
                j = i % len(queries)
                i += 1
                t = time.perf_counter()
                search(queries[j], ef, cats[j] if filtered else None)
                lats[w].append(int((time.perf_counter() - t) * 1e6))
                counts[w] += 1

        ts = [threading.Thread(target=worker, args=(w,)) for w in range(threads)]
        began = time.perf_counter()
        for t in ts:
            t.start()
        time.sleep(secs)
        stop.set()
        for t in ts:
            t.join()
        elapsed = time.perf_counter() - began
        all_lats = sorted(x for l in lats for x in l)
        at = lambda q: all_lats[min(int(len(all_lats) * q), len(all_lats) - 1)] if all_lats else None
        return sum(counts), elapsed, {"p50_us": at(0.5), "p90_us": at(0.9), "p99_us": at(0.99),
                                      "max_us": all_lats[-1] if all_lats else None}

    series = []
    efs = [int(x) for x in a.ef.split(",")]
    conc = [int(x) for x in a.concurrency.split(",")]
    for name, filtered in (("search", False), ("filtered", True)):
        for ef in efs:
            r = recall(ef, filtered)
            print(f"[vec] {name} ef={ef}: recall@{k} {r:.4f}", flush=True)
            for c in conc:
                timed(ef, filtered, c, a.warm)
                for run in range(a.runs):
                    ops, secs, p = timed(ef, filtered, c, a.secs)
                    print(f"[vec] {name} ef={ef} c={c} run {run}: {ops / secs:.0f}/s p50 {p['p50_us']} µs p99 {p['p99_us']} µs", flush=True)
                    series.append({"scenario": name, "ef": ef, "concurrency": c, "run": run, "recall": r,
                                   "ops": ops, "secs": secs, "ops_per_sec": ops / secs, **p})

    disk = sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(where) for f in fs)
    report = {"engine": "lancedb", "server_version": f"lancedb {lancedb.__version__} (embedded, in the client)",
              "n": n, "dim": dim, "queries": len(queries), "k": k, "index": a.index,
              "m": a.m, "ef_construction": a.ef_construction, "nprobes": a.nprobes,
              "load_secs": load_secs, "index_secs": index_secs, "disk_bytes": disk,
              "series_results": series}
    text = json.dumps(report, indent=2)
    if a.out:
        open(a.out, "w").write(text)
    else:
        print(text)
    if not a.table_dir:
        shutil.rmtree(where, ignore_errors=True)


if __name__ == "__main__":
    main()
