//! solidb-graph: the same graph and the same questions against SoliDB,
//! Neo4j, Memgraph, ArangoDB and PostgreSQL (see graph.sh).
//!
//! Every answer is checked against the harness's own copy of the graph,
//! before and during timing: a series with a wrong answer is reported as
//! such, not as a throughput.

mod data;
mod engines;

use data::Graph;
use engines::{Engine, Res, Settings, Tally};
use serde_json::{json, Value};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BATCH: usize = 2_000;
const LOADERS: usize = 8;
const OP_TIMEOUT: Duration = Duration::from_secs(120);
/// Parameter sets prepared per scenario, with their expected answers.
const POOL: usize = 2_000;
const SCENARIOS: [&str; 6] = ["hop1", "hop2", "hop3", "filtered", "fof_bought", "shortest"];

struct Args {
    engine: String,
    settings: Settings,
    people: u32,
    products: u32,
    concurrency: Vec<usize>,
    warm: u64,
    secs: u64,
    runs: usize,
    only: Option<Vec<String>>,
    cgroup: Option<String>,
    out: Option<String>,
}

fn parse_args() -> Args {
    let mut a = Args {
        engine: String::new(),
        settings: Settings {
            addr: String::new(),
            password: "bench".into(),
            flavor: "neo4j".into(),
        },
        people: 1_000_000,
        products: 10_000,
        concurrency: vec![1, 8],
        warm: 5,
        secs: 20,
        runs: 3,
        only: None,
        cgroup: None,
        out: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--engine" => a.engine = value(),
            "--addr" => a.settings.addr = value(),
            "--password" => a.settings.password = value(),
            "--flavor" => a.settings.flavor = value(),
            "--people" => a.people = value().parse().expect("--people"),
            "--products" => a.products = value().parse().expect("--products"),
            "--concurrency" => {
                a.concurrency = value().split(',').map(|c| c.parse().expect("--concurrency")).collect()
            }
            "--warm" => a.warm = value().parse().expect("--warm"),
            "--secs" => a.secs = value().parse().expect("--secs"),
            "--runs" => a.runs = value().parse().expect("--runs"),
            "--only" => a.only = Some(value().split(',').map(str::to_string).collect()),
            "--cgroup" => a.cgroup = Some(value()),
            "--out" => a.out = Some(value()),
            other => panic!("unknown argument {other}"),
        }
    }
    assert!(!a.engine.is_empty() && !a.settings.addr.is_empty(), "--engine and --addr are required");
    a
}

/// CPU seconds the database's cgroup has used.
fn db_cpu(cgroup: &Option<String>) -> Option<f64> {
    let stat = std::fs::read_to_string(format!("{}/cpu.stat", cgroup.as_ref()?)).ok()?;
    let usec = stat.lines().find_map(|l| l.strip_prefix("usage_usec "))?;
    Some(usec.trim().parse::<f64>().ok()? / 1e6)
}

fn percentiles(mut lat: Vec<u32>) -> Value {
    if lat.is_empty() {
        return json!({});
    }
    lat.sort_unstable();
    let at = |p: f64| lat[((lat.len() as f64 * p) as usize).min(lat.len() - 1)];
    json!({"p50_us": at(0.50), "p90_us": at(0.90), "p99_us": at(0.99), "max_us": lat[lat.len() - 1]})
}

