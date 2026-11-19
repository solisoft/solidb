//! solidb-vec: the same vectors and the same nearest-neighbour searches
//! against SoliDB and the engines of `vec.sh` (card #979).
//!
//! `N` vectors of `DIM` dimensions, drawn around `CLUSTERS` centres (as
//! embeddings cluster) and normalised, so that cosine similarity is a dot
//! product. Each carries an integer `id` and a category `cat` (id % 10) for
//! the filtered searches. Queries are drawn the same way, apart from the
//! data. The exact top-k of every query, plain and per category, is computed
//! here by brute force: recall@k is measured against it, and a faster search
//! with a worse recall is reported as such, not as a win.
//!
//! Series, for each `ef` asked: recall@k over every query, then requests/s
//! and latency at each concurrency; then the same with a filter on `cat`.

mod engines;

use engines::{Engine, Res, Settings};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BATCH: usize = 1_000;
const LOADERS: usize = 8;
const CLUSTERS: u64 = 100;
pub const CATEGORIES: u64 = 10;

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A standard normal number, from two hashes (Box-Muller).
fn normal(a: u64, b: u64) -> f32 {
    let u1 = ((mix(a) >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
    let u2 = ((mix(b) >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
    ((-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
}

fn normalised(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

/// Vector `i` of a stream (`stream` 0: the data; 1: the queries): its
/// cluster's centre plus noise, normalised. The same for every engine.
pub fn vector(stream: u64, i: u64, dim: usize) -> Vec<f32> {
    let cluster = mix(stream << 40 ^ i) % CLUSTERS;
    let seed = (stream + 2) << 48 ^ i << 8;
    normalised(
        (0..dim as u64)
            .map(|d| {
                let centre = normal(cluster << 20 ^ d, cluster << 20 ^ d ^ 0xABCD);
                centre + 0.6 * normal(seed ^ d << 1, seed ^ d << 1 ^ 1)
            })
            .collect(),
    )
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// The ids of the `k` data vectors nearest to `q` (largest dot product),
/// among those of category `cat` when given; ties by id.
fn exact(data: &[Vec<f32>], q: &[f32], k: usize, cat: Option<u64>) -> Vec<u64> {
    let mut best: Vec<(f32, u64)> = Vec::with_capacity(k + 1);
    for (i, v) in data.iter().enumerate() {
        let i = i as u64;
        if cat.is_some_and(|c| i % CATEGORIES != c) {
            continue;
        }
        let s = dot(q, v);
        if best.len() < k || s > best[best.len() - 1].0 {
            let at = best.partition_point(|&(b, j)| b > s || (b == s && j < i));
            best.insert(at, (s, i));
            best.truncate(k);
        }
    }
    best.into_iter().map(|(_, i)| i).collect()
}

struct Args {
    engine: String,
    settings: Settings,
    n: u64,
    dim: usize,
    queries: u64,
    k: usize,
    efs: Vec<usize>,
    concurrency: Vec<usize>,
    warm: u64,
    secs: u64,
    runs: usize,
    out: Option<String>,
}

fn parse_args() -> Args {
    let mut a = Args {
        engine: String::new(),
        settings: Settings {
            addr: String::new(),
            password: "bench".into(),
            m: 16,
            ef_construction: 200,
            quantization: None,
        },
        n: 100_000,
        dim: 128,
        queries: 500,
        k: 10,
        efs: vec![40, 100, 200],
        concurrency: vec![1, 8],
        warm: 3,
        secs: 10,
        runs: 3,
        out: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        let list = |s: String| -> Vec<usize> {
            s.split(',')
                .map(|c| c.parse().expect("a list of numbers"))
                .collect()
        };
        match flag.as_str() {
            "--engine" => a.engine = value(),
            "--addr" => a.settings.addr = value(),
            "--password" => a.settings.password = value(),
            "--n" => a.n = value().parse().expect("--n"),
            "--dim" => a.dim = value().parse().expect("--dim"),
            "--queries" => a.queries = value().parse().expect("--queries"),
            "--k" => a.k = value().parse().expect("--k"),
            "--ef" => a.efs = list(value()),
            "--m" => a.settings.m = value().parse().expect("--m"),
            "--ef-construction" => {
                a.settings.ef_construction = value().parse().expect("--ef-construction")
            }
            "--quantization" => a.settings.quantization = Some(value()),
            "--concurrency" => a.concurrency = list(value()),
            "--warm" => a.warm = value().parse().expect("--warm"),
            "--secs" => a.secs = value().parse().expect("--secs"),
            "--runs" => a.runs = value().parse().expect("--runs"),
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

fn percentiles(mut lat: Vec<u32>) -> Value {
    if lat.is_empty() {
        return json!({});
    }
    lat.sort_unstable();
    let at = |p: f64| lat[((lat.len() as f64 * p) as usize).min(lat.len() - 1)];
    json!({"p50_us": at(0.50), "p90_us": at(0.90), "p99_us": at(0.99), "max_us": lat[lat.len() - 1]})
}

/// One query and what the exact search returns for it.
struct Case {
    vector: Vec<f32>,
    cat: Option<u64>,
    want: Vec<u64>,
}

fn recall(want: &[u64], got: &[u64]) -> f64 {
    if want.is_empty() {
        return 1.0;
    }
    want.iter().filter(|w| got.contains(w)).count() as f64 / want.len() as f64
}

/// Recall@k over every case, one request at a time.
async fn measure_recall<E: Engine>(engine: &E, cases: &[Case], k: usize, ef: usize) -> Res<f64> {
    let mut c = engine.connect().await?;
    let mut total = 0.0;
    for case in cases {
        let got = E::search(&mut c, &case.vector, k, ef, case.cat).await?;
        if got.len() > k {
            return Err(format!("{} ids for a top-{k}", got.len()).into());
        }
        total += recall(&case.want, &got);
    }
    Ok(total / cases.len() as f64)
}

/// `clients` connections searching the cases in turn for `secs` seconds.
async fn timed<E: Engine>(
    engine: &Arc<E>,
    cases: &Arc<Vec<Case>>,
    k: usize,
    ef: usize,
    clients: usize,
    secs: u64,
) -> Res<(u64, f64, Vec<u32>)> {
    let stop = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicU64::new(0));
    let lats = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = Vec::new();
    for w in 0..clients {
        let (engine, cases, stop, done, lats) = (
            engine.clone(),
            cases.clone(),
            stop.clone(),
            done.clone(),
            lats.clone(),
        );
        tasks.push(tokio::spawn(async move {
            let mut c = engine.connect().await?;
            let mut mine = Vec::new();
            let mut i = w * 7919;
            while !stop.load(Ordering::Relaxed) {
                let case = &cases[i % cases.len()];
                i += 1;
                let began = Instant::now();
                E::search(&mut c, &case.vector, k, ef, case.cat).await?;
                mine.push(began.elapsed().as_micros().min(u32::MAX as u128) as u32);
                done.fetch_add(1, Ordering::Relaxed);
            }
            lats.lock().unwrap().extend(mine);
            Res::Ok(())
        }));
    }
    let began = Instant::now();
    tokio::time::sleep(Duration::from_secs(secs)).await;
    stop.store(true, Ordering::Relaxed);
    for t in tasks {
        t.await??;
    }
    let elapsed = began.elapsed().as_secs_f64();
    let lats = std::mem::take(&mut *lats.lock().unwrap());
    Ok((done.load(Ordering::Relaxed), elapsed, lats))
}

/// The data, and the exact answers of every query, plain and filtered,
/// before anything is timed.
fn truth(a: &Args) -> (Arc<Vec<Vec<f32>>>, Vec<Case>, Vec<Case>) {
    let began = Instant::now();
    let data: Arc<Vec<Vec<f32>>> = Arc::new((0..a.n).map(|i| vector(0, i, a.dim)).collect());
    let mut cases = Vec::new();
    let mut filtered = Vec::new();
    {
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let queries: Vec<Vec<f32>> = (0..a.queries).map(|i| vector(1, i, a.dim)).collect();
        let chunks: Vec<_> = queries
            .chunks(queries.len().div_ceil(threads).max(1))
            .map(|c| c.to_vec())
            .collect();
        let mut handles = Vec::new();
        let mut first = 0u64;
        for chunk in chunks {
            let data = data.clone();
            let k = a.k;
            let base = first;
            first += chunk.len() as u64;
            handles.push(std::thread::spawn(move || {
                chunk
                    .into_iter()
                    .enumerate()
                    .map(|(j, q)| {
                        let cat = (base + j as u64) % CATEGORIES;
                        let plain = exact(&data, &q, k, None);
                        let in_cat = exact(&data, &q, k, Some(cat));
                        (q, cat, plain, in_cat)
                    })
                    .collect::<Vec<_>>()
            }));
        }
        for h in handles {
            for (q, cat, plain, in_cat) in h.join().expect("ground truth thread") {
                cases.push(Case {
                    vector: q.clone(),
                    cat: None,
                    want: plain,
                });
                filtered.push(Case {
                    vector: q,
                    cat: Some(cat),
                    want: in_cat,
                });
            }
        }
    }
    eprintln!(
        "[vec] {} vectors of {} dims, {} queries, exact answers in {:.1} s",
        a.n,
        a.dim,
        a.queries,
        began.elapsed().as_secs_f64()
    );

    (data, cases, filtered)
}

/// Write the data, the queries and their exact answers for an engine that
/// runs outside this harness (LanceDB, from Python: `vec_lance.py`).
/// `data.f32` and `queries.f32` hold the vectors one after the other as
/// little-endian f32; `truth.json` the answers and each query's category.
fn dump(a: &Args) -> Res<()> {
    let dir = a
        .out
        .clone()
        .ok_or("--out <directory> is required with --engine dump")?;
    std::fs::create_dir_all(&dir)?;
    let (data, cases, filtered) = truth(a);
    let write = |name: &str, rows: &mut dyn Iterator<Item = &Vec<f32>>| -> Res<()> {
        let mut bytes = Vec::new();
        for v in rows {
            for x in v {
                bytes.extend_from_slice(&x.to_le_bytes());
            }
        }
        std::fs::write(format!("{dir}/{name}"), bytes)?;
        Ok(())
    };
    write("data.f32", &mut data.iter())?;
    write("queries.f32", &mut cases.iter().map(|c| &c.vector))?;
    let report = json!({
        "n": a.n, "dim": a.dim, "k": a.k, "categories": CATEGORIES,
        "cats": filtered.iter().map(|c| c.cat).collect::<Vec<_>>(),
        "plain": cases.iter().map(|c| &c.want).collect::<Vec<_>>(),
        "filtered": filtered.iter().map(|c| &c.want).collect::<Vec<_>>(),
    });
    std::fs::write(format!("{dir}/truth.json"), serde_json::to_string(&report)?)?;
    eprintln!(
        "[vec] dumped {} vectors and {} queries to {dir}",
        a.n,
        cases.len()
    );
    Ok(())
}

async fn run<E: Engine>(a: Args) -> Res<()> {
    let engine = Arc::new(E::open(&a.settings).await?);
    let version = engine.version().await?;
    eprintln!("[vec] {} {version}", a.engine);

    let (data, cases, filtered) = truth(&a);

    // Load, then build the index (some engines build it as they load).
    engine.reset(a.dim).await?;
    let began = Instant::now();
    let next = Arc::new(AtomicU64::new(0));
    let mut loaders = Vec::new();
    for _ in 0..LOADERS {
        let (engine, data, next) = (engine.clone(), data.clone(), next.clone());
        loaders.push(tokio::spawn(async move {
            let mut c = engine.connect().await?;
            loop {
                let lo = next.fetch_add(BATCH as u64, Ordering::Relaxed);
                if lo >= data.len() as u64 {
                    break;
                }
                let hi = (lo + BATCH as u64).min(data.len() as u64);
                let rows = (lo..hi).map(|i| (i, data[i as usize].clone())).collect();
                E::insert(&mut c, rows).await?;
            }
            Res::Ok(())
        }));
    }
    for l in loaders {
        l.await??;
    }
    let load_secs = began.elapsed().as_secs_f64();
    let began = Instant::now();
    engine.build_index().await?;
    let index_secs = began.elapsed().as_secs_f64();
    eprintln!("[vec] loaded in {load_secs:.1} s, index ready {index_secs:.1} s later");

    let cases = Arc::new(cases);
    let filtered = Arc::new(filtered);
    let mut series = Vec::new();
    for (name, set) in [("search", &cases), ("filtered", &filtered)] {
        for &ef in &a.efs {
            let r = measure_recall(&*engine, set, a.k, ef).await?;
            eprintln!("[vec] {name} ef={ef}: recall@{} {:.4}", a.k, r);
            for &clients in &a.concurrency {
                timed(&engine, set, a.k, ef, clients, a.warm).await?;
                for run in 0..a.runs {
                    let (ops, secs, lats) = timed(&engine, set, a.k, ef, clients, a.secs).await?;
                    let p = percentiles(lats);
                    eprintln!(
                        "[vec] {name} ef={ef} c={clients} run {run}: {:.0}/s p50 {} µs p99 {} µs",
                        ops as f64 / secs,
                        p["p50_us"],
                        p["p99_us"]
                    );
                    let mut row = json!({
                        "scenario": name, "ef": ef, "concurrency": clients, "run": run,
                        "recall": r, "ops": ops, "secs": secs, "ops_per_sec": ops as f64 / secs,
                    });
                    if let (Some(row), Some(p)) = (row.as_object_mut(), p.as_object()) {
                        row.extend(p.clone());
                    }
                    series.push(row);
                }
            }
        }
    }

    let report = json!({
        "engine": a.engine, "server_version": version,
        "n": a.n, "dim": a.dim, "queries": a.queries, "k": a.k,
        "m": a.settings.m, "ef_construction": a.settings.ef_construction,
        "quantization": a.settings.quantization,
        "load_secs": load_secs, "index_secs": index_secs,
        "queries_text": E::queries(),
        "series_results": series,
    });
    let text = serde_json::to_string_pretty(&report)?;
    match a.out {
        Some(path) => std::fs::write(path, text)?,
        None => println!("{text}"),
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Res<()> {
    let a = parse_args();
    match a.engine.as_str() {
        "dump" => dump(&a),
        "solidb" => run::<engines::soli::Soli>(a).await,
        "pgvector" => run::<engines::pgvector::PgVector>(a).await,
        other => Err(format!("unknown engine {other}").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_are_unit_length_and_repeatable() {
        let v = vector(0, 42, 64);
        assert!((dot(&v, &v) - 1.0).abs() < 1e-4);
        assert_eq!(v, vector(0, 42, 64));
        assert_ne!(v, vector(1, 42, 64));
    }

    #[test]
    fn the_exact_search_finds_a_vector_itself_first() {
        let data: Vec<Vec<f32>> = (0..500).map(|i| vector(0, i, 32)).collect();
        assert_eq!(exact(&data, &data[123], 5, None)[0], 123);
        let in_cat = exact(&data, &data[123], 5, Some(3));
        assert_eq!(in_cat[0], 123);
        assert!(in_cat.iter().all(|i| i % CATEGORIES == 3));
    }
}
