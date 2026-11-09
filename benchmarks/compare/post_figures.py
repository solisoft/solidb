#!/usr/bin/env python3
"""The tables and charts of the blog post, from the JSON files run.sh wrote.

    python3 post_figures.py results/2026-11-09 > figures.html

Every figure on the post's page comes out of this script, unedited: the
medians and flags are report.py's. The output is a sequence of fragments,
each introduced by an HTML comment naming it (`<!-- table:get -->`,
`<!-- chart:get-32 -->`, ...), in the markup the docs site's posts use.
"""

import html
import sys

from report import cell, load

LABELS = {
    "solidb-solistore": "SoliDB",
    "postgres": "PostgreSQL jsonb",
    "postgres-typed": "PostgreSQL typed",
    "mongo": "MongoDB",
    "arango": "ArangoDB",
    "surreal": "SurrealDB",
    "solidb-solistore-durable": "SoliDB",
    "postgres-durable": "PostgreSQL",
    "mongo-durable": "MongoDB",
    "arango-durable": "ArangoDB",
    "surreal-durable": "SurrealDB",
}
EVENTUAL = ["solidb-solistore", "postgres", "postgres-typed", "mongo", "arango", "surreal"]
DURABLE = ["solidb-solistore-durable", "postgres-durable", "mongo-durable", "arango-durable",
           "surreal-durable"]

SOLIDB = "#f5a623"   # the site's accent, remapped per theme by application.css
OTHER = "#6b746c"    # muted ink
INK = "#ece6d6"      # text
INK_2 = "#9ba39c"    # secondary text
RULE = "#29322f"


def esc(s):
    return html.escape(str(s), quote=False)


def ops(v):
    return f"{v:,.0f}"


def us(v):
    if v is None:
        return "—"
    return f"{v / 1000:,.1f} ms" if v >= 1000 else f"{v:,.0f} µs"


def mb(n):
    return f"{int(n) / 1e6:,.0f} MB"


def table(head, rows, best=None):
    """rows: [[cell, ...]]; best: {column: row index} to set in bold."""
    best = best or {}
    out = ['<div class="post-table"><table><thead><tr>']
    out += [f"<th>{esc(h)}</th>" for h in head]
    out.append("</tr></thead><tbody>")
    for i, r in enumerate(rows):
        out.append("<tr>")
        for j, c in enumerate(r):
            v = esc(c)
            if best.get(j) == i:
                v = f"<strong>{v}</strong>"
            out.append(f"<td><pre>{v}</pre></td>")
        out.append("</tr>")
    out.append("</tbody></table></div>")
    return "".join(out)


def argbest(values, lower):
    pairs = [(v, i) for i, v in enumerate(values) if v is not None]
    if not pairs:
        return None
    return (min if lower else max)(pairs)[1]


def chart(name, title, bars, unit, lower_is_better, note):
    """Horizontal bars on one scale from 0. bars: [(label, value, is_solidb)]."""
    if not bars:
        return ""
    x0, width, top, row = 170, 400, 48, 34
    h = top + row * len(bars) + 34
    peak = max(v for _, v, _ in bars) or 1
    tid = f"cmp-{name}-t"
    out = [f'<figure class="post-figure"><svg viewBox="0 0 680 {h}" role="img" aria-labelledby="{tid}">',
           f'<title id="{tid}">{esc(title)}</title>',
           f'<text x="20" y="30" font-family="Inter, sans-serif" font-size="13" fill="{INK_2}">'
           f'{esc(title)}</text>',
           f'<line x1="{x0}" y1="{top - 4}" x2="{x0}" y2="{top + row * len(bars) - 6}" stroke="{RULE}"/>']
    for i, (label, v, mine) in enumerate(bars):
        y = top + i * row
        w = max(2, round(width * v / peak))
        out.append(f'<text x="20" y="{y + 16}" font-family="JetBrains Mono, monospace" font-size="12" '
                   f'fill="{INK}">{esc(label)}</text>')
        out.append(f'<rect x="{x0}" y="{y}" width="{w}" height="22" rx="3" fill="{SOLIDB if mine else OTHER}"/>')
        out.append(f'<text x="{x0 + w + 8}" y="{y + 16}" font-family="JetBrains Mono, monospace" '
                   f'font-size="12" fill="{INK}">{esc(unit(v))}</text>')
    better = "lower is better" if lower_is_better else "higher is better"
    out.append(f'<text x="20" y="{h - 12}" font-family="Inter, sans-serif" font-size="12" fill="{OTHER}">'
               f'Bars share one scale starting at 0; {better}. {esc(note)}</text>')
    out.append("</svg></figure>")
    return "".join(out)