/// One prepared request and the answer the graph gives.
#[derive(Clone)]
enum Case {
    Hops { start: u32, depth: u32, want: Tally },
    Filtered { start: u32, year: i64, country: &'static str, want: Tally },
    Fof { start: u32, product: u32, want: Tally },
    Shortest { from: u32, to: u32, want: u64 },
}

fn cases(graph: &Graph, scenario: &str) -> Vec<Case> {
    let seed = scenario.bytes().fold(7u64, |s, b| s.wrapping_mul(31).wrapping_add(b as u64));
    let person = |i: u64| (data::pick(seed, i) % graph.people as u64) as u32;
    let mut out = Vec::with_capacity(POOL);
    let mut i = 0u64;
    while out.len() < POOL {
        i += 1;
        let start = person(i);
        let case = match scenario {
            "hop1" | "hop2" | "hop3" => {
                let depth = scenario[3..].parse().unwrap();
                Case::Hops { start, depth, want: data::tally(&graph.within(start, depth)) }
            }
            "filtered" => {
                let year = 2005 + (data::pick(seed ^ 1, i) % 15) as i64;
                let country = data::COUNTRIES[(data::pick(seed ^ 2, i) % 20) as usize];
                Case::Filtered { start, year, country, want: data::tally(&graph.filtered(start, year, country)) }
            }
            "fof_bought" => {
                // A product someone near bought, half of the time.
                let near: Vec<u32> = graph.within(start, 2).into_iter().collect();
                let product = match near.iter().find_map(|p| graph.bought[*p as usize].first()) {
                    Some(p) if i % 2 == 0 => *p,
                    _ => (data::pick(seed ^ 3, i) % graph.products as u64) as u32,
                };
                Case::Fof { start, product, want: data::tally(&graph.fof_bought(start, product)) }
            }
            "shortest" => {
                let hops = 2 + (i % 3) as u32;
                match graph.someone_at(start, hops, data::pick(seed ^ 4, i)) {
                    Some(to) => Case::Shortest { from: start, to, want: hops as u64 },
                    None => continue,
                }
            }
            other => panic!("unknown scenario {other}"),
        };
        out.push(case);
    }
    out
}

/// Run one case: `Ok(None)` when the answer is right, the mismatch otherwise.
async fn ask<E: Engine>(conn: &mut E::Conn, case: &Case) -> Res<Option<String>> {
    Ok(match case {
        Case::Hops { start, depth, want } => {
            let got = E::hops(conn, *start, *depth).await?;
            (got != *want).then(|| format!("hops({start}, {depth}): {got:?}, expected {want:?}"))
        }
        Case::Filtered { start, year, country, want } => {
            let got = E::filtered(conn, *start, *year, country).await?;
            (got != *want).then(|| format!("filtered({start}, {year}, {country}): {got:?}, expected {want:?}"))
        }
        Case::Fof { start, product, want } => {
            let got = E::fof_bought(conn, *start, *product).await?;
            (got != *want).then(|| format!("fof_bought({start}, {product}): {got:?}, expected {want:?}"))
        }
        Case::Shortest { from, to, want } => {
            let got = E::shortest(conn, *from, *to).await?;
            (got != Some(*want)).then(|| format!("shortest({from}, {to}): {got:?}, expected {want}"))
        }
    })
}

async fn timed<E: Engine>(
    pool: &mut Vec<E::Conn>,
    n: usize,
    dur: Duration,
    cases: Arc<Vec<Case>>,
    cg: &Option<String>,
) -> Value {
    let stop = Arc::new(AtomicBool::new(false));
    let (errors, wrong) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let first: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let db0 = db_cpu(cg);
    let start = Instant::now();
    let mut handles = Vec::new();
    for (w, mut conn) in pool.drain(..n).enumerate() {
        let (stop, errors, wrong, first, cases) =
            (stop.clone(), errors.clone(), wrong.clone(), first.clone(), cases.clone());
        handles.push(tokio::spawn(async move {
            let mut lat: Vec<u32> = Vec::with_capacity(1 << 14);
            let mut at = w * 97;
            while !stop.load(Ordering::Relaxed) {
                let case = &cases[at % cases.len()];
                at += 1;
                let t = Instant::now();
                let Ok(answer) = tokio::time::timeout(OP_TIMEOUT, ask::<E>(&mut conn, case)).await else {
                    errors.fetch_add(1, Ordering::Relaxed);
                    first.lock().unwrap().get_or_insert(format!("no answer within {OP_TIMEOUT:?}"));
                    return (None, lat);
                };
                let us = t.elapsed().as_micros().min(u32::MAX as u128) as u32;
                match answer {
                    Ok(None) => lat.push(us),
                    Ok(Some(mismatch)) => {
                        lat.push(us);
                        wrong.fetch_add(1, Ordering::Relaxed);
                        first.lock().unwrap().get_or_insert(mismatch);
                    }
                    Err(e) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                        first.lock().unwrap().get_or_insert(e.to_string());
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
    back.append(pool);
    *pool = back;
    let secs = start.elapsed().as_secs_f64();
    let ops = lat.len();
    let mut v = json!({
        "ops": ops,
        "secs": secs,
        "ops_per_sec": (ops as f64 / secs * 10.0).round() / 10.0,
        "errors": errors.load(Ordering::Relaxed),
        "wrong": wrong.load(Ordering::Relaxed),
        "first_problem": *first.lock().unwrap(),
        "db_cpu_cores": db0.and_then(|d0| db_cpu(cg).map(|d1| (d1 - d0) / secs)),
    });
    v.as_object_mut().unwrap().extend(percentiles(lat).as_object().unwrap().clone());
    v
}

/// Send `batches` from `LOADERS` connections, each batch in one request.
async fn load<E, T, F, Fut>(pool: &mut Vec<E::Conn>, batches: Vec<Vec<T>>, send: F) -> Res<()>
where
    E: Engine,
    T: Send + 'static,
    F: Fn(E::Conn, Vec<T>) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = (E::Conn, Res<()>)> + Send,
{
    let queue = Arc::new(Mutex::new(batches));
    let mut handles = Vec::new();
    for mut conn in pool.drain(..LOADERS.min(pool.len())) {
        let (queue, send) = (queue.clone(), send.clone());
        handles.push(tokio::spawn(async move {
            loop {
                let Some(batch) = queue.lock().unwrap().pop() else {
                    return (conn, Ok(()));
                };
                let (c, result) = send(conn, batch).await;
                conn = c;
                if result.is_err() {
                    return (conn, result);
                }
            }
        }));
    }
    let mut outcome = Ok(());
    for h in handles {
        let (conn, result) = h.await.expect("loader");
        pool.push(conn);
        if outcome.is_ok() {
            outcome = result;
        }
    }
    outcome
}

fn batches<T>(rows: Vec<T>) -> Vec<Vec<T>> {
    let mut out = Vec::new();
    let mut rows = rows.into_iter().peekable();
    while rows.peek().is_some() {
        out.push(rows.by_ref().take(BATCH).collect());
    }
    out
}

async fn run<E: Engine>(a: &Args) -> Res<Value> {
    eprintln!("generating {} people…", a.people);
    let graph = Arc::new(Graph::new(a.people, a.products));
    eprintln!("{} edges, {} purchases", graph.edges(), graph.purchases());

    let engine = E::open(&a.settings).await?;
    engine.reset().await?;
    let workers = a.concurrency.iter().copied().max().unwrap_or(1).max(LOADERS);
    let mut pool = Vec::with_capacity(workers);
    for _ in 0..workers {
        pool.push(engine.connect().await?);
    }

    let people = batches((0..a.people).map(|n| (n, data::country(n), data::age(n))).collect());
    let products = batches((0..a.products).collect());
    let knows = batches(
        graph.knows.iter().enumerate()
            .flat_map(|(from, to)| {
                let from = from as u32;
                to.iter().map(move |to| (from, *to, data::since(from, *to), data::weight(from, *to)))
            })
            .collect(),
    );
    let bought = batches(
        graph.bought.iter().enumerate()
            .flat_map(|(person, items)| items.iter().map(move |item| (person as u32, *item)))
            .collect(),
    );
    let started = Instant::now();
    load::<E, _, _, _>(&mut pool, people, |mut c, b| async move { let r = E::load_people(&mut c, b).await; (c, r) }).await?;
    load::<E, _, _, _>(&mut pool, products, |mut c, b| async move { let r = E::load_products(&mut c, b).await; (c, r) }).await?;
    let vertices_secs = started.elapsed().as_secs_f64();
    load::<E, _, _, _>(&mut pool, knows, |mut c, b| async move { let r = E::load_knows(&mut c, b).await; (c, r) }).await?;
    load::<E, _, _, _>(&mut pool, bought, |mut c, b| async move { let r = E::load_bought(&mut c, b).await; (c, r) }).await?;
    engine.after_load().await?;
    let load_secs = started.elapsed().as_secs_f64();
    eprintln!("loaded in {load_secs:.1} s");

    let mut series = Vec::new();
    for scenario in SCENARIOS {
        if a.only.as_ref().is_some_and(|only| !only.iter().any(|s| s == scenario)) {
            continue;
        }
        let cases = Arc::new(cases(&graph, scenario));
        // Checked before anything is timed: a wrong answer stops the scenario.
        let mut guard = Ok(None);
        for case in cases.iter().take(50) {
            guard = ask::<E>(&mut pool[0], case).await;
            if !matches!(guard, Ok(None)) {
                break;
            }
        }
        match guard {
            Ok(None) => {}
            Ok(Some(mismatch)) => {
                eprintln!("{scenario}: WRONG {mismatch}");
                series.push(json!({"scenario": scenario, "skipped": format!("wrong answer: {mismatch}")}));
                continue;
            }
            Err(e) => {
                eprintln!("{scenario}: ERROR {e}");
                series.push(json!({"scenario": scenario, "skipped": format!("error: {e}")}));
                continue;
            }
        }
        for &n in &a.concurrency {
            timed::<E>(&mut pool, n, Duration::from_secs(a.warm), cases.clone(), &a.cgroup).await;
            for run in 0..a.runs {
                let mut v = timed::<E>(&mut pool, n, Duration::from_secs(a.secs), cases.clone(), &a.cgroup).await;
                eprintln!(
                    "{scenario} c={n} run {run}: {}/s p50 {} µs p99 {} µs, {} wrong, {} errors",
                    v["ops_per_sec"], v["p50_us"], v["p99_us"], v["wrong"], v["errors"]
                );
                let o = v.as_object_mut().unwrap();
                o.insert("scenario".into(), json!(scenario));
                o.insert("concurrency".into(), json!(n));
                o.insert("run".into(), json!(run));
                series.push(v);
            }
        }
    }
    Ok(json!({
        "engine": a.engine,
        "flavor": a.settings.flavor,
        "server_version": engine.version().await.unwrap_or_else(|e| format!("unknown: {e}")),
        "people": a.people,
        "products": a.products,
        "edges": graph.edges(),
        "purchases": graph.purchases(),
        "load": {"secs": load_secs, "vertices_secs": vertices_secs,
                 "edges_per_sec": ((graph.edges() + graph.purchases()) as f64 / (load_secs - vertices_secs)).round()},
        "warm_secs": a.warm, "measure_secs": a.secs, "runs": a.runs,
        "queries": E::queries(&a.settings).into_iter()
            .map(|(scenario, query)| json!({"scenario": scenario, "query": query})).collect::<Vec<_>>(),
        "series": series,
    }))
}

#[tokio::main]
async fn main() {
    let a = parse_args();
    let result = match a.engine.as_str() {
        "solidb" => run::<engines::soli::Soli>(&a).await,
        "postgres" => run::<engines::postgres::Postgres>(&a).await,
        "arango" => run::<engines::arango::Arango>(&a).await,
        "bolt" => run::<engines::bolt::Bolt>(&a).await,
        other => panic!("unknown engine {other}"),
    };
    match result {
        Ok(v) => {
            let text = serde_json::to_string_pretty(&v).unwrap();
            match &a.out {
                Some(path) => std::fs::write(path, text).expect("write --out"),
                None => println!("{text}"),
            }
        }
        Err(e) => {
            eprintln!("failed: {e}");
            std::process::exit(1);
        }
    }
}
