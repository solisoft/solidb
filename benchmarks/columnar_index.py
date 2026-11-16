#!/usr/bin/env python3
"""The columnar index benchmark: a history of metric values.

One row per metric, perimeter and day (500 x 20 x 365 = 3,650,000 by
default), in a columnar collection `mv_col` with the columns `metric_id`,
`perimeter_id`, `day` (a timestamp in milliseconds) and `value`, where

    value = (metric * 31 + perimeter * 17 + day_index * 7) % 100

The rows are generated here, in the order day, metric, perimeter, so two
runs load the same data. The script starts a throwaway server on its own
data directory, and prints:

  - the load time, and the time of an insert of one night (10,000 rows)
    without an index and with the two indexes;
  - the time to build the indexes on `metric_id` and on `day`;
  - the median of 5 runs, after a warm-up, of five questions;
  - the size of the data directory after the load, after the indexes and
    after a compaction, and what each index takes.

    python3 benchmarks/columnar_index.py --solidb target/release/solidb
    python3 benchmarks/columnar_index.py --solidb … --days 30      # a smaller pass
    python3 benchmarks/columnar_index.py --solidb … --json out.json

Only the standard library is used. Every answer is checked against what
the generator knows (row counts, and the averages of the formula).
"""

import argparse
import base64
import http.client
import json
import os
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time

DB = "_system"
NAME = "mv_col"
DAY_MS = 86_400_000
FIRST_DAY_MS = 1_735_689_600_000  # 2025-01-01T00:00:00Z


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

    def rss_mb(self):
        with open(f"/proc/{self.process.pid}/status") as status:
            for line in status:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) / 1024
        return 0.0

    def cpu_seconds(self):
        with open(f"/proc/{self.process.pid}/stat") as stat:
            fields = stat.read().rsplit(")", 1)[1].split()
        return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")

    def disk_mb(self, suffix=""):
        """What the files of the data directory take (those ending in `suffix`)."""
        total = 0
        for root, _, files in os.walk(os.path.join(self.data_dir, "data")):
            for name in files:
                if name.endswith(suffix):
                    try:
                        total += os.path.getsize(os.path.join(root, name))
                    except OSError:
                        pass
        return total / 1e6

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


def rows_of_day(day, metrics, perimeters):
    at = FIRST_DAY_MS + day * DAY_MS
    return [
        {"metric_id": metric, "perimeter_id": perimeter, "day": at,
         "value": (metric * 31 + perimeter * 17 + day * 7) % 100}
        for metric in range(metrics) for perimeter in range(perimeters)
    ]


def timed(run):
    start = time.perf_counter()
    answer = run()
    return time.perf_counter() - start, answer


