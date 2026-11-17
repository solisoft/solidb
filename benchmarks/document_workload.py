#!/usr/bin/env python3
"""The document workload benchmark: the nightly job of an indicator engine.

`mv_doc` holds one document per metric and day (500 x 365 = 182,500 by
default), for the 365 days ending 2026-11-16:

    { "_key": "m42_20261116", "m": 42, "d": 20773,
      "v": { "1": 67, ..., "20": 90 }, "s": 970, "n": 20 }

    v[p] = (m * 31 + p * 17 + i * 7) % 100     i: the day's index, 0..364
    d    = days since 1970-01-01, s = sum of v, n = 20

with persistent indexes on ["d"] and on ["m", "d"]. 1,000 indicators of 5
distinct metrics each are drawn with random.Random(7). Three problems:

  worker   one query: the 3-day average of every metric and perimeter
           (500 DOCUMENT() calls of 3 keys, 10,000 AVG subqueries), then
           per indicator (a0 + a1) / (a2 + a3 + a4) * 100 per perimeter,
           written to `iv_doc` with overwrite. Also timed cut after the
           LET, and without the INSERT.
  collect  FOR x IN mv_doc FILTER x.d >= @f COLLECT m = x.m
           AGGREGATE s = SUM(x.s), n = SUM(x.n) RETURN {m, a: s / n},
           over the year and over the last 30 days.
  remove   FOR x IN mv_doc FILTER x.d < @c REMOVE x IN mv_doc, the first
           181 days (90,500 documents), which are loaded again after
           each run.

Every timing is a client-side median over one keep-alive connection through
POST /_api/database/_system/cursor. A bind value changes at every run, the
answer must say "cached": false, and every result is checked against a
computation made here.

    python3 benchmarks/document_workload.py --solidb target/release/solidb
    python3 benchmarks/document_workload.py --solidb … --only collect
    python3 benchmarks/document_workload.py --solidb … --json out.json
    python3 benchmarks/document_workload.py --solidb … --spin worker --seconds 30
        (runs one problem in a loop and prints the server's pid, for a profiler)

Only the standard library is used.
"""

import argparse
import base64
import datetime
import http.client
import json
import os
import random
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time

DB = "_system"
LAST_DAY = datetime.date(2026, 11, 16)
EPOCH = datetime.date(1970, 1, 1)

WORKER_LET = """
LET mavg = ZIP(
  (FOR m IN @metrics RETURN TO_STRING(m)),
  (FOR m IN @metrics
     LET ds = DOCUMENT('mv_doc', (FOR d IN @days RETURN CONCAT('m', m, '_', d)))
     RETURN ZIP(@perimeters, (FOR p IN @perimeters
       RETURN AVG((FOR x IN ds FILTER x != null AND x.v[p] != null RETURN x.v[p])))))
)
"""
WORKER_LOOP = """
FOR ind IN @inds
  LET a = (FOR m IN ind.metrics RETURN mavg[TO_STRING(m)])
  LET per_perimeter = (
    FOR p IN @perimeters
      RETURN {p, v: (a[0][p] + a[1][p]) / (a[2][p] + a[3][p] + a[4][p]) * 100}
  )
"""
WORKER_INSERT = """
  INSERT {_key: CONCAT('i', ind.id, '_', @day), indicator: ind.id, day: @day,
          perimeters: ZIP(per_perimeter[*].p, per_perimeter[*].v), value: AVG(per_perimeter[*].v)}
    INTO iv_doc OPTIONS {overwrite: true}
"""
COLLECT = """
FOR x IN mv_doc FILTER x.d >= @f
  COLLECT m = x.m AGGREGATE s = SUM(x.s), n = SUM(x.n)
  RETURN {m, a: s / n}
"""
REMOVE = "FOR x IN mv_doc FILTER x.d < @c REMOVE x IN mv_doc"