def series_table(runs, scenario, concs=(1, 32)):
    head = ["Engine"]
    for c in concs:
        head += [f"c={c} ops/s", f"c={c} p50", f"c={c} p99"]
    rows, cols = [], {}
    for r in runs:
        line, any_cell = [LABELS.get(r["label"], r["label"])], False
        for k, c in enumerate(concs):
            x = cell(r, scenario, c)
            if x:
                any_cell = True
                flag = f" {x['flags']}" if x["flags"] else ""
                line += [ops(x["ops"]) + flag, us(x["p50"]), us(x["p99"])]
                for j, (v, lower) in enumerate([(x["ops"], False), (x["p50"], True), (x["p99"], True)]):
                    cols.setdefault(1 + 3 * k + j, []).append((v, lower))
            else:
                line += ["—"] * 3
                for j in range(3):
                    cols.setdefault(1 + 3 * k + j, []).append((None, False))
        if any_cell:
            rows.append(line)
    best = {j: argbest([v for v, _ in vs], vs[0][1]) for j, vs in cols.items()}
    return table(head, rows, best)


def bars(runs, scenario, conc, metric):
    out = []
    for r in runs:
        x = cell(r, scenario, conc)
        if x:
            out.append((LABELS.get(r["label"], r["label"]), x[metric], r["label"].startswith("solidb")))
    return out


def load_table(runs):
    head = ["Engine", "customers, 8 loaders", "orders, 1 loader", "orders, 8 loaders"]
    rows, cols = [], {}
    for r in runs:
        phases = {p["phase"]: p["result"]["docs_per_sec"] for p in r.get("load", [])}
        if not phases:
            continue
        vals = [phases.get("customers"), phases.get("orders_1"), phases.get("orders_8")]
        rows.append([LABELS.get(r["label"], r["label"])] + [ops(v) if v else "—" for v in vals])
        for j, v in enumerate(vals):
            cols.setdefault(1 + j, []).append(v)
    best = {j: argbest(vs, False) for j, vs in cols.items()}
    return table(head, rows, best)


def cpu_bars(runs, scenario, conc):
    """Database CPU time per request, in core-milliseconds: the median latency
    times the cores the container kept busy. A one-client series runs one
    request at a time, so this is what each request cost the server."""
    out = []
    for r in runs:
        x = cell(r, scenario, conc)
        if x and x["db_cores"] is not None:
            out.append((LABELS.get(r["label"], r["label"]), x["p50"] / 1000 * x["db_cores"],
                        r["label"].startswith("solidb")))
    return out


def load_bars(runs, phase):
    out = []
    for r in runs:
        for p in r.get("load", []):
            if p["phase"] == phase:
                out.append((LABELS.get(r["label"], r["label"]), p["result"]["docs_per_sec"],
                            r["label"].startswith("solidb")))
    return out


def aggregate_table(runs):
    head = ["Engine", "per query (p50)", "p99", "DB cores used"]
    rows, p50s = [], []
    for r in runs:
        x = cell(r, "aggregate", 1)
        if not x:
            continue
        rows.append([LABELS.get(r["label"], r["label"]), us(x["p50"]), us(x["p99"]),
                     "—" if x["db_cores"] is None else f"{x['db_cores']:.1f}"])
        p50s.append(x["p50"])
    return table(head, rows, {1: argbest(p50s, True)})


