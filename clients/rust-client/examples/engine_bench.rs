//! Load a running SoliDB over the native driver and report throughput and
//! latency per phase, as one JSON line each. Written to compare storage
//! engines (RocksDB and solistore) behind the same server, so every query
//! runs with the query cache off.
//!
//! ```text
//! engine_bench <addr> <password> [docs] [workers] [phase_secs]
//! ```

use serde_json::{json, Value};
use solidb_client::{SoliDBClient, SoliDBClientBuilder};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const DB: &str = "_system";
const COLL: &str = "engine_bench";

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn doc(i: u64) -> Value {
    json!({
        "n": i,
        "group": i % 1000,
        "name": format!("user {i}"),
        "email": format!("user{i}@example.com"),
        "score": (i * 7919) % 10_000,
        "tags": ["alpha", "beta", if i % 2 == 0 { "even" } else { "odd" }],
        "bio": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor.",
    })
}

/// Connect, retrying: a server under load drops connections it cannot
/// dispatch in time ("driver dispatch channel full").
async fn connect(addr: &str, password: &str) -> SoliDBClient {
    let mut attempt = 0;
    loop {
        match SoliDBClientBuilder::new(addr)
            .auth(DB, "admin", password)
            .build()
            .await
        {
            Ok(client) => return client,
            Err(e) if attempt < 50 => {
                attempt += 1;
                eprintln!("connect attempt {attempt}: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => panic!("connect: {e}"),
        }
    }
}

#[derive(Default)]
struct Latencies(Vec<u32>);

impl Latencies {
    fn record(&mut self, since: Instant) {
        self.0
            .push(since.elapsed().as_micros().min(u32::MAX as u128) as u32);
    }
}

fn report(phase: &str, elapsed: Duration, mut lat: Vec<u32>) {
    lat.sort_unstable();
    let pct = |p: f64| -> u32 {
        if lat.is_empty() {
            0
        } else {
            lat[((lat.len() - 1) as f64 * p) as usize]
        }
    };
    let ops = lat.len() as f64;
    println!(
        "{}",
        json!({
            "phase": phase,
            "ops": lat.len(),
            "secs": elapsed.as_secs_f64(),
            "ops_per_sec": (ops / elapsed.as_secs_f64()).round(),
            "p50_us": pct(0.50),
            "p99_us": pct(0.99),
            "p999_us": pct(0.999),
        })
    );
}

/// Run `op` on every pooled connection until `secs` have passed; each
/// worker keeps its own random stream. The connections go back to the pool.
async fn timed<F, Fut>(phase: &str, pool: &mut Vec<SoliDBClient>, secs: u64, op: F)
where
    F: Fn(SoliDBClient, Rng) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = (SoliDBClient, Rng, bool)> + Send,
{
    // `PHASES=update,get` runs only those timed phases (the load always runs).
    if let Ok(only) = std::env::var("PHASES") {
        if !only.split(',').any(|p| p.trim() == phase) {
            return;
        }
    }
    // Mark the phase start for a profiler watching stderr.
    eprintln!("phase {phase} start");
    let stop = Arc::new(AtomicBool::new(false));
    let errors = Arc::new(AtomicU64::new(0));
    let start = Instant::now();
    let mut handles = Vec::new();
    for (w, mut client) in pool.drain(..).enumerate() {
        let (stop, errors, op) = (stop.clone(), errors.clone(), op.clone());
        handles.push(tokio::spawn(async move {
            let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ ((w as u64 + 1) << 32));
            let mut lat = Latencies::default();
            while !stop.load(Ordering::Relaxed) {
                let t = Instant::now();
                let (c, r, ok) = op(client, rng).await;
                client = c;
                rng = r;
                if ok {
                    lat.record(t);
                } else {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            (client, lat.0)
        }));
    }
    tokio::time::sleep(Duration::from_secs(secs)).await;
    stop.store(true, Ordering::Relaxed);
    let mut all = Vec::new();
    for h in handles {
        let (client, lat) = h.await.unwrap();
        pool.push(client);
        all.extend(lat);
    }
    let elapsed = start.elapsed();
    let errors = errors.load(Ordering::Relaxed);
    if errors > 0 {
        eprintln!("{phase}: {errors} errors");
    }
    report(phase, elapsed, all);
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let addr = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:6745".into());
    let password = args.get(2).cloned().unwrap_or_else(|| "admin".into());
    let docs: u64 = args
        .get(3)
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000);
    let workers: usize = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(64);
    let secs: u64 = args.get(5).and_then(|v| v.parse().ok()).unwrap_or(15);

    let mut admin = connect(&addr, &password).await;
    admin.delete_collection(DB, COLL).await.ok();
    admin
        .create_collection(DB, COLL, None)
        .await
        .expect("create collection");
    admin
        .create_index(DB, COLL, "by_group", vec!["group".into()], false, false)
        .await
        .expect("index group");
    admin
        .create_index(DB, COLL, "by_n", vec!["n".into()], false, false)
        .await
        .expect("index n");

    // One connection per worker, opened one at a time before any load and
    // kept for every phase.
    let mut pool = Vec::with_capacity(workers);
    for _ in 0..workers {
        pool.push(connect(&addr, &password).await);
    }

    // Load: every worker inserts its share of documents, one per request.
    {
        let start = Instant::now();
        let mut handles = Vec::new();
        for (w, mut client) in pool.drain(..).enumerate() {
            let w = w as u64;
            handles.push(tokio::spawn(async move {
                let mut lat = Latencies::default();
                let mut i = w;
                while i < docs {
                    let t = Instant::now();
                    client
                        .insert(DB, COLL, Some(&format!("d{i}")), doc(i))
                        .await
                        .expect("insert");
                    lat.record(t);
                    i += workers as u64;
                }
                (client, lat.0)
            }));
        }
        let mut all = Vec::new();
        for h in handles {
            let (client, lat) = h.await.unwrap();
            pool.push(client);
            all.extend(lat);
        }
        report("insert", start.elapsed(), all);
    }

    timed("get", &mut pool, secs, move |mut c, mut r| async move {
        let key = format!("d{}", r.below(docs));
        let ok = c.get(DB, COLL, &key).await.is_ok();
        (c, r, ok)
    })
    .await;

    timed("update", &mut pool, secs, move |mut c, mut r| async move {
        let key = format!("d{}", r.below(docs));
        let score = r.below(10_000);
        let ok = c
            .update(DB, COLL, &key, json!({ "score": score }), true)
            .await
            .is_ok();
        (c, r, ok)
    })
    .await;

    timed(
        "query_index",
        &mut pool,
        secs,
        move |mut c, mut r| async move {
            let mut vars = HashMap::new();
            vars.insert("g".to_string(), json!(r.below(1000)));
            let ok = c
                .query_with_cache(
                    DB,
                    "FOR d IN engine_bench FILTER d.group == @g LIMIT 10 RETURN d",
                    Some(vars),
                    false,
                )
                .await
                .is_ok();
            (c, r, ok)
        },
    )
    .await;

    // SoliDB's index range scan reads from the lower bound to the end of
    // the index and filters the upper bound afterwards (LIMIT does not stop
    // it either), so the lower bound sits in the last 2,000 documents: each
    // query walks at most 2,000 index entries and returns 100 documents.
    timed(
        "query_range",
        &mut pool,
        secs,
        move |mut c, mut r| async move {
            let tail = docs.min(2_000);
            let lo = docs - tail + r.below(tail.saturating_sub(100).max(1));
            let mut vars = HashMap::new();
            vars.insert("lo".to_string(), json!(lo));
            vars.insert("hi".to_string(), json!(lo + 100));
            let ok = c
                .query_with_cache(
                    DB,
                    "FOR d IN engine_bench FILTER d.n >= @lo AND d.n < @hi RETURN d",
                    Some(vars),
                    false,
                )
                .await
                .is_ok();
            (c, r, ok)
        },
    )
    .await;

    timed(
        "mixed_90_10",
        &mut pool,
        secs,
        move |mut c, mut r| async move {
            let key = format!("d{}", r.below(docs));
            let ok = if r.below(10) == 0 {
                c.update(DB, COLL, &key, json!({ "score": r.below(10_000) }), true)
                    .await
                    .is_ok()
            } else {
                c.get(DB, COLL, &key).await.is_ok()
            };
            (c, r, ok)
        },
    )
    .await;
}