class Server:
    """A throwaway solidb on its own data directory."""

    def __init__(self, binary, port, data_dir, engine):
        self.port = port
        self.data_dir = data_dir
        env = dict(os.environ, SOLIDB_ADMIN_PASSWORD="bench-password")
        if engine:
            env["SOLIDB_ENGINE"] = engine
        self.log = open(os.path.join(data_dir, "server.log"), "w")
        self.process = subprocess.Popen(
            [binary, "--host", "127.0.0.1", "--port", str(port),
             "--data-dir", os.path.join(data_dir, "data"), "--no-lua"],
            env=env, stdout=self.log, stderr=subprocess.STDOUT, cwd=data_dir)
        self.auth = "Basic " + base64.b64encode(b"admin:bench-password").decode()
        self.connection = None
        for _ in range(240):
            if self.process.poll() is not None:
                raise SystemExit(f"the server exited: see {data_dir}/server.log")
            try:
                status, _ = self.call("GET", "/_api/health")
                if status == 200:
                    return
            except OSError:
                pass
            time.sleep(0.25)
        raise SystemExit("the server never answered")

    def call(self, method, path, body=None, timeout=3600):
        payload = None if body is None else json.dumps(body).encode()
        headers = {"Authorization": self.auth, "Content-Type": "application/json"}
        for attempt in (0, 1):
            if self.connection is None:
                self.connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)
            try:
                self.connection.request(method, path, body=payload, headers=headers)
                response = self.connection.getresponse()
                raw = response.read()
                break
            except (http.client.HTTPException, OSError):
                self.connection.close()
                self.connection = None
                if attempt:
                    raise
        try:
            return response.status, json.loads(raw) if raw else None
        except ValueError:
            return response.status, raw.decode(errors="replace")

    def ok(self, method, path, body=None):
        status, answer = self.call(method, path, body)
        if status >= 300:
            raise SystemExit(f"{method} {path} -> {status}: {answer}")
        return answer

    def query(self, text, binds=None):
        """Runs a query; returns (rows, seconds). The answer must not be cached."""
        body = {"query": text, "bindVars": binds or {}, "batchSize": 1_000_000}
        start = time.perf_counter()
        answer = self.ok("POST", f"/_api/database/{DB}/cursor", body)
        seconds = time.perf_counter() - start
        if answer.get("cached"):
            raise SystemExit(f"answered from the query cache: {text[:60]!r}")
        return answer.get("result"), seconds

    def stop(self):
        if self.process.poll() is not None:
            return
        if self.connection is not None:
            self.connection.close()
        self.process.send_signal(signal.SIGTERM)
        try:
            self.process.wait(timeout=60)
        except subprocess.TimeoutExpired:
            self.process.kill()
        self.log.close()


