//! Write-path benchmarks shaped like `benchmarks/compare` (card #774): a bulk
//! load in batches of 1,000, single-document inserts, and single-field
//! updates of an indexed field, each sent as the native driver sends them.
//!
//! Every operation goes through what a driver connection pays on the server:
//! the MessagePack frame is decoded into a `Command`, the document handler
//! runs (`driver::handlers::document`, the same functions the connection loop
//! calls), and the response is encoded. Only the socket and the Tokio task are
//! missing. The frames are built before the clock starts, so the client's work
//! is not counted.
//!
//! The orders are the harness's: about 500 bytes of JSON each, with three
//! single-field indexes (`customer_id`, `created_at`, `status`) created before
//! the load, as the driver's CreateIndex creates them (sparse). The replication
//! log is opened and deferred, as on a server started without `--peer`.
//!
//! Run with:
//!   cargo test --profile ci --features bench-tests,engine-solistore \
//!     --config 'profile.ci.package.solidb.opt-level=3' \
//!     --test benchmark_writes -- --ignored --nocapture
//! (the `ci` profile builds this crate at opt-level 1, which is no way to
//! measure it). `SOLIDB_ENGINE` picks the engine as it does for a server.
//!
//! Each phase prints operations per second and the CPU time per operation,
//! of the connection threads and of the whole process (the engine's flush
//! and compaction threads included). On a shared machine the wall time
//! moves with everyone else's load and the CPU figures much less; `perf
//! record -e instructions:u` with `--comm bench-conn` gives figures that do
//! not move at all.
//!
//! Sizes: `BENCH_DOCS` (default 1,000,000 loaded), `BENCH_OPS` (200,000
//! inserts and as many updates per concurrency), `BENCH_THREADS` (8, the
//! harness gives each database eight hardware threads).

use serde_json::{json, Value};
use solidb::driver::handlers::{document, index, DriverHandler};
use solidb::driver::protocol::{decode_command, encode_command, encode_response, Command};
use solidb::storage::StorageEngine;
use solidb::sync::log::SyncLog;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const DB: &str = "bench";
const COLL: &str = "orders";
const BATCH: usize = 1_000;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

// ------------------------------------------------- the harness's documents

