//! Run the same documents and the same queries against one database, and
//! write what was measured as JSON. `run.sh` starts each database in a
//! container with the same limits and calls this once per configuration.
//!
//! ```text
//! solidb-compare --engine solidb|postgres|mongo|arango|surreal --addr HOST:PORT
//!     [--password P] [--label NAME] [--typed] [--durable]
//!     [--docs 1000000] [--customers 100000] [--concurrency 1,32]
//!     [--warm 5] [--secs 20] [--runs 3] [--phases load,get,...]
//!     [--db-cgroup /sys/fs/cgroup/...] [--after-load CMD] [--after-all CMD]
//!     [--meta key=value ...] [--resume] --out results.json
//! ```

mod data;
mod engines;

use data::{customer, order, order_facts, order_key, window, Reference, Rng, STATUSES};
use engines::{Engine, Res, Settings, Table};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BATCH: u64 = 1_000;
const OP_TIMEOUT: Duration = Duration::from_secs(120);
/// The error a run stops with when a series failed: the server is probably
/// gone. main exits with 3 for it, which tells run.sh to restart the
/// database on the same data and `--resume`.
const ABORTED: &str = "series failed, stopping:";
const LOADERS: usize = 8;
/// Every phase, in the order they run: reads before writes, so that the
/// reference counts still hold while the reads are checked.
const PHASES: &[&str] = &[
    "load",
    "get",
    "by_customer",
    "recent",
    "join",
    "aggregate",
    "insert",
    "update",
];

struct Args {
    engine: String,
    label: String,
    settings: Settings,
    docs: u64,
    customers: u64,
    concurrency: Vec<usize>,
    warm: Duration,
    secs: Duration,
    runs: usize,
    phases: Vec<String>,
    db_cgroup: Option<String>,
    after_load: Option<String>,
    after_all: Option<String>,
    meta: Vec<(String, String)>,
    out: String,
    resume: bool,
}

