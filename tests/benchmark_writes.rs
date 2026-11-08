//! Write-path benchmark shaped like `benchmarks/compare` (card #774): a bulk
//! load in batches of 1,000, single-document inserts, and single-field
//! updates of an indexed field, over the native driver on loopback TCP.
//!
//! The server is the real connection handler (`DriverHandler::
//! handle_connection`) on its own Tokio runtime, eight workers named
//! `bench-server` as the harness gives each database eight hardware threads;
//! the clients are `solidb-client` connections on another runtime
//! (`bench-client`), each building its documents as the harness does. So a
//! write pays what it pays in production: the socket, the frame decode, the
//! authorization, the handler, the reply.
//!
//! The orders are the harness's: about 500 bytes of JSON each, with three
//! single-field indexes (`customer_id`, `created_at`, `status`) created before
//! the load (sparse, as the driver's CreateIndex makes them). The replication
//! log is opened and deferred, as on a server started without `--peer`.
//!
//! Each phase prints operations per second, the latency percentiles of its
//! requests, and the server's CPU time per operation: every thread but the
//! clients', the storage engine's flush and compaction threads included,
//! read from `/proc` (Linux). On a shared machine the wall time moves with
//! everyone else's load, and the CPU time much less.
//!
//! Run with:
//!   cargo test --profile ci --features bench-tests,engine-solistore \
//!     --config 'profile.ci.package.solidb.opt-level=3' \
//!     --test benchmark_writes -- --ignored --nocapture
//! (the `ci` profile builds this crate at opt-level 1, which is no way to
//! measure it). `SOLIDB_ENGINE` picks the engine as it does for a server.
//!
//! Sizes: `BENCH_DOCS` (default 1,000,000 loaded), `BENCH_OPS` (200,000
//! single writes at the higher concurrency, a quarter at one connection),
//! `BENCH_CONCURRENCY` (32), `BENCH_SERVER_THREADS` (8), `BENCH_ONLY`
//! (`load`, `insert` or `update`; the load always runs).

use serde_json::{json, Value};
use solidb::driver::handlers::index;
use solidb::storage::StorageEngine;
use solidb::sync::log::SyncLog;
use solidb_client::{SoliDBClient, SoliDBClientBuilder};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const DB: &str = "bench";
const COLL: &str = "orders";
const BATCH: u64 = 1_000;
const PASSWORD: &str = "bench-password";

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

struct Bench {
    addr: String,
    clients: tokio::runtime::Runtime,
    storage: Arc<StorageEngine>,
    _server: tokio::runtime::Runtime,
    _tmp: TempDir,
}

impl Bench {
    fn start() -> Self {
        std::env::set_var("SOLIDB_ADMIN_PASSWORD", PASSWORD);
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(StorageEngine::new(tmp.path().join("data")).unwrap());
        storage.create_database("_system".to_string()).unwrap();
        solidb::server::auth::AuthService::init(&storage, None, tmp.path().to_str().unwrap())
            .unwrap();
        let log = SyncLog::new(
            "bench-node".to_string(),
            tmp.path().to_str().unwrap(),
            10_000,
        )
        .unwrap();
        log.defer_until_needed();
        let log = Arc::new(log);

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

        let server = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(env_usize("BENCH_SERVER_THREADS", 8))
            .thread_name("bench-server")
            .thread_keep_alive(Duration::from_secs(3600))
            .enable_all()
            .build()
            .unwrap();
        let listener = server
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let accept_storage = storage.clone();
        server.spawn(async move {
            use tokio::io::AsyncReadExt;
            loop {
                let (mut stream, peer) = listener.accept().await.unwrap();
                stream.set_nodelay(true).unwrap();
                let storage = accept_storage.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    let mut magic = [0u8; 14];
                    stream.read_exact(&mut magic).await.unwrap();
                    let mut handler = solidb::driver::DriverHandler::new(storage, Some(log));
                    handler.handle_connection(stream, peer.to_string()).await;
                });
            }
        });
        let clients = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(8)
            .thread_name("bench-client")
            .enable_all()
            .build()
            .unwrap();
        Bench {
            addr,
            clients,
            storage,
            _server: server,
            _tmp: tmp,
        }
    }

    fn count(&self) -> usize {
        self.storage
            .get_database(DB)
            .unwrap()
            .get_collection(COLL)
            .unwrap()
            .count()
    }
}

async fn connect(addr: String) -> SoliDBClient {
    SoliDBClientBuilder::new(&addr)
        .auth(DB, "admin", PASSWORD)
        .pool_size(1)
        .build()
        .await
        .unwrap()
}

