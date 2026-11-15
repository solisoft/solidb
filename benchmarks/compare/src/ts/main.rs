//! solidb-ts: the same time series and the same questions against SoliDB,
//! TimescaleDB and QuestDB (see ts.sh).
//!
//! `SERIES` series, one point every ten seconds over `HOURS` hours, loaded
//! in time order as an ingest would send them. Every value is a function
//! of its series and its instant, so the harness computes each expected
//! answer itself and checks every reply, before and during timing.

mod engines;

use engines::{Engine, Res, Settings};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The first instant, in milliseconds since the epoch (2025-10-09 08:53:20 UTC).
/// On a five-minute boundary, so that a window starting a whole number of
/// buckets after it holds whole buckets.
pub const T0: i64 = 1_760_000_100_000;
pub const STEP_MS: i64 = 10_000;
const BATCH: usize = 5_000;
const LOADERS: usize = 8;
const OP_TIMEOUT: Duration = Duration::from_secs(120);
const POOL: usize = 2_000;
const SCENARIOS: [&str; 5] = ["range", "bucket", "last", "window_all", "insert"];

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The value of `series` at step `i`: a whole number from 0 to 999, so that
/// sums are exact in a double. With `TS_VALUES=thirds` in the environment,
/// a third of that: values a double rounds, as measurements are, and sums
/// that round too.
pub fn value(series: u32, i: u64) -> f64 {
    static THIRDS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let whole = (mix(mix(series as u64) ^ i.wrapping_mul(0xD6E8_FEB8_6659_FD93)) % 1000) as f64;
    if *THIRDS.get_or_init(|| std::env::var("TS_VALUES").as_deref() == Ok("thirds")) {
        whole / 3.0
    } else {
        whole
    }
}

pub fn name(series: u32) -> String {
    format!("s{series:05}")
}

/// (series, milliseconds, value)
pub type Point = (u32, i64, f64);

struct Args {
    engine: String,
    settings: Settings,
    series: u32,
    hours: u64,
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
        },
        series: 1_000,
        hours: 24,
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
            "--series" => a.series = value().parse().expect("--series"),
            "--hours" => a.hours = value().parse().expect("--hours"),
            "--concurrency" => {
                a.concurrency = value()
                    .split(',')
                    .map(|c| c.parse().expect("--concurrency"))
                    .collect()
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
    assert!(
        !a.engine.is_empty() && !a.settings.addr.is_empty(),
        "--engine and --addr are required"
    );
    a
}

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

/// One prepared request and the answer the generator gives.
#[derive(Clone)]
enum Case {
    /// Count and sum of one series over `[lo, hi)`.
    Range {
        series: u32,
        lo: i64,
        hi: i64,
        want: (u64, f64),
    },
    /// Five-minute averages of one series over `[lo, hi)`: how many buckets
    /// and the sum of the averages.
    Bucket {
        series: u32,
        lo: i64,
        hi: i64,
        want: (u64, f64),
    },
    /// The last point of one series.
    Last { series: u32, want: (i64, f64) },
    /// Count and sum of every series over `[lo, hi)`.
    WindowAll { lo: i64, hi: i64, want: (u64, f64) },
    /// One point of a series nothing else reads.
    Insert,
}

struct Data {
    series: u32,
    steps: u64,
}

impl Data {
    fn tally(&self, series: std::ops::Range<u32>, lo: i64, hi: i64) -> (u64, f64) {
        let (from, to) = (((lo - T0) / STEP_MS) as u64, ((hi - T0) / STEP_MS) as u64);
        let (mut count, mut sum) = (0u64, 0f64);
        for s in series {
            for i in from..to.min(self.steps) {
                count += 1;
                sum += value(s, i);
            }
        }
        (count, sum)
    }

    fn buckets(&self, series: u32, lo: i64, hi: i64) -> (u64, f64) {
        let per = (300_000 / STEP_MS) as u64;
        let (from, to) = (((lo - T0) / STEP_MS) as u64, ((hi - T0) / STEP_MS) as u64);
        let (mut buckets, mut total) = (0u64, 0f64);
        let mut at = from;
        while at < to.min(self.steps) {
            let end = (at + per).min(to).min(self.steps);
            let sum: f64 = (at..end).map(|i| value(series, i)).sum();
            total += sum / (end - at) as f64;
            buckets += 1;
            at = end;
        }
        (buckets, total)
    }
}