fn mix(seed: u64, i: u64) -> u64 {
    let mut z = seed ^ i.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

const STATUSES: [&str; 5] = ["delivered", "shipped", "pending", "cancelled", "returned"];
const COUNTRIES: [&str; 8] = ["FR", "DE", "ES", "IT", "GB", "NL", "US", "JP"];
const CITIES: [&str; 4] = ["Lyon", "Paris", "Berlin", "Madrid"];
const TAGS: [&str; 6] = ["gift", "express", "promo", "b2b", "fragile", "eco"];
const CHANNELS: [&str; 4] = ["web", "mobile", "store", "phone"];

fn order_key(i: u64) -> String {
    format!("o{i:07}")
}

fn order(i: u64) -> Value {
    let h = mix(4, i);
    let secs = 1_704_067_200 + mix(3, i) % (2 * 365 * 86_400);
    let created = chrono::DateTime::from_timestamp(secs as i64, 0)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let items: Vec<Value> = (0..3)
        .map(|k| {
            let h = mix(10 + k, i);
            json!({"sku": format!("SKU-{:05}", h % 50_000), "qty": 1 + (h >> 20) % 4,
                   "price": (199 + (h >> 32) % 9_800) as f64 / 100.0})
        })
        .collect();
    json!({
        "customer_id": format!("c{:05}", mix(1, i) % 100_000),
        "status": STATUSES[(mix(2, i) % 5) as usize],
        "amount": (mix(5, i) % 100_000) as f64 / 100.0,
        "currency": "EUR",
        "created_at": created,
        "country": COUNTRIES[(h % 8) as usize],
        "channel": CHANNELS[(h >> 8) as usize % 4],
        "tags": [TAGS[(h >> 12) as usize % 6], TAGS[(h >> 16) as usize % 6], TAGS[(h >> 20) as usize % 6]],
        "items": items,
        "address": {
            "street": format!("{} rue des Lilas", 1 + (h >> 24) % 180),
            "city": CITIES[(h >> 36) as usize % 4],
            "zip": format!("{:05}", 1000 + (h >> 40) % 94_000),
        },
        "note": "Leave the parcel with the concierge if nobody answers the door.",
    })
}

// ------------------------------------------------------------ the server

struct Server {
    storage: Arc<StorageEngine>,
    log: Arc<SyncLog>,
    _tmp: TempDir,
}

impl Server {
    fn start() -> Self {
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(StorageEngine::new(tmp.path().join("data")).unwrap());
        let log = SyncLog::new(
            "bench-node".to_string(),
            tmp.path().to_str().unwrap(),
            10_000,
        )
        .unwrap();
        log.defer_until_needed();
        storage.create_database(DB.to_string()).unwrap();
        let db = storage.get_database(DB).unwrap();
        db.create_collection(COLL.to_string(), None).unwrap();
        let coll = db.get_collection(COLL).unwrap();
        for field in ["customer_id", "created_at", "status"] {
            index::create_persistent_index(
                &coll,
                format!("idx_{field}"),
                vec![field.to_string()],
                false,
            );
        }
        Server {
            storage,
            log: Arc::new(log),
            _tmp: tmp,
        }
    }

    fn connection(&self) -> DriverHandler {
        DriverHandler::new(self.storage.clone(), Some(self.log.clone()))
    }
}

/// What the connection loop does with one frame: decode, run, encode.
fn serve(handler: &DriverHandler, frame: &[u8]) -> usize {
    let command: Command = decode_command(&frame[4..]).unwrap();
    let response = match command {
        Command::Insert {
            database,
            collection,
            key,
            document,
        } => document::handle_insert(handler, database, collection, key, document),
        Command::Update {
            database,
            collection,
            key,
            document,
            merge,
        } => document::handle_update(handler, database, collection, key, document, merge),
        Command::BulkInsert {
            database,
            collection,
            documents,
        } => document::handle_bulk_insert(handler, database, collection, documents),
        other => panic!("not a write: {other:?}"),
    };
    let bytes = encode_response(&response).unwrap();
    assert!(
        matches!(response, solidb::driver::protocol::Response::Ok { .. }),
        "{response:?}"
    );
    bytes.len()
}

/// CPU time of the whole process (every thread, the engine's background
/// ones included), or of the calling thread.
fn cpu_time(clock: libc::clockid_t) -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid timespec for the call to fill.
    unsafe { libc::clock_gettime(clock, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// What one phase cost: wall time, the connections' CPU time, and the CPU
/// time of the whole process. The CPU figures hold still on a busy machine,
/// where the wall time does not.
struct Cost {
    wall: Duration,
    conn_cpu: Duration,
    proc_cpu: Duration,
}

/// Run `frames` split across `threads` connections.
fn run(server: &Server, frames: Vec<Vec<u8>>, threads: usize) -> Cost {
    let mut per_thread: Vec<Vec<Vec<u8>>> = (0..threads).map(|_| Vec::new()).collect();
    for (i, f) in frames.into_iter().enumerate() {
        per_thread[i % threads].push(f);
    }
    let barrier = Arc::new(std::sync::Barrier::new(threads + 1));
    let handles: Vec<_> = per_thread
        .into_iter()
        .map(|frames| {
            let handler = server.connection();
            let barrier = barrier.clone();
            // Named, so a profile can keep the connections' work apart from
            // the frames being built: `perf report --comm bench-conn`.
            std::thread::Builder::new()
                .name("bench-conn".to_string())
                .spawn(move || {
                    barrier.wait();
                    let start = cpu_time(libc::CLOCK_THREAD_CPUTIME_ID);
                    for f in &frames {
                        serve(&handler, f);
                    }
                    cpu_time(libc::CLOCK_THREAD_CPUTIME_ID) - start
                })
                .unwrap()
        })
        .collect();
    barrier.wait();
    let start = Instant::now();
    let proc_start = cpu_time(libc::CLOCK_PROCESS_CPUTIME_ID);
    let conn_cpu = handles.into_iter().map(|h| h.join().unwrap()).sum();
    Cost {
        wall: start.elapsed(),
        conn_cpu,
        proc_cpu: cpu_time(libc::CLOCK_PROCESS_CPUTIME_ID) - proc_start,
    }
}

fn bulk_frames(range: std::ops::Range<u64>) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut i = range.start;
    while i < range.end {
        let end = (i + BATCH as u64).min(range.end);
        let documents = (i..end)
            .map(|n| {
                let mut d = order(n);
                d["_key"] = json!(order_key(n));
                d
            })
            .collect();
        frames.push(
            encode_command(&Command::BulkInsert {
                database: DB.to_string(),
                collection: COLL.to_string(),
                documents,
            })
            .unwrap(),
        );
        i = end;
    }
    frames
}

fn insert_frames(range: std::ops::Range<u64>) -> Vec<Vec<u8>> {
    range
        .map(|n| {
            encode_command(&Command::Insert {
                database: DB.to_string(),
                collection: COLL.to_string(),
                key: Some(order_key(n)),
                document: order(n),
            })
            .unwrap()
        })
        .collect()
}

fn update_frames(count: usize, docs: u64, seed: u64) -> Vec<Vec<u8>> {
    (0..count as u64)
        .map(|n| {
            let r = mix(seed, n);
            encode_command(&Command::Update {
                database: DB.to_string(),
                collection: COLL.to_string(),
                key: order_key(r % docs),
                document: json!({"status": STATUSES[((r >> 32) % 5) as usize]}),
                merge: true,
            })
            .unwrap()
        })
        .collect()
}

fn report(label: &str, ops: usize, cost: Cost) {
    let per_sec = ops as f64 / cost.wall.as_secs_f64();
    let us = |d: Duration| d.as_secs_f64() * 1e6 / ops as f64;
    println!(
        "{label:<32} {ops:>8} ops {per_sec:>9.0}/s  CPU/op: connections {:>6.2} µs, process {:>6.2} µs",
        us(cost.conn_cpu),
        us(cost.proc_cpu)
    );
}

/// The whole card: load, then inserts and updates at 1 and N threads.
#[test]
#[ignore]
fn benchmark_writes() {
    let docs = env_usize("BENCH_DOCS", 1_000_000) as u64;
    let ops = env_usize("BENCH_OPS", 200_000);
    let threads = env_usize("BENCH_THREADS", 8);
    let server = Server::start();

    // Load: the first 10% from one connection, the rest from `threads`.
    let first = docs / 10;
    let frames = bulk_frames(0..first);
    let cost = run(&server, frames, 1);
    report("bulk load, 1 connection", first as usize, cost);

    let frames = bulk_frames(first..docs);
    let cost = run(&server, frames, threads);
    report(
        &format!("bulk load, {threads} connections"),
        (docs - first) as usize,
        cost,
    );

    let coll = server
        .storage
        .get_database(DB)
        .unwrap()
        .get_collection(COLL)
        .unwrap();
    assert_eq!(coll.count(), docs as usize);

    // Inserts, then updates, at 1 and `threads` connections.
    let mut next = docs;
    for t in [1, threads] {
        let n = if t == 1 { ops / 4 } else { ops };
        let frames = insert_frames(next..next + n as u64);
        next += n as u64;
        let cost = run(&server, frames, t);
        report(&format!("insert one, {t} connections"), n, cost);
    }
    for t in [1, threads] {
        let n = if t == 1 { ops / 4 } else { ops };
        let frames = update_frames(n, docs, 77 + t as u64);
        let cost = run(&server, frames, t);
        report(&format!("update status, {t} connections"), n, cost);
    }
    assert_eq!(coll.count(), next as usize);
}

/// Only the single-document paths, on a smaller collection: quick to rerun
/// under a profiler.
#[test]
#[ignore]
fn benchmark_single_writes() {
    let docs = env_usize("BENCH_DOCS", 100_000) as u64;
    let ops = env_usize("BENCH_OPS", 200_000);
    let threads = env_usize("BENCH_THREADS", 8);
    // `BENCH_ONLY=insert` or `update` runs one of the two.
    let only = std::env::var("BENCH_ONLY").unwrap_or_default();
    let server = Server::start();
    run(&server, bulk_frames(0..docs), threads);

    if only != "update" {
        let frames = insert_frames(docs..docs + ops as u64);
        let cost = run(&server, frames, threads);
        report(&format!("insert one, {threads} connections"), ops, cost);
    }

    if only != "insert" {
        let frames = update_frames(ops, docs, 5);
        let cost = run(&server, frames, threads);
        report(&format!("update status, {threads} connections"), ops, cost);
    }
}

/// Only the bulk load: quick to rerun under a profiler.
#[test]
#[ignore]
fn benchmark_bulk_load() {
    let docs = env_usize("BENCH_DOCS", 300_000) as u64;
    let threads = env_usize("BENCH_THREADS", 8);
    let server = Server::start();
    let frames = bulk_frames(0..docs);
    let cost = run(&server, frames, threads);
    report(
        &format!("bulk load, {threads} connections"),
        docs as usize,
        cost,
    );
}