/// CPU time of every thread of this process but the clients'.
fn server_cpu() -> Duration {
    // SAFETY: sysconf has no preconditions.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return Duration::ZERO;
    };
    let mut total = 0u64;
    for task in tasks.flatten() {
        let path = task.path();
        let comm = std::fs::read_to_string(path.join("comm")).unwrap_or_default();
        if comm.starts_with("bench-client") {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(path.join("stat")) else {
            continue;
        };
        // After the parenthesised name: state is the first field, utime the
        // twelfth, stime the thirteenth.
        let after = &stat[stat.rfind(')').map_or(0, |i| i + 2)..];
        let fields: Vec<&str> = after.split_whitespace().collect();
        let utime: u64 = fields.get(11).and_then(|v| v.parse().ok()).unwrap_or(0);
        let stime: u64 = fields.get(12).and_then(|v| v.parse().ok()).unwrap_or(0);
        total += utime + stime;
    }
    Duration::from_micros(total * 1_000_000 / ticks)
}

/// Run `requests` requests from `conns` connections, each the `op` for the
/// next index of a shared counter and worth `per_request` operations; print
/// what they cost.
fn phase<F, Fut>(bench: &Bench, label: &str, requests: u64, conns: usize, per_request: u64, op: F)
where
    F: Fn(SoliDBClient, u64) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = SoliDBClient> + Send + 'static,
{
    let next = Arc::new(AtomicU64::new(0));
    let clients: Vec<SoliDBClient> = bench.clients.block_on(futures::future::join_all(
        (0..conns).map(|_| connect(bench.addr.clone())),
    ));
    let cpu_start = server_cpu();
    let start = Instant::now();
    let latencies: Vec<Vec<Duration>> = bench.clients.block_on(async {
        let tasks = clients.into_iter().map(|mut client| {
            let next = next.clone();
            let op = op.clone();
            tokio::spawn(async move {
                let mut taken = Vec::new();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= requests {
                        break;
                    }
                    let t = Instant::now();
                    client = op(client, i).await;
                    taken.push(t.elapsed());
                }
                taken
            })
        });
        futures::future::join_all(tasks)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect()
    });
    let wall = start.elapsed();
    let cpu = server_cpu().saturating_sub(cpu_start);
    let mut all: Vec<Duration> = latencies.into_iter().flatten().collect();
    all.sort_unstable();
    let pct = |p: f64| all[((all.len() - 1) as f64 * p) as usize].as_secs_f64() * 1e6;
    let ops = requests * per_request;
    println!(
        "{label:<26} {ops:>8} ops {:>9.0}/s  request p50 {:>8.1} µs p99 {:>8.1} µs  server CPU/op {:>6.2} µs",
        ops as f64 / wall.as_secs_f64(),
        pct(0.5),
        pct(0.99),
        cpu.as_secs_f64() * 1e6 / ops as f64,
    );
}

fn load(bench: &Bench, range: std::ops::Range<u64>, conns: usize) {
    let (first, end) = (range.start, range.end);
    phase(
        bench,
        &format!("bulk load, {conns} connections"),
        (end - first).div_ceil(BATCH),
        conns,
        BATCH,
        move |mut client, b| async move {
            let start = first + b * BATCH;
            let docs: Vec<Value> = (start..(start + BATCH).min(end))
                .map(|n| {
                    let mut d = order(n);
                    d["_key"] = json!(order_key(n));
                    d
                })
                .collect();
            let n = docs.len();
            assert_eq!(client.bulk_insert(DB, COLL, docs).await.unwrap(), n);
            client
        },
    );
}

/// The harness's write series: the load, then inserts and updates at one
/// connection and at `BENCH_CONCURRENCY`.
#[test]
#[ignore]
fn benchmark_writes() {
    let docs = env_usize("BENCH_DOCS", 1_000_000) as u64;
    let ops = env_usize("BENCH_OPS", 200_000) as u64;
    let concurrency = env_usize("BENCH_CONCURRENCY", 32);
    let only = std::env::var("BENCH_ONLY").unwrap_or_default();
    let bench = Bench::start();

    let first = docs / 10;
    load(&bench, 0..first, 1);
    load(&bench, first..docs, 8);
    assert_eq!(bench.count(), docs as usize);

    let mut next_key = docs;
    if only.is_empty() || only == "insert" {
        for conns in [1, concurrency] {
            let n = if conns == 1 { ops / 4 } else { ops };
            let base = next_key;
            next_key += n;
            phase(
                &bench,
                &format!("insert one, c={conns}"),
                n,
                conns,
                1,
                move |mut client, i| async move {
                    let k = base + i;
                    client
                        .insert(DB, COLL, Some(&order_key(k)), order(k))
                        .await
                        .unwrap();
                    client
                },
            );
        }
        assert_eq!(bench.count(), next_key as usize);
    }
    if only.is_empty() || only == "update" {
        for conns in [1, concurrency] {
            let n = if conns == 1 { ops / 4 } else { ops };
            let seed = 77 + conns as u64;
            phase(
                &bench,
                &format!("update status, c={conns}"),
                n,
                conns,
                1,
                move |mut client, i| async move {
                    let r = mix(seed, i);
                    let status = STATUSES[((r >> 32) % 5) as usize];
                    client
                        .update(
                            DB,
                            COLL,
                            &order_key(r % docs),
                            json!({ "status": status }),
                            true,
                        )
                        .await
                        .unwrap();
                    client
                },
            );
        }
    }
}