class Data:
    """What the generator knows: the documents and the answers."""

    def __init__(self, metrics, days, perimeters, indicators):
        self.metrics = metrics
        self.days = days
        self.perimeters = perimeters
        self.first = LAST_DAY - datetime.timedelta(days=days - 1)
        self.first_d = (self.first - EPOCH).days
        rnd = random.Random(7)
        self.indicators = [
            {"id": i, "metrics": rnd.sample(range(1, metrics + 1), 5)}
            for i in range(indicators)
        ]

    def value(self, m, p, i):
        return (m * 31 + p * 17 + i * 7) % 100

    def stamp(self, i):
        return (self.first + datetime.timedelta(days=i)).strftime("%Y%m%d")

    def document(self, m, i):
        v = {str(p): self.value(m, p, i) for p in range(1, self.perimeters + 1)}
        return {"_key": f"m{m}_{self.stamp(i)}", "m": m, "d": self.first_d + i,
                "v": v, "s": sum(v.values()), "n": self.perimeters}

    def documents(self, from_day, to_day):
        """Day by day, metric by metric: the order of the load."""
        for i in range(from_day, to_day):
            for m in range(1, self.metrics + 1):
                yield self.document(m, i)

    def worker_binds(self, last):
        """The binds of the worker query for the 3 days ending at index `last`."""
        days = [self.stamp(last - back) for back in range(3)]
        return {
            "metrics": list(range(1, self.metrics + 1)),
            "perimeters": [str(p) for p in range(1, self.perimeters + 1)],
            "days": days, "day": days[0], "inds": self.indicators,
        }

    def worker_reference(self, last):
        """indicator id -> (perimeter -> value, global value)."""
        window = [last - back for back in range(3) if last - back >= 0]
        avg = {
            m: {p: sum(self.value(m, p, i) for i in window) / len(window)
                for p in range(1, self.perimeters + 1)}
            for m in range(1, self.metrics + 1)
        }
        out = {}
        for ind in self.indicators:
            a = [avg[m] for m in ind["metrics"]]
            per = {}
            for p in range(1, self.perimeters + 1):
                below = a[2][p] + a[3][p] + a[4][p]
                per[str(p)] = None if below == 0 else (a[0][p] + a[1][p]) / below * 100
            kept = [v for v in per.values() if v is not None]
            out[ind["id"]] = (per, sum(kept) / len(kept) if kept else None)
        return out

    def collect_reference(self, from_day):
        """metric -> average of every value from day index `from_day` on."""
        out = {}
        for m in range(1, self.metrics + 1):
            total = sum(self.value(m, p, i)
                        for i in range(from_day, self.days)
                        for p in range(1, self.perimeters + 1))
            out[m] = total / ((self.days - from_day) * self.perimeters)
        return out


def close(a, b):
    if a is None or b is None:
        return a is b
    return abs(a - b) <= 1e-9 * max(1.0, abs(a), abs(b))


def fail(what):
    raise SystemExit(f"WRONG RESULT: {what}")


def load(server, data, from_day, to_day, batch=5000):
    """Inserts the documents of the days [from_day, to_day); returns seconds."""
    start = time.perf_counter()
    pending = []
    for document in data.documents(from_day, to_day):
        pending.append(document)
        if len(pending) == batch:
            server.query("FOR x IN @docs INSERT x INTO mv_doc", {"docs": pending})
            pending = []
    if pending:
        server.query("FOR x IN @docs INSERT x INTO mv_doc", {"docs": pending})
    return time.perf_counter() - start


def count(server, name):
    rows, _ = server.query(f"RETURN COLLECTION_COUNT('{name}')")
    return rows[0]


def setup(server, data):
    out = {}
    for name in ("mv_doc", "iv_doc"):
        server.ok("POST", f"/_api/database/{DB}/collection", {"name": name})
    out["load_s"] = load(server, data, 0, data.days)
    for name, fields in (("idx_d", ["d"]), ("idx_m_d", ["m", "d"])):
        start = time.perf_counter()
        server.ok("POST", f"/_api/database/{DB}/index/mv_doc",
                  {"name": name, "fields": fields, "type": "persistent"})
        out[f"{name}_s"] = time.perf_counter() - start
    if count(server, "mv_doc") != data.metrics * data.days:
        fail("mv_doc does not hold every document")
    return out


def check_worker(server, data, last):
    rows, _ = server.query(
        "FOR x IN iv_doc FILTER x.day == @day RETURN x", {"day": data.stamp(last)})
    reference = data.worker_reference(last)
    if len(rows) != len(reference):
        fail(f"iv_doc holds {len(rows)} indicators of day {data.stamp(last)}, not {len(reference)}")
    for row in rows:
        per, value = reference[row["indicator"]]
        if row["_key"] != f"i{row['indicator']}_{data.stamp(last)}":
            fail(f"key {row['_key']}")
        if not close(row["value"], value):
            fail(f"indicator {row['indicator']}: {row['value']} != {value}")
        if set(row["perimeters"]) != set(per):
            fail(f"indicator {row['indicator']}: perimeters {sorted(row['perimeters'])}")
        for p, v in per.items():
            if not close(row["perimeters"][p], v):
                fail(f"indicator {row['indicator']} perimeter {p}: {row['perimeters'][p]} != {v}")