fn parse_args() -> Args {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut kv: HashMap<String, Vec<String>> = HashMap::new();
    let mut flags = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].trim_start_matches("--").to_string();
        if matches!(a.as_str(), "typed" | "durable" | "resume") {
            flags.push(a);
            i += 1;
        } else {
            let v = argv
                .get(i + 1)
                .cloned()
                .unwrap_or_else(|| panic!("--{a} takes a value"));
            kv.entry(a).or_default().push(v);
            i += 2;
        }
    }
    let one = |k: &str| kv.get(k).and_then(|v| v.last().cloned());
    let num = |k: &str, d: u64| one(k).map(|v| v.parse().expect(k)).unwrap_or(d);
    let engine = one("engine").expect("--engine");
    Args {
        label: one("label").unwrap_or_else(|| engine.clone()),
        settings: Settings {
            addr: one("addr").expect("--addr"),
            password: one("password").unwrap_or_else(|| "bench".into()),
            durable: flags.iter().any(|f| f == "durable"),
            typed: flags.iter().any(|f| f == "typed"),
        },
        engine,
        docs: num("docs", 1_000_000),
        customers: num("customers", 100_000),
        concurrency: one("concurrency")
            .unwrap_or_else(|| "1,32".into())
            .split(',')
            .map(|c| c.trim().parse().expect("concurrency"))
            .collect(),
        warm: Duration::from_secs(num("warm", 5)),
        secs: Duration::from_secs(num("secs", 20)),
        runs: num("runs", 3) as usize,
        phases: one("phases")
            .map(|p| p.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_else(|| PHASES.iter().map(|s| s.to_string()).collect()),
        db_cgroup: one("db-cgroup"),
        after_load: one("after-load"),
        after_all: one("after-all"),
        meta: kv
            .get("meta")
            .map(|v| {
                v.iter()
                    .filter_map(|m| {
                        m.split_once('=')
                            .map(|(a, b)| (a.to_string(), b.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        out: one("out").expect("--out"),
        resume: flags.iter().any(|f| f == "resume"),
    }
}

/// Seconds of CPU this process has used (user + system), from /proc.
fn self_cpu() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let after = stat.rsplit_once(')').map(|(_, r)| r).unwrap_or("");
    let f: Vec<&str> = after.split_whitespace().collect();
    // Fields 14 and 15 of the whole line are the 12th and 13th after ")".
    let ticks = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    (ticks(11) + ticks(12)) / 100.0
}

/// Seconds of CPU the database's container has used, from its cgroup.
fn db_cpu(cgroup: &Option<String>) -> Option<f64> {
    let text = std::fs::read_to_string(format!("{}/cpu.stat", cgroup.as_ref()?)).ok()?;
    let usec = text.lines().find_map(|l| l.strip_prefix("usage_usec "))?;
    Some(usec.trim().parse::<f64>().ok()? / 1e6)
}

fn cgroup_memory(cgroup: &Option<String>) -> Value {
    let Some(cg) = cgroup else { return Value::Null };
    let stat = std::fs::read_to_string(format!("{cg}/memory.stat")).unwrap_or_default();
    let field = |name: &str| {
        stat.lines()
            .find_map(|l| l.strip_prefix(&format!("{name} ")))
            .and_then(|v| v.trim().parse::<u64>().ok())
    };
    let read = |f: &str| {
        std::fs::read_to_string(format!("{cg}/{f}"))
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
    };
    json!({
        "anon_bytes": field("anon"),
        "file_bytes": field("file"),
        "current_bytes": read("memory.current"),
        "peak_bytes": read("memory.peak"),
    })
}

fn load_avg() -> f64 {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|l| l.split_whitespace().next().and_then(|v| v.parse().ok()))
        .unwrap_or(f64::NAN)
}

fn shell(cmd: &Option<String>) -> Value {
    let Some(cmd) = cmd else { return Value::Null };
    match std::process::Command::new("sh").arg("-c").arg(cmd).output() {
        Ok(o) => Value::String(String::from_utf8_lossy(&o.stdout).trim().to_string()),
        Err(e) => Value::String(format!("error: {e}")),
    }
}

/// What one worker's operation came back with.
enum Outcome {
    Ok,
    /// The database answered, but not what the generator says it should.
    Wrong(String),
    Err(String),
}

fn check<T: PartialEq + std::fmt::Debug>(r: Res<T>, expected: T) -> Outcome {
    match r {
        Ok(v) if v == expected => Outcome::Ok,
        Ok(v) => Outcome::Wrong(format!("got {v:?}, expected {expected:?}")),
        Err(e) => Outcome::Err(e.to_string()),
    }
}

fn done(r: Res<()>) -> Outcome {
    match r {
        Ok(()) => Outcome::Ok,
        Err(e) => Outcome::Err(e.to_string()),
    }
}

fn percentiles(mut lat: Vec<u32>) -> Value {
    lat.sort_unstable();
    let pct = |p: f64| {
        if lat.is_empty() {
            0
        } else {
            lat[((lat.len() - 1) as f64 * p) as usize]
        }
    };
    json!({"p50_us": pct(0.50), "p90_us": pct(0.90), "p99_us": pct(0.99), "p999_us": pct(0.999), "max_us": lat.last().copied().unwrap_or(0)})
}

struct Ctx {
    reference: Arc<Reference>,
    docs: u64,
    customers: u64,
}

/// Run `op` on `n` connections for `dur`, each worker with its own random
/// stream, and return the series' figures. The connections go back to `pool`.
async fn timed<C, F, Fut>(
    pool: &mut Vec<C>,
    n: usize,
    dur: Duration,
    seed: u64,
    cg: &Option<String>,
    op: F,
) -> Value
where
    C: Send + 'static,
    F: Fn(C, Rng) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = (C, Rng, Outcome)> + Send,
{
    let stop = Arc::new(AtomicBool::new(false));
    let errors = Arc::new(AtomicU64::new(0));
    let wrong = Arc::new(AtomicU64::new(0));
    let first_problem: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let (cpu0, db0, load) = (self_cpu(), db_cpu(cg), load_avg());
    let start = Instant::now();
    let mut handles = Vec::new();
    for (w, mut conn) in pool.drain(..n).enumerate() {
        let (stop, errors, wrong, first, op) = (
            stop.clone(),
            errors.clone(),
            wrong.clone(),
            first_problem.clone(),
            op.clone(),
        );
        handles.push(tokio::spawn(async move {
            let mut rng = Rng::new(seed ^ ((w as u64 + 1) << 40));
            let mut lat: Vec<u32> = Vec::with_capacity(1 << 16);
            while !stop.load(Ordering::Relaxed) {
                let t = Instant::now();
                // A server that hangs instead of dying would otherwise keep
                // this worker, and the series, waiting forever. The
                // connection is lost with the request; the series then has
                // an error and the engine's run stops.
                let Ok((c, r, outcome)) = tokio::time::timeout(OP_TIMEOUT, op(conn, rng)).await
                else {
                    errors.fetch_add(1, Ordering::Relaxed);
                    first
                        .lock()
                        .unwrap()
                        .get_or_insert(format!("no answer within {OP_TIMEOUT:?}"));
                    return (None, lat);
                };
                conn = c;
                rng = r;
                let us = t.elapsed().as_micros().min(u32::MAX as u128) as u32;
                match outcome {
                    Outcome::Ok => lat.push(us),
                    Outcome::Wrong(m) => {
                        lat.push(us);
                        wrong.fetch_add(1, Ordering::Relaxed);
                        first.lock().unwrap().get_or_insert(m);
                    }
                    Outcome::Err(m) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                        first.lock().unwrap().get_or_insert(m);
                        // Do not spin on a server that has gone away.
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
            (Some(conn), lat)
        }));
    }
    tokio::time::sleep(dur).await;
    stop.store(true, Ordering::Relaxed);
    let mut lat = Vec::new();
    let mut back = Vec::new();
    for h in handles {
        let (c, l) = h.await.expect("worker");
        back.extend(c);
        lat.extend(l);
    }
    // Restore the pool in its original order, used connections first.
    back.append(pool);
    *pool = back;
    let secs = start.elapsed().as_secs_f64();
    let client_cpu = (self_cpu() - cpu0) / secs;
    let db_cores = db0.and_then(|d0| db_cpu(cg).map(|d1| (d1 - d0) / secs));
    let ops = lat.len();
    let mut v = json!({
        "ops": ops,
        "secs": secs,
        "ops_per_sec": (ops as f64 / secs).round(),
        "errors": errors.load(Ordering::Relaxed),
        "wrong": wrong.load(Ordering::Relaxed),
        "first_problem": *first_problem.lock().unwrap(),
        "client_cpu_cores": client_cpu,
        "db_cpu_cores": db_cores,
        "load_avg_before": load,
    });
    v.as_object_mut()
        .unwrap()
        .extend(percentiles(lat).as_object().unwrap().clone());
    v
}

/// Load documents `range` of a table in batches of 1,000 from `loaders`
/// connections. The batches are generated before the clock starts.
async fn load_range<E: Engine>(
    pool: &mut Vec<E::Conn>,
    loaders: usize,
    table: fn() -> Table,
    range: std::ops::Range<u64>,
    make: fn(u64, u64) -> Value,
    customers: u64,
) -> Res<Value> {
    let mut batches = Vec::new();
    let mut i = range.start;
    while i < range.end {
        let end = (i + BATCH).min(range.end);
        batches.push((i..end).map(|n| make(n, customers)).collect::<Vec<Value>>());
        i = end;
    }
    let count = range.end - range.start;
    let queue = Arc::new(Mutex::new(batches.into_iter()));
    let failed: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let start = Instant::now();
    let mut handles = Vec::new();
    for mut conn in pool.drain(..loaders) {
        let (queue, failed) = (queue.clone(), failed.clone());
        handles.push(tokio::spawn(async move {
            loop {
                let next = queue.lock().unwrap().next();
                let Some(batch) = next else { break };
                if let Err(e) = E::load(&mut conn, table(), batch).await {
                    failed.lock().unwrap().get_or_insert(e.to_string());
                    break;
                }
            }
            conn
        }));
    }
    let mut back = Vec::new();
    for h in handles {
        back.push(h.await.expect("loader"));
    }
    back.append(pool);
    *pool = back;
    if let Some(e) = failed.lock().unwrap().take() {
        return Err(e.into());
    }
    let secs = start.elapsed().as_secs_f64();
    Ok(
        json!({"docs": count, "loaders": loaders, "secs": secs, "docs_per_sec": (count as f64 / secs).round()}),
    )
}

fn order_doc(i: u64, customers: u64) -> Value {
    order(i, &order_key(i), customers)
}

fn customer_doc(c: u64, _: u64) -> Value {
    customer(c)
}

/// Check every read scenario against the reference before timing anything:
/// a wrong index or a wrong query stops the run instead of "winning".
async fn guard<E: Engine>(conn: &mut E::Conn, ctx: &Ctx) -> Res<()> {
    let mut r = Rng::new(7);
    for _ in 0..50 {
        let i = r.below(ctx.docs);
        let want = data::customer_key(order_facts(i, ctx.customers).customer);
        let got = E::get(conn, &order_key(i)).await?;
        if got.as_deref() != Some(want.as_str()) {
            return Err(format!("get {}: {got:?}, expected {want}", order_key(i)).into());
        }
        let c = r.below(ctx.customers);
        let got = E::by_customer(conn, &data::customer_key(c)).await?;
        if got != ctx.reference.per_customer[c as usize] as usize {
            return Err(format!(
                "by_customer c{c}: {got} rows, expected {}",
                ctx.reference.per_customer[c as usize]
            )
            .into());
        }
        let (lo, hi) = window(&mut r);
        let want = ctx.reference.recent_rows(lo, hi);
        let got = E::recent(conn, &data::iso(lo), &data::iso(hi)).await?;
        if got != want {
            return Err(format!("recent: {got} rows, expected {want}").into());
        }
        let got = E::join(conn, &data::iso(lo), &data::iso(hi)).await?;
        if got != want {
            return Err(format!("join: {got} named rows, expected {want}").into());
        }
    }
    check_groups(&E::aggregate(conn).await?, &ctx.reference)
}

fn check_groups(got: &engines::Groups, reference: &Reference) -> Res<()> {
    if got.len() != STATUSES.len() {
        return Err(format!(
            "aggregate: {} groups, expected {}",
            got.len(),
            STATUSES.len()
        )
        .into());
    }
    for (n, (status, _)) in STATUSES.iter().enumerate() {
        let (count, cents) = reference.per_status[n];
        let Some((_, c, total)) = got.iter().find(|g| g.0 == *status) else {
            return Err(format!("aggregate: no group {status}").into());
        };
        let want = cents as f64 / 100.0;
        if *c != count || ((total - want) / want).abs() > 1e-9 {
            return Err(
                format!("aggregate {status}: ({c}, {total}), expected ({count}, {want})").into(),
            );
        }
    }
    Ok(())
}

async fn run<E: Engine>(a: &Args) -> Res<Value> {
    let eng = E::open(&a.settings).await?;
    // `--resume`: the server was restarted on the same data after a series
    // took it down. Carry on in the same file, after what was measured.
    let mut out = if a.resume {
        let prev = std::fs::read(&a.out).map_err(|e| format!("--resume needs {}: {e}", a.out))?;
        serde_json::from_slice::<Value>(&prev)?
    } else {
        json!({
            "engine": a.engine,
            "label": a.label,
            "server_version": eng.version().await?,
            "settings": {"durable": a.settings.durable, "typed": a.settings.typed},
            "docs": a.docs,
            "customers": a.customers,
            "concurrency": a.concurrency,
            "warm_secs": a.warm.as_secs(),
            "measure_secs": a.secs.as_secs(),
            "runs": a.runs,
            "queries": E::queries(&a.settings).into_iter().map(|(k, q)| json!({"scenario": k, "query": q})).collect::<Vec<_>>(),
            "meta": a.meta.iter().cloned().collect::<HashMap<_, _>>(),
            "load": [],
            "series": [],
            "aborted": [],
        })
    };
    let save = |out: &Value| {
        std::fs::write(&a.out, serde_json::to_vec_pretty(out).unwrap()).expect("write results");
    };
    // (scenario, concurrency) pairs already measured, or that took the
    // server down: a resumed run skips both.
    let measured: std::collections::HashSet<(String, u64)> = ["series", "aborted"]
        .iter()
        .flat_map(|k| out[*k].as_array().cloned().unwrap_or_default())
        .map(|s| {
            (
                s["scenario"].as_str().unwrap_or_default().to_string(),
                s["concurrency"].as_u64().unwrap_or(0),
            )
        })
        .collect();
    let want = |p: &str| a.phases.iter().any(|x| x == p) && !(a.resume && p == "load");
    let max_conc = a
        .concurrency
        .iter()
        .copied()
        .max()
        .unwrap_or(1)
        .max(LOADERS);

    eprintln!("[{}] building the reference for {} orders", a.label, a.docs);
    let ctx = Arc::new(Ctx {
        reference: Arc::new(Reference::build(a.docs, a.customers)),
        docs: a.docs,
        customers: a.customers,
    });

    let mut pool = Vec::with_capacity(max_conc);
    if want("load") {
        eng.reset().await?;
    }
    for _ in 0..max_conc {
        pool.push(eng.connect().await?);
    }
    if want("load") {
        eprintln!("[{}] loading {} customers", a.label, a.customers);
        let c = load_range::<E>(
            &mut pool,
            LOADERS,
            || Table::Customers,
            0..a.customers,
            customer_doc,
            a.customers,
        )
        .await?;
        out["load"]
            .as_array_mut()
            .unwrap()
            .push(json!({"phase": "customers", "result": c}));
        let first = a.docs / 10;
        eprintln!("[{}] loading orders 0..{first} from 1 connection", a.label);
        let r1 = load_range::<E>(
            &mut pool,
            1,
            || Table::Orders,
            0..first,
            order_doc,
            a.customers,
        )
        .await?;
        eprintln!("[{}]   {}", a.label, r1);
        out["load"]
            .as_array_mut()
            .unwrap()
            .push(json!({"phase": "orders_1", "result": r1}));
        eprintln!(
            "[{}] loading orders {first}..{} from {LOADERS} connections",
            a.label, a.docs
        );
        let r8 = load_range::<E>(
            &mut pool,
            LOADERS,
            || Table::Orders,
            first..a.docs,
            order_doc,
            a.customers,
        )
        .await?;
        eprintln!("[{}]   {}", a.label, r8);
        out["load"]
            .as_array_mut()
            .unwrap()
            .push(json!({"phase": "orders_8", "result": r8}));
        // Let background flushes settle before measuring disk and memory.
        tokio::time::sleep(Duration::from_secs(10)).await;
        out["after_load"] =
            json!({"disk": shell(&a.after_load), "memory": cgroup_memory(&a.db_cgroup)});
        save(&out);
    }

    // The reference holds until the first write; a run resumed after one
    // (a write series that took the server down) has nothing left to read.
    let wrote = measured.iter().any(|(s, _)| s == "insert" || s == "update");
    let reads = ["get", "by_customer", "recent", "join", "aggregate"]
        .iter()
        .any(|p| want(p));
    if reads && !wrote {
        eprintln!("[{}] checking every read against the reference", a.label);
        guard::<E>(&mut pool[0], &ctx).await?;
    }

    // Every read runs on the loaded million documents, before any write:
    // inserts grow the collection by as much as an engine can insert in the
    // time given, which would hand the slower writers a smaller collection
    // to read.
    let mut plan: Vec<(&str, usize)> = Vec::new();
    for s in [
        "get",
        "by_customer",
        "aggregate",
        "recent",
        "join",
        "insert",
        "update",
    ] {
        let concs = if s == "aggregate" {
            vec![1]
        } else {
            a.concurrency.clone()
        };
        plan.extend(concs.into_iter().map(|c| (s, c)));
    }
    for (scenario, n) in plan {
        if !want(scenario) || measured.contains(&(scenario.to_string(), n as u64)) {
            continue;
        }
        {
            for run in 0..=a.runs {
                let seed = data::mix(
                    run as u64 * 1000 + n as u64,
                    scenario.len() as u64 * 7919 + scenario.as_bytes()[0] as u64,
                );
                let dur = if run == 0 { a.warm } else { a.secs };
                let ctx = ctx.clone();
                let v = match scenario {
                    "get" => {
                        timed(
                            &mut pool,
                            n,
                            dur,
                            seed,
                            &a.db_cgroup,
                            move |mut c, mut r| {
                                let ctx = ctx.clone();
                                async move {
                                    let i = r.below(ctx.docs);
                                    let want =
                                        data::customer_key(order_facts(i, ctx.customers).customer);
                                    let o = check(E::get(&mut c, &order_key(i)).await, Some(want));
                                    (c, r, o)
                                }
                            },
                        )
                        .await
                    }
                    "by_customer" => {
                        timed(
                            &mut pool,
                            n,
                            dur,
                            seed,
                            &a.db_cgroup,
                            move |mut c, mut r| {
                                let ctx = ctx.clone();
                                async move {
                                    let k = r.below(ctx.customers);
                                    let want = ctx.reference.per_customer[k as usize] as usize;
                                    let o = check(
                                        E::by_customer(&mut c, &data::customer_key(k)).await,
                                        want,
                                    );
                                    (c, r, o)
                                }
                            },
                        )
                        .await
                    }
                    "recent" | "join" => {
                        let join = scenario == "join";
                        timed(
                            &mut pool,
                            n,
                            dur,
                            seed,
                            &a.db_cgroup,
                            move |mut c, mut r| {
                                let ctx = ctx.clone();
                                async move {
                                    let (lo, hi) = window(&mut r);
                                    let want = ctx.reference.recent_rows(lo, hi);
                                    let (lo_s, hi_s) = (data::iso(lo), data::iso(hi));
                                    let got = if join {
                                        E::join(&mut c, &lo_s, &hi_s).await
                                    } else {
                                        E::recent(&mut c, &lo_s, &hi_s).await
                                    };
                                    (c, r, check(got, want))
                                }
                            },
                        )
                        .await
                    }
                    "aggregate" => {
                        timed(&mut pool, n, dur, seed, &a.db_cgroup, move |mut c, r| {
                            let ctx = ctx.clone();
                            async move {
                                let o = match E::aggregate(&mut c).await {
                                    Ok(g) => match check_groups(&g, &ctx.reference) {
                                        Ok(()) => Outcome::Ok,
                                        Err(e) => Outcome::Wrong(e.to_string()),
                                    },
                                    Err(e) => Outcome::Err(e.to_string()),
                                };
                                (c, r, o)
                            }
                        })
                        .await
                    }
                    "insert" => {
                        timed(
                            &mut pool,
                            n,
                            dur,
                            seed,
                            &a.db_cgroup,
                            move |mut c, mut r| {
                                let ctx = ctx.clone();
                                async move {
                                    // Fresh keys from a 64-bit stream; the content is
                                    // an existing order's.
                                    let key = format!("i{:016x}", r.next());
                                    let doc = order(r.below(ctx.docs), &key, ctx.customers);
                                    let o = done(E::insert_one(&mut c, doc).await);
                                    (c, r, o)
                                }
                            },
                        )
                        .await
                    }
                    "update" => {
                        timed(
                            &mut pool,
                            n,
                            dur,
                            seed,
                            &a.db_cgroup,
                            move |mut c, mut r| {
                                let ctx = ctx.clone();
                                async move {
                                    let key = order_key(r.below(ctx.docs));
                                    let status =
                                        STATUSES[r.below(STATUSES.len() as u64) as usize].0;
                                    let o = done(E::update_status(&mut c, &key, status).await);
                                    (c, r, o)
                                }
                            },
                        )
                        .await
                    }
                    _ => unreachable!(),
                };
                if v["errors"].as_u64() != Some(0) {
                    // A series with errors measures nothing; if the server went
                    // away (killed, crashed), every later one would only count
                    // more. Record it and stop this engine here.
                    eprintln!(
                        "[{}] {scenario} c={n} run {run}: {} errors after {} ops, first: {}; stopping",
                        a.label, v["errors"], v["ops"], v["first_problem"]
                    );
                    let mut entry = json!({"scenario": scenario, "concurrency": n, "run": run});
                    entry
                        .as_object_mut()
                        .unwrap()
                        .extend(v.as_object().unwrap().clone());
                    if !out["aborted"].is_array() {
                        out["aborted"] = json!([]);
                    }
                    out["aborted"].as_array_mut().unwrap().push(entry);
                    save(&out);
                    return Err(
                        format!("{ABORTED} {scenario} c={n}: {}", v["first_problem"]).into(),
                    );
                }
                if run == 0 {
                    continue;
                }
                eprintln!(
                    "[{}] {scenario} c={n} run {run}: {} ops/s p50 {}us p99 {}us errors {} wrong {}",
                    a.label, v["ops_per_sec"], v["p50_us"], v["p99_us"], v["errors"], v["wrong"]
                );
                if v["wrong"].as_u64() != Some(0) || v["errors"].as_u64() != Some(0) {
                    eprintln!("[{}]   first problem: {}", a.label, v["first_problem"]);
                }
                let mut entry = json!({"scenario": scenario, "concurrency": n, "run": run});
                entry
                    .as_object_mut()
                    .unwrap()
                    .extend(v.as_object().unwrap().clone());
                out["series"].as_array_mut().unwrap().push(entry);
                save(&out);
            }
        }
    }
    out["after_all"] = json!({"disk": shell(&a.after_all), "memory": cgroup_memory(&a.db_cgroup)});
    save(&out);
    Ok(out)
}

#[tokio::main]
async fn main() {
    let a = parse_args();
    let r = match a.engine.as_str() {
        "solidb" => run::<engines::soli::Soli>(&a).await,
        "postgres" => run::<engines::postgres::Postgres>(&a).await,
        "mongo" => run::<engines::mongo::Mongo>(&a).await,
        "arango" => run::<engines::arango::Arango>(&a).await,
        "surreal" => run::<engines::surreal::SurrealEngine>(&a).await,
        other => {
            eprintln!("unknown engine {other}");
            std::process::exit(2);
        }
    };
    if let Err(e) = r {
        eprintln!("[{}] FAILED: {e}", a.label);
        std::process::exit(if e.to_string().starts_with(ABORTED) {
            3
        } else {
            1
        });
    }
    eprintln!("[{}] done: {}", a.label, a.out);
}
