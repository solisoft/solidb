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
    emit("chart:insert-durable-32",
         chart("insert-durable-32", "Inserts fsynced before they are acknowledged, 32 clients (per second)",
               bars(du, "insert", 32, "ops"), ops, False, "One document per request."))


if __name__ == "__main__":
    main(sys.argv[1])