fn cases(data: &Data, scenario: &str) -> Vec<Case> {
    let seed = scenario
        .bytes()
        .fold(11u64, |s, b| s.wrapping_mul(31).wrapping_add(b as u64));
    let pick = |i: u64, salt: u64| mix(mix(seed ^ salt) ^ i);
    // A window of `len` steps starting on a five-minute boundary.
    let window = |i: u64, len: u64| {
        let slots = (data.steps.saturating_sub(len) / 30).max(1);
        let lo = T0 + ((pick(i, 2) % slots) * 30) as i64 * STEP_MS;
        (lo, lo + len as i64 * STEP_MS)
    };
    (0..POOL as u64)
        .map(|i| {
            let series = (pick(i, 1) % data.series as u64) as u32;
            match scenario {
                "range" => {
                    let (lo, hi) = window(i, 360);
                    Case::Range {
                        series,
                        lo,
                        hi,
                        want: data.tally(series..series + 1, lo, hi),
                    }
                }
                "bucket" => {
                    let (lo, hi) = window(i, 2_160);
                    Case::Bucket {
                        series,
                        lo,
                        hi,
                        want: data.buckets(series, lo, hi),
                    }
                }
                "last" => {
                    let i = data.steps - 1;
                    Case::Last {
                        series,
                        want: (T0 + i as i64 * STEP_MS, value(series, i)),
                    }
                }
                "window_all" => {
                    let (lo, hi) = window(i, 60);
                    Case::WindowAll {
                        lo,
                        hi,
                        want: data.tally(0..data.series, lo, hi),
                    }
                }
                "insert" => Case::Insert,
                other => panic!("unknown scenario {other}"),
            }
        })
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * b.abs().max(1.0)
}

/// Points written by the `insert` scenario: series past the loaded ones.
static INSERTED: AtomicU64 = AtomicU64::new(0);

async fn ask<E: Engine>(conn: &mut E::Conn, case: &Case, series: u32) -> Res<Option<String>> {
    Ok(match case {
        Case::Range {
            series,
            lo,
            hi,
            want,
        } => {
            let got = E::range(conn, *series, *lo, *hi).await?;
            (got.0 != want.0 || !close(got.1, want.1))
                .then(|| format!("range({series}, {lo}, {hi}): {got:?}, expected {want:?}"))
        }
        Case::Bucket {
            series,
            lo,
            hi,
            want,
        } => {
            let got = E::bucket(conn, *series, *lo, *hi).await?;
            (got.0 != want.0 || !close(got.1, want.1))
                .then(|| format!("bucket({series}, {lo}, {hi}): {got:?}, expected {want:?}"))
        }
        Case::Last { series, want } => {
            let got = E::last(conn, *series).await?;
            (got.0 != want.0 || !close(got.1, want.1))
                .then(|| format!("last({series}): {got:?}, expected {want:?}"))
        }
        Case::WindowAll { lo, hi, want } => {
            let got = E::window_all(conn, *lo, *hi).await?;
            (got.0 != want.0 || !close(got.1, want.1))
                .then(|| format!("window_all({lo}, {hi}): {got:?}, expected {want:?}"))
        }
        Case::Insert => {
            let n = INSERTED.fetch_add(1, Ordering::Relaxed);
            // Series nothing reads, at instants after the loaded span.
            let point = (
                series + (n % 1000) as u32,
                T0 + 400 * 86_400_000 + n as i64,
                (n % 1000) as f64,
            );
            E::insert(conn, vec![point]).await?;
            None
        }
    })
}