def footprint_table(runs):
    """Right after the load, when every engine holds the same 1.1M documents.
    Later figures depend on how many inserts each engine managed."""
    head = ["Engine", "data directory", "process memory", "page cache"]
    rows, cols = [], {}
    for r in runs:
        a = r.get("after_load") or {}
        mem = a.get("memory") or {}
        vals = [int(a["disk"]) if a.get("disk") else None, mem.get("anon_bytes"), mem.get("file_bytes")]
        rows.append([LABELS.get(r["label"], r["label"])] + [mb(v) if v else "—" for v in vals])
        for j, v in enumerate(vals[:2]):
            cols.setdefault(1 + j, []).append(v)
    best = {j: argbest(vs, True) for j, vs in cols.items()}
    return table(head, rows, best)


def queries_card(runs, scenario):
    """The text each engine was sent for `scenario`, as recorded in its result."""
    lines = []
    for r in runs:
        q = {x["scenario"]: x["query"] for x in r.get("queries", [])}.get(scenario)
        if q:
            lines.append(f'<span class="tok-com"># {esc(LABELS.get(r["label"], r["label"]))}</span>')
            lines.append(esc(q))
            lines.append("")
    body = "\n".join(lines).rstrip()
    return (f'<div class="code-card post-code"><div class="cc-bar"><span class="dotk"></span> '
            f'{esc(scenario)}, as sent</div><pre>{body}</pre></div>')


HOME_ENGINES = [("SoliDB", ["solidb-solistore"]), ("PostgreSQL", ["postgres", "postgres-typed"]),
                ("MongoDB", ["mongo"]), ("ArangoDB", ["arango"]), ("SurrealDB", ["surreal"])]
HOME_DURABLE = [("SoliDB", ["solidb-solistore-durable"]), ("PostgreSQL", ["postgres-durable"]),
                ("MongoDB", ["mongo-durable"]), ("ArangoDB", ["arango-durable"]), ("SurrealDB", ["surreal-durable"])]


def home_perf(runs):
    """The home page's charts: series at 32 clients (or 8 loaders), the
    full-scan aggregation at one client, and inserts fsynced before they are
    acknowledged; one bar per database. PostgreSQL is the faster of its two
    setups in each. Under an insert chart, which runs spread by more than
    10% and by how much."""
    by = {r["label"]: r for r in runs}

    def value(label, what):
        """(value, spread of the runs) of one configuration, or None."""
        r = by.get(label)
        if not r:
            return None
        if what == "load":
            v = next((p["result"]["docs_per_sec"] for p in r.get("load", []) if p["phase"] == "orders_8"), None)
            return None if v is None else (v, 1.0)
        x = cell(r, what, 1 if what == "aggregate" else 32)
        if not x:
            return None
        return (x["p50"], x["spread"]) if what == "aggregate" else (x["ops"], x["spread"])

    charts = [("get", "One order by its key", "requests/s, 32 clients", HOME_ENGINES),
              ("by_customer", "A customer&rsquo;s orders", "requests/s, 32 clients", HOME_ENGINES),
              ("join", "Join: 20 orders, customers", "requests/s, 32 clients", HOME_ENGINES),
              ("aggregate", "Full-scan COLLECT", "ms per query, lower is better", HOME_ENGINES),
              ("insert", "Insert one order", "requests/s, 32 clients", HOME_ENGINES),
              ("insert", "Insert one order, fsynced", "requests/s, 32 clients", HOME_DURABLE),
              ("update", "Update one field", "requests/s, 32 clients", HOME_ENGINES),
              ("load", "Load 1M orders", "documents/s, 8 connections", HOME_ENGINES)]
    out = ['<div class="shapes">']
    for what, title, unit, engines in charts:
        rows = []
        for name, labels in engines:
            vals = [v for v in (value(l, what) for l in labels) if v is not None]
            if vals:
                pick = min if what == "aggregate" else max
                rows.append((name, *pick(vals, key=lambda t: t[0])))
        if not rows:
            continue
        peak = max(v for _, v, _ in rows)
        out.append(f'<div class="shape"><div class="shape-head"><b>{title}</b><span>{unit}</span></div>')
        out.append(f'<ol class="perf-rows" aria-label="{title}, {unit}">')
        for name, v, _ in rows:
            cls = "perf-row is-solidb" if name == "SoliDB" else "perf-row"
            out.append(f'<li class="{cls}"><span>{name}</span><span class="perf-track">'
                       f'<span class="perf-bar" style="--w: {v / peak:.3f}" aria-hidden="true"></span>'
                       f'<span class="perf-val">{us(v) if what == "aggregate" else ops(v)}</span></span></li>')
        out.append("</ol>")
        wide = [(name, s) for name, _, s in rows if s > 1.10]
        if what == "insert" and wide:
            parts = [f"{round((s - 1) * 100)}% for {name}" for name, s in wide]
            listed = parts[0] if len(parts) == 1 else ", ".join(parts[:-1]) + " and " + parts[-1]
            out.append(f'<p class="perf-cap">Insert runs vary more than reads: the fastest of a database&rsquo;s '
                       f'three runs is ahead of its slowest by {listed}. The bars are the medians.</p>')
        out.append("</div>")
    out.append("</div>")
    return "\n".join(out)