def worker_runs(data, runs):
    """Day indexes, one per run and one for the warm-up, all different."""
    return [data.days - 1 - k for k in range(runs + 1)]


def bench_worker(server, data, runs):
    out = {}
    lasts = worker_runs(data, runs)
    shapes = (
        ("worker_let", WORKER_LET + "RETURN LENGTH(mavg)"),
        ("worker_no_insert", WORKER_LET + WORKER_LOOP + "RETURN AVG(per_perimeter[*].v)"),
        ("worker", WORKER_LET + WORKER_LOOP + WORKER_INSERT),
    )
    for name, text in shapes:
        times = []
        for run, last in enumerate(lasts):
            rows, seconds = server.query(text, data.worker_binds(last))
            if run:
                times.append(seconds)
            if name == "worker_let" and rows != [data.metrics]:
                fail(f"LENGTH(mavg) = {rows}")
            if name == "worker_no_insert":
                reference = data.worker_reference(last)
                if len(rows) != len(reference) or not all(
                        close(row, reference[i][1]) for i, row in enumerate(rows)):
                    fail(f"global values of day {data.stamp(last)}")
            if name == "worker":
                check_worker(server, data, last)
        out[name] = times
    # A window that reaches before the first day: DOCUMENT() with missing keys.
    server.query(shapes[2][1], data.worker_binds(1))
    check_worker(server, data, 1)
    return out


def bench_collect(server, data, runs):
    out = {}
    for name, from_day in (("collect_year", 0), ("collect_30_days", data.days - 30)):
        reference = data.collect_reference(from_day)
        times = []
        for run in range(runs + 1):
            # The same documents at every run, another bind value.
            f = data.first_d + from_day - 0.001 * (run + 1)
            rows, seconds = server.query(COLLECT, {"f": f})
            if run:
                times.append(seconds)
            answer = {row["m"]: row["a"] for row in rows}
            if len(rows) != len(reference) or set(answer) != set(reference):
                fail(f"{name}: {len(rows)} groups")
            for m, a in reference.items():
                if not close(answer[m], a):
                    fail(f"{name}: metric {m}: {answer[m]} != {a}")
        out[name] = times
    return out


def index_entries(server, data):
    """What the two indexes answer for: counts read through each of them."""
    by_d, _ = server.query(
        "FOR x IN mv_doc FILTER x.d == @d COLLECT WITH COUNT INTO n RETURN n",
        {"d": data.first_d})
    by_m, _ = server.query(
        "FOR x IN mv_doc FILTER x.m == 1 AND x.d < @c RETURN x._key",
        {"c": data.first_d + data.days})
    return by_d[0], len(by_m)


def bench_remove(server, data, runs):
    cut = min(181, data.days - 1)
    times = []
    for run in range(runs + 1):
        c = data.first_d + cut - 0.001 * (run + 1)
        _, seconds = server.query(REMOVE, {"c": c})
        if run:
            times.append(seconds)
        if count(server, "mv_doc") != data.metrics * (data.days - cut):
            fail("REMOVE left another number of documents")
        left, _ = server.query(
            "FOR x IN mv_doc FILTER x.d < @c LIMIT 1 RETURN x._key", {"c": c})
        if left:
            fail(f"REMOVE left {left}")
        if index_entries(server, data) != (0, data.days - cut):
            fail(f"the indexes still answer for removed documents: {index_entries(server, data)}")
        load(server, data, 0, cut)
        if index_entries(server, data) != (data.metrics, data.days):
            fail(f"the indexes after the reload: {index_entries(server, data)}")
    return {"remove": times}