def median_of(run, runs=5):
    """The median of `runs` runs after one that is not counted."""
    run()
    return statistics.median(timed(run)[0] for _ in range(runs))


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--solidb", required=True, help="the solidb binary to measure")
    parser.add_argument("--port", type=int, default=6791)
    parser.add_argument("--metrics", type=int, default=500)
    parser.add_argument("--perimeters", type=int, default=20)
    parser.add_argument("--days", type=int, default=365)
    parser.add_argument("--batch", type=int, default=20_000, help="rows per insert of the load")
    parser.add_argument("--engine", default="", help="rocksdb (the default) or solistore")
    parser.add_argument("--data-dir", default="", help="where the server keeps its data (default: a temporary directory)")
    parser.add_argument("--keep", action="store_true", help="leave the data directory behind")
    parser.add_argument("--json", default="", help="write the figures there as JSON")
    args = parser.parse_args()

    metrics, perimeters, days = args.metrics, args.perimeters, args.days
    night = metrics * perimeters
    total = night * days
    data_dir = os.path.abspath(args.data_dir or tempfile.mkdtemp(prefix="columnar-index-"))
    os.makedirs(data_dir, exist_ok=True)
    server = Server(os.path.abspath(args.solidb), args.port, data_dir, args.engine)
    base = f"/_api/database/{DB}/columnar"
    out = {"rows": total, "binary": args.solidb}
    try:
        version = subprocess.run([args.solidb, "--version"], capture_output=True, text=True).stdout.strip()
        out["version"] = version
        print(f"{version}: {total:,} rows = {metrics} metrics x {perimeters} perimeters x {days} days")

        server.ok("POST", base, {
            "name": NAME, "compression": "lz4",
            "columns": [{"name": "metric_id", "type": "int64"}, {"name": "perimeter_id", "type": "int64"},
                        {"name": "day", "type": "timestamp"}, {"name": "value", "type": "int64"}]})

        # ---- load -------------------------------------------------------
        def load():
            pending = []
            for day in range(days):
                pending.extend(rows_of_day(day, metrics, perimeters))
                while len(pending) >= args.batch:
                    server.ok("POST", f"{base}/{NAME}/insert", {"rows": pending[:args.batch]})
                    del pending[:args.batch]
            if pending:
                server.ok("POST", f"{base}/{NAME}/insert", {"rows": pending})

        generated = timed(lambda: [rows_of_day(day, metrics, perimeters) for day in range(days)])[0]
        out["load_s"], _ = timed(load)
        out["generate_s"] = generated
        print(f"load                         {out['load_s']:8.1f} s   (of which {generated:.1f} s generating the rows here)")

        def stored():
            """What the blocks and the tail take, as the server counts it."""
            stats = server.ok("GET", f"{base}/{NAME}").get("stats", {})
            return stats.get("compressed_size_bytes", 0) / 1e6, stats.get("uncompressed_size_bytes", 0) / 1e6

        out["rows_stored_mb"], out["rows_plain_mb"] = stored()
        print(f"rows as stored               {out['rows_stored_mb']:8.1f} MB   ({out['rows_plain_mb']:.1f} MB before LZ4)")

        # ---- one night, with and without the indexes ---------------------
        def nights(first):
            """Insert 5 nights after the last day, time each, take them out again."""
            times = []
            for extra in range(5):
                rows = rows_of_day(first + extra, metrics, perimeters)
                times.append(timed(lambda: server.ok("POST", f"{base}/{NAME}/insert", {"rows": rows}))[0])
            server.ok("POST", f"{base}/{NAME}/delete", {
                "filter": {"column": "day", "op": "GTE", "value": FIRST_DAY_MS + days * DAY_MS}})
            return statistics.median(times)

        out["insert_night_plain_s"] = nights(days)
        print(f"insert {night:,} rows, no index   {out['insert_night_plain_s'] * 1000:8.1f} ms")

        # ---- the indexes --------------------------------------------------
        def build(column, kind):
            cpu, start = server.cpu_seconds(), time.perf_counter()
            status, answer = server.call("POST", f"{base}/{NAME}/index?wait=false", {"column": column, "index_type": kind})
            if status >= 300:
                return None, answer
            job = answer.get("job") if isinstance(answer, dict) else None
            while job and answer.get("status") not in ("done", "created", "failed"):
                time.sleep(0.05)
                answer = server.ok("GET", f"{base}/{NAME}/index/{column}")
            elapsed = time.perf_counter() - start
            return elapsed, {"cpu_s": server.cpu_seconds() - cpu, "answer": answer}

        out["indexes"] = {}
        for column, kind in (("metric_id", "sorted"), ("day", "minmax"), ("day", "sorted")):
            elapsed, detail = build(column, kind)
            label = f"index {column} ({kind})"
            if elapsed is None:
                print(f"{label:28} refused: {str(detail)[:110]}")
                out["indexes"][f"{column}:{kind}"] = {"refused": str(detail)}
                continue
            out["indexes"][f"{column}:{kind}"] = {"build_s": elapsed, "cpu_s": detail["cpu_s"]}
            print(f"{label:28} {elapsed:8.2f} s   ({detail['cpu_s']:.1f} s of CPU, {server.rss_mb():.0f} MB RSS)")
            if column == "day":
                break
        out["index_build_s"] = sum(i.get("build_s", 0) for i in out["indexes"].values())
        print(f"both indexes                 {out['index_build_s']:8.2f} s")
        listed = server.ok("GET", f"{base}/{NAME}/indexes")
        for index in listed.get("indexes", []):
            if "size_bytes" in index:
                out["indexes"].setdefault(f"{index['column']}:{index['index_type']}", {})["size_mb"] = index["size_bytes"] / 1e6
                print(f"  index on {index['column']:18} {index['size_bytes'] / 1e6:8.1f} MB   ({index.get('rows', '?'):,} rows held)")

        # ---- the questions ------------------------------------------------
        metric, perimeter = min(42, metrics - 1), min(7, perimeters - 1)
        last = FIRST_DAY_MS + (days - 1) * DAY_MS
        query = lambda body: server.ok("POST", f"{base}/{NAME}/query", body)
        eq = lambda column, value: {"column": column, "op": "EQ", "value": value}

        def expect(answer, count, what):
            got = answer["count"] if isinstance(answer, dict) and "count" in answer else len(answer)
            if got != count:
                raise SystemExit(f"{what}: {got} rows, expected {count}")

        def one_series():
            expect(query({"columns": ["day", "value"], "filter": {"and": [eq("metric_id", metric), eq("perimeter_id", perimeter)]}}),
                   days, "one metric on one perimeter")

        def one_metric():
            expect(query({"columns": ["perimeter_id", "day", "value"], "filter": eq("metric_id", metric)}),
                   days * perimeters, "one metric on its perimeters")

        def average_of(m, day_range):
            values = [(m * 31 + p * 17 + d * 7) % 100 for p in range(perimeters) for d in day_range]
            return sum(values) / len(values)

        def check_groups(groups, day_range, what):
            if len(groups) != metrics:
                raise SystemExit(f"{what}: {len(groups)} groups, expected {metrics}")
            for group in groups:
                m = int(group["metric_id"])
                value = group.get("_agg", group.get("avg"))
                if abs(value - average_of(m, day_range)) > 1e-9:
                    raise SystemExit(f"{what}: metric {m} averages {value}, expected {average_of(m, day_range)}")

        def averages():
            answer = server.ok("POST", f"{base}/{NAME}/aggregate",
                               {"column": "value", "operation": "AVG", "group_by": ["metric_id"]})
            check_groups(answer["groups"], range(days), "average per metric")

        def averages_sdbql():
            answer = server.ok("POST", f"/_api/database/{DB}/cursor", {
                "query": f"FOR r IN {NAME} COLLECT metric_id = r.metric_id AGGREGATE avg = AVG(r.value) RETURN {{metric_id, avg}}",
                "batchSize": 100000})
            check_groups(answer["result"], range(days), "average per metric (SDBQL)")

        recent = range(max(0, days - 30), days)
        since = FIRST_DAY_MS + recent.start * DAY_MS

        def averages_recent():
            status, answer = server.call("POST", f"{base}/{NAME}/aggregate", {
                "column": "value", "operation": "AVG", "group_by": ["metric_id"],
                "filter": {"column": "day", "op": "GTE", "value": since}})
            if status >= 300:
                raise SystemExit(f"filtered aggregate -> {status}: {answer}")
            check_groups(answer["groups"], recent, "average per metric, last 30 days")

        def last_day():
            expect(query({"columns": ["metric_id", "perimeter_id", "value"], "filter": eq("day", last)}),
                   night, "the last day")

        questions = [
            ("1 metric, 1 perimeter", one_series, days),
            ("1 metric, its perimeters", one_metric, days * perimeters),
            ("average per metric, the year", averages, metrics),
            ("the same in SDBQL (COLLECT)", averages_sdbql, metrics),
            ("average per metric, 30 days", averages_recent, metrics),
            ("all values of the last day", last_day, night),
        ]
        out["queries_ms"] = {}
        for label, run, rows in questions:
            try:
                first, _ = timed(run)
                # A question that takes minutes is run once.
                elapsed = first if first > 20 else median_of(run)
            except SystemExit as refused:
                # The filtered aggregate checks its groups against the formula:
                # an endpoint that ignores `filter` fails here.
                print(f"{label:32} n/a: {str(refused)[:90]}")
                out["queries_ms"][label] = None
                continue
            out["queries_ms"][label] = elapsed * 1000
            print(f"{label:32} {elapsed * 1000:9.2f} ms   ({rows:,} rows)")

        explain = server.call("POST", f"/_api/database/{DB}/explain", {
            "query": f"FOR r IN {NAME} FILTER r.metric_id == {metric} RETURN r.value"})[1]
        if isinstance(explain, dict):
            notes = json.dumps(explain)
            out["explain_eq"] = explain
            print("EXPLAIN of the equality on metric_id:",
                  "names the index" if "index" in notes and "sorted" in notes else "no index named")

        # ---- one night with the indexes -------------------------------------
        out["insert_night_indexed_s"] = nights(days)
        print(f"insert {night:,} rows, 2 indexes  {out['insert_night_indexed_s'] * 1000:8.1f} ms"
              f"   ({out['insert_night_indexed_s'] / out['insert_night_plain_s']:.2f} x the plain insert)")
        out["equality_after_inserts_ms"] = median_of(one_metric) * 1000
        print(f"1 metric, after those writes     {out['equality_after_inserts_ms']:9.2f} ms")

        # ---- delete the oldest half ---------------------------------------
        half = days * 181 // 365
        cutoff = FIRST_DAY_MS + half * DAY_MS
        elapsed, answer = timed(lambda: server.ok("POST", f"{base}/{NAME}/delete", {
            "filter": {"column": "day", "op": "LT", "value": cutoff}}))
        if answer["deleted"] != half * night:
            raise SystemExit(f"delete: {answer['deleted']} rows, expected {half * night}")
        out["delete_half_s"] = elapsed
        print(f"delete the rows before day {half:<4}  {elapsed:9.2f} s    ({answer['deleted']:,} rows)")
        kept_days = days - half

        def one_metric_after():
            expect(query({"columns": ["perimeter_id", "day", "value"], "filter": eq("metric_id", metric)}),
                   kept_days * perimeters, "one metric after the delete")

        out["equality_after_delete_ms"] = median_of(one_metric_after) * 1000
        print(f"1 metric, after the delete       {out['equality_after_delete_ms']:9.2f} ms")

        # ---- footprint -------------------------------------------------------
        # While the server runs the directory is mostly write-ahead log, of
        # a size that says nothing of the data: the files are measured once
        # the collection is compacted and the server has stopped.
        out["rows_stored_half_mb"], _ = stored()
        print(f"rows as stored ({kept_days} days)    {out['rows_stored_half_mb']:8.1f} MB")
        out["rss_mb"] = server.rss_mb()
        server.call("PUT", f"/_api/database/{DB}/collection/_columnar_{NAME}/compact")
        time.sleep(1)
        server.stop()
        out["disk_sst_mb"] = server.disk_mb(".sst")
        out["disk_total_mb"] = server.disk_mb()
        print(f"data dir once stopped        {out['disk_total_mb']:8.1f} MB   (of which {out['disk_sst_mb']:.1f} MB of table files, the rest log)")
        print(f"server RSS at the end        {out['rss_mb']:8.0f} MB")
    finally:
        server.stop()
        if args.json:
            with open(args.json, "w") as target:
                json.dump(out, target, indent=2)
        if not args.keep:
            shutil.rmtree(data_dir, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
