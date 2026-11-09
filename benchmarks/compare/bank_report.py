#!/usr/bin/env python3
"""Markdown tables from a bank.sh results directory.

    python3 bank_report.py results/2026-11-09/bank
"""
import json
import statistics
import sys
from pathlib import Path


def load(d: Path):
    runs, crashes = [], []
    for f in sorted(d.glob("*.json")):
        v = json.loads(f.read_text())
        if "mode" not in v:  # not a solidb-bank result
            continue
        (crashes if "cycles" in v else runs).append(v)
    return runs, crashes


def us(v):
    return f"{v / 1000:.1f} ms" if v >= 10_000 else f"{v:,} µs"


def checks(bad, n):
    """'ok', or how many runs broke the checks and how."""
    if not bad:
        return "ok"
    drifts = ", ".join(f"{c['money_drift']:+,}" for c in bad if c["money_drift"])
    off = max(c["accounts_off_acknowledged"] for c in bad)
    parts = [f"**{len(bad)} of {n} runs**"]
    if drifts:
        parts.append(f"money {drifts}")
    if off:
        parts.append(f"up to {off:,} accounts wrong")
    for key, what in [("acknowledged_missing", "acknowledged transfers missing"),
                      ("negative_balances", "negative balances")]:
        worst = max(c[key] for c in bad)
        if worst:
            parts.append(f"{worst:,} {what}")
    return "; ".join(parts)


def throughput(runs):
    print("| Configuration | Accounts | Clients | Transfers/s | p50 | p99 | Conflicts per commit "
          "| DB CPU per commit | Checks | Audits off |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---|---:|")
    for v in runs:
        for s in v.get("series", []):
            rs = s["runs"]
            m = rs[s["median_run"]]
            bad = [r["check"] for r in rs if r["check"]["violations"]]
            audits = sum(r["audits"]["n"] for r in rs)
            wrong = sum(r["audits"]["wrong_total"] for r in rs)
            spread = [r["per_sec"] for r in rs]
            rate = f"{m['per_sec']:,.0f}"
            if len(spread) > 1:
                rate += f" ({min(spread):,.0f}–{max(spread):,.0f})"
            cpu = m.get("db_cpu_us_per_commit")
            print(
                f"| {v['label']} | {s['accounts']:,} | {s['clients']} | {rate} | {us(m['p50_us'])} "
                f"| {us(m['p99_us'])} | {m['conflicts_per_commit']:.2f} "
                f"| {f'{cpu:.0f} µs' if cpu is not None else '—'} "
                f"| {checks(bad, len(rs))} | {wrong:,} of {audits:,} |"
            )


def crash(crashes):
    print("| Configuration | Kills | Transfers acknowledged | In flight at the kill (landed) "
          "| Acknowledged, then lost | Accounts off | Ready again (median) | Violations |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|")
    for v in crashes:
        cs = v["cycles"]
        last = cs[-1]["check"]
        inflight = sum(c["check"]["in_flight"] for c in cs)
        landed = sum(c["check"]["in_flight_landed"] for c in cs)
        lost = max(c["check"]["acknowledged_missing"] for c in cs)
        off = max(c["check"]["accounts_off_acknowledged"] for c in cs)
        rec = statistics.median(c["recovery_ms"] for c in cs)
        nviol = sum(len(c["check"]["violations"]) for c in cs)
        print(
            f"| {v['label']} | {len(cs)} | {v['acknowledged_total']:,} | {inflight} ({landed}) "
            f"| {lost:,} | {off:,} | {rec:.0f} ms | {nviol} |"
        )
        if last["violations"]:
            print(f"|  | last check: {'; '.join(last['violations'])} |||||||")


def main():
    d = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    runs, crashes = load(d)
    if runs:
        throughput(runs)
        print()
    if crashes:
        crash(crashes)


if __name__ == "__main__":
    main()