def checked(runs):
    """One sentence on what the harness found wrong while timing: every timed
    read and aggregation is compared with the generator's answer."""
    n = sum(s["ops"] for r in runs for s in r["series"])
    bad = sum(s["wrong"] + s["errors"] for r in runs for s in r["series"])
    what = "none" if bad == 0 else f"{bad:,}"
    return f"Across the {n:,} timed requests measured here, {what} got a wrong answer or an error."


def emit(name, fragment):
    print(f"<!-- {name} -->")
    print(fragment)
    print()


def main(folder):
    runs = {r["label"]: r for r in load(folder)}
    ev = [runs[k] for k in EVENTUAL if k in runs]
    du = [runs[k] for k in DURABLE if k in runs]
    for sc in ["get", "by_customer", "recent", "join", "insert", "update"]:
        emit(f"table:{sc}", series_table(ev, sc))
    for sc in ["insert", "update"]:
        emit(f"table:{sc}-durable", series_table(du, sc))
    emit("table:aggregate", aggregate_table(ev))
    emit("table:load", load_table(ev))
    emit("table:load-durable", load_table(du))
    emit("table:footprint", footprint_table(ev))
    emit("queries:join", queries_card(ev, "join"))
    emit("queries:aggregate", queries_card(ev, "aggregate"))
    emit("text:checked", checked(ev + du))
    emit("home:perf", home_perf(ev + du))
    note = "1M orders, warm cache."
    emit("chart:load-8", chart("load-8", "Loading 1M orders from 8 connections (documents per second)",
                               load_bars(ev, "orders_8"), ops, False, "Batches of 1,000."))
    emit("chart:get-32", chart("get-32", "One order by key, 32 clients (requests per second)",
                               bars(ev, "get", 32, "ops"), ops, False, note))
    for sc, what in [("by_customer", "Every order of a customer"), ("recent", "The 20 most recent orders"),
                     ("join", "20 orders with their customers")]:
        emit(f"chart:{sc}-1", chart(f"{sc}-1", f"{what}, one client (median latency)",
                                    bars(ev, sc, 1, "p50"), us, True, note))
        emit(f"chart:{sc}-32", chart(f"{sc}-32", f"{what}, 32 clients (requests per second)",
                                     bars(ev, sc, 32, "ops"), ops, False, note))
    emit("chart:aggregate-1",
         chart("aggregate-1", "Count and sum by status over 1M orders, one client (median per query)",
               bars(ev, "aggregate", 1, "p50"), us, True, "Full scan, no index."))
    emit("chart:aggregate-cpu",
         chart("aggregate-cpu", "The same query: database CPU time per query (core-milliseconds)",
               cpu_bars(ev, "aggregate", 1), lambda v: f"{v:,.0f}", True, "Latency times cores used."))
    emit("chart:insert-durable-32",
         chart("insert-durable-32", "Inserts fsynced before they are acknowledged, 32 clients (per second)",
               bars(du, "insert", 32, "ops"), ops, False, "One document per request."))


if __name__ == "__main__":
    main(sys.argv[1])