async fn timed<E: Engine>(
    pool: &mut Vec<E::Conn>,
    n: usize,
    dur: Duration,
    cases: Arc<Vec<Case>>,
    series: u32,
    cg: &Option<String>,
) -> Value {
    let stop = Arc::new(AtomicBool::new(false));
    let (errors, wrong) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let first: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let db0 = db_cpu(cg);
    let start = Instant::now();
    let mut handles = Vec::new();
    for (w, mut conn) in pool.drain(..n).enumerate() {
        let (stop, errors, wrong, first, cases) = (
            stop.clone(),
            errors.clone(),
            wrong.clone(),
            first.clone(),
            cases.clone(),
        );
        handles.push(tokio::spawn(async move {
            let mut lat: Vec<u32> = Vec::with_capacity(1 << 14);
            let mut at = w * 97;
            while !stop.load(Ordering::Relaxed) {
                let case = &cases[at % cases.len()];
                at += 1;
                let t = Instant::now();
                let Ok(answer) =
                    tokio::time::timeout(OP_TIMEOUT, ask::<E>(&mut conn, case, series)).await
                else {
                    errors.fetch_add(1, Ordering::Relaxed);
                    first
                        .lock()
                        .unwrap()
                        .get_or_insert(format!("no answer within {OP_TIMEOUT:?}"));
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
    v.as_object_mut()
        .unwrap()
        .extend(percentiles(lat).as_object().unwrap().clone());
    v
}

async fn run<E: Engine>(a: &Args) -> Res<Value> {
    let data = Data {
        series: a.series,
        steps: a.hours * 3_600_000 / STEP_MS as u64,
    };
    let points = data.series as u64 * data.steps;
    eprintln!(
        "{} series x {} steps = {points} points",
        data.series, data.steps
    );

    let engine = E::open(&a.settings).await?;
    engine.reset().await?;
    let workers = a
        .concurrency
        .iter()
        .copied()
        .max()
        .unwrap_or(1)
        .max(LOADERS);
    let mut pool = Vec::with_capacity(workers);
    for _ in 0..workers {
        pool.push(engine.connect().await?);
    }

    // In time order, as an ingest sends them: every series' point of one
    // instant, then the next instant. Each loader takes the next batch.
    let next = Arc::new(AtomicU64::new(0));
    let (series, steps) = (data.series as u64, data.steps);
    let started = Instant::now();
    let mut handles = Vec::new();
    for mut conn in pool.drain(..LOADERS.min(workers)) {
        let next = next.clone();
        handles.push(tokio::spawn(async move {
            loop {
                let from = next.fetch_add(BATCH as u64, Ordering::Relaxed);
                if from >= series * steps {
                    return (conn, Ok(()));
                }
                let batch: Vec<Point> = (from..(from + BATCH as u64).min(series * steps))
                    .map(|at| {
                        let (i, s) = (at / series, (at % series) as u32);
                        (s, T0 + i as i64 * STEP_MS, value(s, i))
                    })
                    .collect();
                if let Err(e) = E::insert(&mut conn, batch).await {
                    return (conn, Err(e));
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
    outcome?;
    engine.after_load(points).await?;
    let load_secs = started.elapsed().as_secs_f64();
    eprintln!(
        "loaded in {load_secs:.1} s ({:.0} points/s)",
        points as f64 / load_secs
    );

    let mut results = Vec::new();
    for scenario in SCENARIOS {
        if a.only
            .as_ref()
            .is_some_and(|only| !only.iter().any(|s| s == scenario))
        {
            continue;
        }
        let cases = Arc::new(cases(&data, scenario));
        let mut guard = Ok(None);
        for case in cases.iter().take(30) {
            guard = ask::<E>(&mut pool[0], case, data.series).await;
            if !matches!(guard, Ok(None)) {
                break;
            }
        }
        match guard {
            Ok(None) => {}
            Ok(Some(mismatch)) => {
                eprintln!("{scenario}: WRONG {mismatch}");
                results.push(
                    json!({"scenario": scenario, "skipped": format!("wrong answer: {mismatch}")}),
                );
                continue;
            }
            Err(e) => {
                eprintln!("{scenario}: ERROR {e}");
                results.push(json!({"scenario": scenario, "skipped": format!("error: {e}")}));
                continue;
            }
        }
        for &n in &a.concurrency {
            timed::<E>(
                &mut pool,
                n,
                Duration::from_secs(a.warm),
                cases.clone(),
                data.series,
                &a.cgroup,
            )
            .await;
            for run in 0..a.runs {
                let mut v = timed::<E>(
                    &mut pool,
                    n,
                    Duration::from_secs(a.secs),
                    cases.clone(),
                    data.series,
                    &a.cgroup,
                )
                .await;
                eprintln!(
                    "{scenario} c={n} run {run}: {}/s p50 {} µs p99 {} µs, {} wrong, {} errors",
                    v["ops_per_sec"], v["p50_us"], v["p99_us"], v["wrong"], v["errors"]
                );
                let o = v.as_object_mut().unwrap();
                o.insert("scenario".into(), json!(scenario));
                o.insert("concurrency".into(), json!(n));
                o.insert("run".into(), json!(run));
                results.push(v);
            }
        }
    }
    Ok(json!({
        "engine": a.engine,
        "server_version": engine.version().await.unwrap_or_else(|e| format!("unknown: {e}")),
        "series": data.series, "hours": a.hours, "points": points,
        "load": {"secs": load_secs, "points_per_sec": (points as f64 / load_secs).round()},
        "warm_secs": a.warm, "measure_secs": a.secs, "runs": a.runs,
        "queries": E::queries().into_iter()
            .map(|(scenario, query)| json!({"scenario": scenario, "query": query})).collect::<Vec<_>>(),
        "series_results": results,
    }))
}

#[tokio::main]
async fn main() {
    let a = parse_args();
    let result = match a.engine.as_str() {
        "solidb" => run::<engines::soli::Soli>(&a).await,
        "timescale" => run::<engines::timescale::Timescale>(&a).await,
        "questdb" => run::<engines::questdb::QuestDb>(&a).await,
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
