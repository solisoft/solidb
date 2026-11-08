#!/usr/bin/env python3
"""Turn the JSON files run.sh wrote into Markdown tables.

    python3 report.py results/2026-11-08

Each figure is the median of the runs of a series. A series whose runs
spread by more than 10% (max / min of ops/s), or that had an error or a
wrong answer, is marked with `!`; one where the harness used more than 80%
of its cores is marked `client-bound`.
"""

import json
import pathlib
import statistics
import sys

ORDER = [
    "solidb", "solidb-solistore", "postgres", "postgres-typed", "mongo", "arango", "surreal",
    "solidb-solistore-durable", "postgres-durable", "mongo-durable", "arango-durable", "surreal-durable",
]
CLIENT_CORES = 16


def load(folder):
    runs = {}
    for f in sorted(pathlib.Path(folder).glob("*.json")):
        if f.name.endswith(".container.json"):
            continue
        r = json.loads(f.read_text())
        runs[r["label"]] = r
    return [runs[k] for k in ORDER if k in runs] + [v for k, v in runs.items() if k not in ORDER]


def series(r, scenario, conc):
    return [s for s in r["series"] if s["scenario"] == scenario and s["concurrency"] == conc]


def med(xs):
    return statistics.median(xs) if xs else None


def cell(r, scenario, conc):
    s = series(r, scenario, conc)
    if not s:
        return None
    # ops / secs, not the rounded ops_per_sec: a series of 4-second
    # aggregates has a rate below one.
    ops = [x["ops"] / x["secs"] for x in s]
    flags = []
    if min(ops) > 0 and max(ops) / min(ops) > 1.10:
        flags.append("!")
    if any(x["errors"] or x["wrong"] for x in s):
        flags.append("!")
    if any(x["client_cpu_cores"] > 0.8 * CLIENT_CORES for x in s):
        flags.append("client-bound")
    return {
        "ops": med(ops),
        "p50": med([x["p50_us"] for x in s]),
        "p99": med([x["p99_us"] for x in s]),
        "db_cores": med([x["db_cpu_cores"] for x in s if x.get("db_cpu_cores") is not None]),
        "spread": max(ops) / min(ops) if min(ops) > 0 else float("inf"),
        "flags": " ".join(sorted(set(flags))),
    }


def fmt_ops(v):
    return "—" if v is None else f"{v:,.0f}"


def fmt_us(v):
    if v is None:
        return "—"
    return f"{v / 1000:.2f} ms" if v >= 1000 else f"{v:.0f} µs"


def table(rows, scenario, concs):
    head = ["Engine"]
    for c in concs:
        head += [f"c={c} ops/s", f"c={c} p50", f"c={c} p99", f"c={c} DB cores"]
    out = ["| " + " | ".join(head) + " |", "|" + "---|" * len(head)]
    for r in rows:
        line = [r["label"]]
        any_cell = False
        for c in concs:
            x = cell(r, scenario, c)
            if x:
                any_cell = True
                line += [fmt_ops(x["ops"]) + (f" {x['flags']}" if x["flags"] else ""), fmt_us(x["p50"]),
                         fmt_us(x["p99"]), "—" if x["db_cores"] is None else f"{x['db_cores']:.1f}"]
            else:
                line += ["—"] * 4
        if any_cell:
            out.append("| " + " | ".join(line) + " |")
    return "\n".join(out)


def main(folder):
    rows = load(folder)
    concs = sorted({c for r in rows for c in r["concurrency"]})
    print(f"# Results in {folder}\n")
    print("## Versions and containers\n")
    for r in rows:
        m = r.get("meta", {})
        v = m.get("server_version") or r.get("server_version")
        print(f"- **{r['label']}** — {v}; `{m.get('image_digest', '')}`")
        print(f"  - `{m.get('container', '')}`")
    print()
    print("## Load (documents/s)\n")
    print("| Engine | 1 connection | 8 connections | disk after load | memory after load (anon) |")
    print("|---|---|---|---|---|")
    for r in rows:
        ph = {p["phase"]: p["result"] for p in r.get("load", [])}
        disk = (r.get("after_load") or {}).get("disk")
        mem = ((r.get("after_load") or {}).get("memory") or {}).get("anon_bytes")
        disk_s = f"{int(disk) / 2**20:,.0f} MiB" if disk and str(disk).isdigit() else "—"
        mem_s = f"{mem / 2**20:,.0f} MiB" if mem else "—"
        print(f"| {r['label']} | {fmt_ops(ph.get('orders_1', {}).get('docs_per_sec'))} | "
              f"{fmt_ops(ph.get('orders_8', {}).get('docs_per_sec'))} | {disk_s} | {mem_s} |")
    print()
    for scenario in ["get", "by_customer", "recent", "join", "insert", "update"]:
        print(f"## {scenario}\n")
        print(table(rows, scenario, concs))
        print()
    print("## aggregate (one connection)\n")
    print("| Engine | per query (p50) | p99 | queries in 20 s | DB cores |")
    print("|---|---|---|---|---|")
    for r in rows:
        x = cell(r, "aggregate", 1)
        if x:
            cores = "—" if x["db_cores"] is None else f"{x['db_cores']:.1f}"
            print(f"| {r['label']} | {fmt_us(x['p50'])} {x['flags']} | {fmt_us(x['p99'])} | "
                  f"{x['ops'] * 20:.0f} | {cores} |")
    print()
    print("## Series that took the database down\n")
    for r in rows:
        for a in r.get("aborted", []):
            print(f"- {r['label']} {a['scenario']} c={a['concurrency']}: {a['errors']} errors after "
                  f"{a['ops']} answers; first: {a['first_problem']}")
    print()
    print("## Container state after each attempt\n")
    for f in sorted(pathlib.Path(folder).glob("*.container.json")):
        for line in f.read_text().splitlines():
            if line.strip():
                st = json.loads(line)
                if st.get("oom_killed") or st.get("status") != "running":
                    print(f"- {f.name.removesuffix('.container.json')} attempt {st['attempt']}: {st['status']}, "
                          f"oom_killed={st['oom_killed']}, exit {st['exit_code']}")
    print()
    print("## Spread between runs (max/min ops/s)\n")
    worst = []
    for r in rows:
        for s in {(x["scenario"], x["concurrency"]) for x in r["series"]}:
            x = cell(r, *s)
            worst.append((x["spread"], r["label"], *s))
    for sp, label, sc, c in sorted(worst, reverse=True)[:10]:
        print(f"- {label} {sc} c={c}: {sp:.3f}")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else ".")