PROBLEMS = {"worker": bench_worker, "collect": bench_collect, "remove": bench_remove}
LABELS = {
    "worker_let": "worker, cut after LET mavg",
    "worker_no_insert": "worker, without the INSERT",
    "worker": "worker, whole query",
    "collect_year": "COLLECT over the year",
    "collect_30_days": "COLLECT over 30 days",
    "remove": "range REMOVE",
}


def spin(server, data, problem, seconds):
    """One problem in a loop, for a profiler attached to the server."""
    print(f"pid {server.process.pid}: {problem} for {seconds} s", flush=True)
    end = time.time() + seconds
    run = 0
    while time.time() < end:
        run += 1
        if problem == "worker":
            server.query(WORKER_LET + WORKER_LOOP + WORKER_INSERT,
                         data.worker_binds(data.days - 1 - run % 300))
        elif problem == "worker_let":
            server.query(WORKER_LET + "RETURN LENGTH(mavg)",
                         data.worker_binds(data.days - 1 - run % 300))
        elif problem == "collect":
            server.query(COLLECT, {"f": data.first_d - 0.001 * run})
        elif problem == "collect_30_days":
            server.query(COLLECT, {"f": data.first_d + data.days - 30 - 0.001 * run})
        elif problem == "remove":
            cut = min(181, data.days - 1)
            server.query(REMOVE, {"c": data.first_d + cut - 0.001 * run})
            load(server, data, 0, cut)
        else:
            raise SystemExit(f"unknown problem {problem}")
    print(f"{run} runs", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--solidb", required=True, help="the solidb binary")
    parser.add_argument("--engine", default="", help="rocksdb or solistore (default: the binary's)")
    parser.add_argument("--port", type=int, default=16745)
    parser.add_argument("--metrics", type=int, default=500)
    parser.add_argument("--days", type=int, default=365)
    parser.add_argument("--perimeters", type=int, default=20)
    parser.add_argument("--indicators", type=int, default=1000)
    parser.add_argument("--runs", type=int, default=6)
    parser.add_argument("--only", default="", help="worker, collect, remove (comma separated)")
    parser.add_argument("--spin", default="", help="run one problem in a loop, for a profiler")
    parser.add_argument("--seconds", type=int, default=30)
    parser.add_argument("--json", default="", help="write the figures there")
    parser.add_argument("--tmp", default=None,
                        help="where the throwaway data directory is made (default: the system's)")
    args = parser.parse_args()
    if args.days < 4 or args.metrics < 5:
        raise SystemExit("at least 4 days and 5 metrics")

    data = Data(args.metrics, args.days, args.perimeters, args.indicators)
    data_dir = tempfile.mkdtemp(prefix="solidb-docbench-", dir=args.tmp)
    server = Server(os.path.abspath(args.solidb), args.port, data_dir, args.engine)
    figures = {}
    try:
        figures.update(setup(server, data))
        print(f"load {figures['load_s']:.1f} s, index d {figures['idx_d_s']:.2f} s, "
              f"index m,d {figures['idx_m_d_s']:.2f} s", flush=True)
        if args.spin:
            spin(server, data, args.spin, args.seconds)
            return
        only = [name for name in args.only.split(",") if name] or list(PROBLEMS)
        for name in only:
            for key, times in PROBLEMS[name](server, data, args.runs).items():
                figures[key] = {"median_ms": statistics.median(times) * 1000,
                                "runs_ms": [t * 1000 for t in times]}
                print(f"{LABELS[key]:32} {figures[key]['median_ms']:9.1f} ms   "
                      f"(min {min(times) * 1000:.1f}, max {max(times) * 1000:.1f}, "
                      f"{len(times)} runs)", flush=True)
        print("every result checked against the reference")
    finally:
        server.stop()
        shutil.rmtree(data_dir, ignore_errors=True)
    if args.json:
        with open(args.json, "w") as out:
            json.dump(figures, out, indent=2)


if __name__ == "__main__":
    main()
