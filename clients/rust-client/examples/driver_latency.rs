//! A request's latency over the native driver at concurrency 1, as
//! `benchmarks/compare` measures it, and where that time goes.
//!
//! ```text
//! driver_latency <addr> <password> [--seed ORDERS | --orders ORDERS]
//!     [--secs 5] [--warm 1]
//!     [--series ping,get,by_customer,recent,join] [--split] [--decode]
//! ```
//!
//! - `--seed N` (re)creates the `latency_bench` database with N orders and
//!   N / 10 customers, generated as `benchmarks/compare/src/data.rs` does,
//!   behind indexes on `customer_id`, `created_at` and `status`. Without
//!   it, `--orders` (1,000,000) says how many a previous `--seed` wrote.
//! - Each series runs one query back to back through `SoliDBClient`
//!   (`pool_size(1)`, `query_with_cache(.., false)`, rows returned as
//!   `serde_json::Value` and dropped), on a multi-thread runtime as the
//!   harness does. It prints p50/p90/p99 in microseconds.
//! - `--split` sends the same requests on a socket of its own, twice per
//!   iteration: once the way the client did up to 1.1.0 (`encode_command`,
//!   `read_exact` of the length then of a zeroed payload, `rmp_serde`) and
//!   once the way it does now (one reused buffer each way, the payload
//!   decoded where it was read, `decode_response`). It prints the median of
//!   each stage: encode, write, wait (from the write to the first bytes of
//!   the response: the server and both wake-ups), read, decode, drop.
//! - `--decode` decodes one captured response of each series in a loop,
//!   both ways, without the network, and counts the allocations a row
//!   costs (the allocator is the system's, as in the harness, counted).
//!   `DECODE_ONLY=old|new` times only one way, for a profiler.
//! - `--no-client` skips the series.
//!
//! The series run on a worker of the runtime, as the harness's do: run
//! from `main`'s own thread, every response woke a worker that then woke
//! `main`, two futex calls per request that the harness does not make.
//!
//! The release build is the one to measure (`cargo build --release -p
//! solidb-client --example driver_latency`): the `ci` profile compiles this
//! crate at opt-level 1, which slows rmp_serde's generic code most.

use serde_json::{json, Value};
use solidb_client::protocol::{
    decode_message, decode_response, encode_command, Command, Response, DRIVER_MAGIC,
};
use solidb_client::{SoliDBClient, SoliDBClientBuilder};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const DB: &str = "latency_bench";

/// The system allocator, as the harness uses, counting its allocations so
/// that `--decode` can say how many a row costs.
struct Counting;

static ALLOCATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations() -> u64 {
    ALLOCATIONS.load(std::sync::atomic::Ordering::Relaxed)
}

const Q_BY_CUSTOMER: &str = "FOR o IN orders FILTER o.customer_id == @c RETURN o";
const Q_RECENT: &str = "FOR o IN orders FILTER o.created_at >= @lo AND o.created_at < @hi \
     SORT o.created_at DESC LIMIT 20 RETURN o";
const Q_JOIN: &str = "FOR o IN orders FILTER o.created_at >= @lo AND o.created_at < @hi \
     SORT o.created_at DESC LIMIT 20 \
     LET c = DOCUMENT(CONCAT(\"customers/\", o.customer_id)) \
     RETURN {order: o._key, amount: o.amount, name: c.name, tier: c.tier}";

// ---- the dataset of benchmarks/compare/src/data.rs -------------------------

const EPOCH: u64 = 1_704_067_200;
const SPAN_SECS: u64 = 2 * 365 * 86_400;
const WINDOW_SECS: u64 = 86_400;
const STATUSES: [(&str, u64); 5] = [
    ("delivered", 70),
    ("shipped", 15),
    ("pending", 8),
    ("cancelled", 5),
    ("returned", 2),
];
const COUNTRIES: [&str; 20] = [
    "FR", "DE", "ES", "IT", "GB", "NL", "BE", "CH", "PT", "SE", "NO", "DK", "PL", "AT", "IE", "US",
    "CA", "BR", "JP", "AU",
];
const CITIES: [&str; 16] = [
    "Lyon",
    "Paris",
    "Lille",
    "Nantes",
    "Bordeaux",
    "Toulouse",
    "Nice",
    "Rennes",
    "Berlin",
    "Madrid",
    "Milan",
    "London",
    "Amsterdam",
    "Brussels",
    "Geneva",
    "Lisbon",
];
const STREETS: [&str; 8] = [
    "rue des Lilas",
    "avenue Jean Jaurès",
    "boulevard Voltaire",
    "rue de la République",
    "place Bellecour",
    "quai Saint-Antoine",
    "rue Victor Hugo",
    "chemin des Vignes",
];
const TAGS: [&str; 12] = [
    "gift",
    "express",
    "promo",
    "first-order",
    "b2b",
    "fragile",
    "bulky",
    "subscription",
    "loyalty",
    "marketplace",
    "pickup",
    "eco",
];
const CHANNELS: [&str; 4] = ["web", "mobile", "store", "phone"];
const TIERS: [&str; 4] = ["bronze", "silver", "gold", "platinum"];
const NOTES: [&str; 4] = [
    "Leave the parcel with the concierge if nobody answers the door.",
    "Call before delivery; the intercom does not work.",
    "Deliver to the side entrance, next to the bakery.",
    "",
];

fn mix(seed: u64, i: u64) -> u64 {
    let mut z = seed ^ i.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(mix(seed, 0x5eed) | 1)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn iso(secs: u64) -> String {
    let (y, m, d) = civil((secs / 86_400) as i64);
    let s = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

fn item(i: u64, k: u64) -> (u64, u64, u64) {
    let h = mix(10 + k, i);
    (h % 50_000, 1 + (h >> 20) % 4, 199 + (h >> 32) % 9_800)
}

fn order(i: u64, customers: u64) -> Value {
    let h1 = mix(1, i);
    let roll = mix(2, i) % 100;
    let mut acc = 0;
    let mut status = 0;
    for (n, (_, weight)) in STATUSES.iter().enumerate() {
        acc += weight;
        if roll < acc {
            status = n;
            break;
        }
    }
    let amount_cents: u64 = (0..3).map(|k| item(i, k)).map(|(_, q, p)| q * p).sum();
    let h = mix(4, i);
    let items: Vec<Value> = (0..3)
        .map(|k| {
            let (sku, qty, price) = item(i, k);
            json!({"sku": format!("SKU-{sku:05}"), "qty": qty, "price": price as f64 / 100.0})
        })
        .collect();
    json!({
        "_key": format!("o{i:07}"),
        "customer_id": format!("c{:05}", h1 % customers),
        "status": STATUSES[status].0,
        "amount": amount_cents as f64 / 100.0,
        "currency": "EUR",
        "created_at": iso(EPOCH + mix(3, i) % SPAN_SECS),
        "country": COUNTRIES[(h % 20) as usize],
        "channel": CHANNELS[(h >> 8) as usize % 4],
        "tags": [TAGS[(h >> 12) as usize % 12], TAGS[(h >> 16) as usize % 12], TAGS[(h >> 20) as usize % 12]],
        "items": items,
        "address": {
            "street": format!("{} {}", 1 + (h >> 24) % 180, STREETS[(h >> 32) as usize % 8]),
            "city": CITIES[(h >> 36) as usize % 16],
            "zip": format!("{:05}", 1000 + (h >> 40) % 94_000),
        },
        "note": NOTES[(h >> 48) as usize % 4],
    })
}

fn customer(c: u64) -> Value {
    let h = mix(5, c);
    json!({
        "_key": format!("c{c:05}"),
        "name": format!("Customer {c}"),
        "email": format!("customer{c}@example.com"),
        "tier": TIERS[(h % 4) as usize],
        "country": COUNTRIES[(h >> 8) as usize % 20],
        "since": iso(EPOCH - (h >> 16) % (3 * 365 * 86_400)),
    })
}

// ---- the series ------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Series {
    Ping,
    Get,
    ByCustomer,
    Recent,
    Join,
}

impl Series {
    fn parse(s: &str) -> Series {
        match s {
            "ping" => Series::Ping,
            "get" => Series::Get,
            "by_customer" => Series::ByCustomer,
            "recent" => Series::Recent,
            "join" => Series::Join,
            other => panic!("unknown series {other}"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Series::Ping => "ping",
            Series::Get => "get",
            Series::ByCustomer => "by_customer",
            Series::Recent => "recent",
            Series::Join => "join",
        }
    }

    /// The command the client sends for request `r`.
    fn command(self, rng: &mut Rng, orders: u64) -> Command {
        let customers = (orders / 10).max(1);
        let query = |sdbql: &str, vars: Vec<(&str, Value)>| Command::Query {
            database: DB.to_string(),
            sdbql: sdbql.to_string(),
            bind_vars: Some(vars.into_iter().map(|(k, v)| (k.to_string(), v)).collect()),
            cache: false,
        };
        let window = |rng: &mut Rng| {
            let lo = EPOCH + rng.below(SPAN_SECS - WINDOW_SECS);
            vec![("lo", json!(iso(lo))), ("hi", json!(iso(lo + WINDOW_SECS)))]
        };
        match self {
            Series::Ping => Command::Ping,
            Series::Get => Command::Get {
                database: DB.to_string(),
                collection: "orders".to_string(),
                key: format!("o{:07}", rng.below(orders)),
            },
            Series::ByCustomer => query(
                Q_BY_CUSTOMER,
                vec![("c", json!(format!("c{:05}", rng.below(customers))))],
            ),
            Series::Recent => query(Q_RECENT, window(rng)),
            Series::Join => query(Q_JOIN, window(rng)),
        }
    }
}

/// What the harness does for one request, through the client's API.
async fn client_request(c: &mut SoliDBClient, command: Command) -> usize {
    match command {
        Command::Ping => {
            c.ping().await.unwrap();
            1
        }
        Command::Get {
            database,
            collection,
            key,
        } => {
            let doc = c.get(&database, &collection, &key).await.unwrap();
            usize::from(doc.get("customer_id").is_some())
        }
        Command::Query {
            database,
            sdbql,
            bind_vars,
            cache,
        } => c
            .query_with_cache(&database, &sdbql, bind_vars, cache)
            .await
            .unwrap()
            .len(),
        _ => unreachable!(),
    }
}

fn pct(sorted: &[u32], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p) as usize] as f64 / 1e3
}

/// Back to back through `SoliDBClient` for `secs`, after `warm`.
async fn series(
    addr: &str,
    password: &str,
    s: Series,
    orders: u64,
    warm: Duration,
    secs: Duration,
) {
    let mut c = connect(addr, password).await;
    let mut rng = Rng::new(s as u64 + 1);
    let start = Instant::now();
    let (from, until) = (start + warm, start + warm + secs);
    let mut lat: Vec<u32> = Vec::with_capacity(1 << 20);
    let mut rows = 0usize;
    let mut all = 0u64;
    loop {
        let command = s.command(&mut rng, orders);
        let t = Instant::now();
        if t >= until {
            break;
        }
        all += 1;
        let n = client_request(&mut c, command).await;
        if t >= from {
            lat.push(t.elapsed().as_nanos() as u32);
            rows += n;
        }
    }
    lat.sort_unstable();
    println!(
        "LAT|{}|{} ops|{:.1} rows/op|p50 {:.1} us|p90 {:.1} us|p99 {:.1} us|{all} ops with the warm-up",
        s.name(),
        lat.len(),
        rows as f64 / lat.len() as f64,
        pct(&lat, 0.5),
        pct(&lat, 0.9),
        pct(&lat, 0.99),
    );
}

async fn connect(addr: &str, password: &str) -> SoliDBClient {
    SoliDBClientBuilder::new(addr)
        .auth("_system", "admin", password)
        .pool_size(1)
        .build()
        .await
        .expect("connect")
}

// ---- the split, on a socket of our own -------------------------------------

/// Nanosecond stamps of one request: start, encoded, written, first bytes
/// read, frame read, decoded, dropped.
type Stamps = [u64; 7];

struct RawConn {
    stream: TcpStream,
    rbuf: Vec<u8>,
    wbuf: Vec<u8>,
}

impl RawConn {
    async fn open(addr: &str, password: &str) -> RawConn {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.set_nodelay(true).unwrap();
        stream.write_all(DRIVER_MAGIC).await.unwrap();
        let mut conn = RawConn {
            stream,
            rbuf: Vec::with_capacity(32 * 1024),
            wbuf: Vec::new(),
        };
        let auth = Command::Auth {
            database: "_system".into(),
            username: "admin".into(),
            password: password.into(),
            api_key: None,
        };
        let mut stamps = [0; 7];
        conn.as_before(&auth, Instant::now(), &mut stamps).await;
        conn
    }

    /// The client up to 1.1.0.
    async fn as_before(&mut self, command: &Command, t0: Instant, st: &mut Stamps) -> usize {
        let ns = |t0: Instant| t0.elapsed().as_nanos() as u64;
        let data = encode_command(command).unwrap();
        st[1] = ns(t0);
        self.stream.write_all(&data).await.unwrap();
        self.stream.flush().await.unwrap();
        st[2] = ns(t0);
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await.unwrap();
        st[3] = ns(t0);
        let mut payload = vec![0u8; u32::from_be_bytes(len_buf) as usize];
        self.stream.read_exact(&mut payload).await.unwrap();
        st[4] = ns(t0);
        let response: Response = decode_message(&payload).unwrap();
        let rows = rows_of(response);
        st[5] = ns(t0);
        let n = rows.len();
        drop(rows);
        drop(payload);
        st[6] = ns(t0);
        n
    }

    /// The client now.
    async fn as_now(&mut self, command: &Command, t0: Instant, st: &mut Stamps) -> usize {
        let ns = |t0: Instant| t0.elapsed().as_nanos() as u64;
        self.wbuf.clear();
        self.wbuf.extend_from_slice(&[0; 4]);
        rmp_serde::encode::write_named(&mut self.wbuf, command).unwrap();
        let len = (self.wbuf.len() - 4) as u32;
        self.wbuf[..4].copy_from_slice(&len.to_be_bytes());
        st[1] = ns(t0);
        self.stream.write_all(&self.wbuf).await.unwrap();
        self.stream.flush().await.unwrap();
        st[2] = ns(t0);
        self.rbuf.clear();
        while self.rbuf.len() < 4 {
            self.rbuf.reserve(4);
            assert!(self.stream.read_buf(&mut self.rbuf).await.unwrap() > 0);
        }
        st[3] = ns(t0);
        let end = 4 + u32::from_be_bytes(self.rbuf[..4].try_into().unwrap()) as usize;
        while self.rbuf.len() < end {
            self.rbuf.reserve(end - self.rbuf.len());
            assert!(self.stream.read_buf(&mut self.rbuf).await.unwrap() > 0);
        }
        st[4] = ns(t0);
        let rows = rows_of(decode_response(&self.rbuf[4..end]).unwrap());
        st[5] = ns(t0);
        let n = rows.len();
        drop(rows);
        st[6] = ns(t0);
        n
    }
}

fn rows_of(response: Response) -> Vec<Value> {
    match response {
        Response::Ok {
            data: Some(Value::Array(rows)),
            ..
        } => rows,
        Response::Ok { data: Some(v), .. } => vec![v],
        Response::Ok { data: None, .. } => Vec::new(),
        Response::Pong { .. } => vec![Value::Null],
        other => panic!("unexpected response {other:?}"),
    }
}

async fn split(addr: &str, password: &str, s: Series, orders: u64, warm: Duration, secs: Duration) {
    let mut conn = RawConn::open(addr, password).await;
    let mut rng = Rng::new(s as u64 + 100);
    let start = Instant::now();
    let (from, until) = (start + warm, start + warm + secs);
    let (mut olds, mut news): (Vec<Stamps>, Vec<Stamps>) = (Vec::new(), Vec::new());
    let mut flip = false;
    while Instant::now() < until {
        let command = s.command(&mut rng, orders);
        // Alternate which goes first, so neither always meets a warmer cache.
        flip = !flip;
        for way in [flip, !flip] {
            let mut st = [0; 7];
            let t0 = Instant::now();
            if way {
                conn.as_before(&command, t0, &mut st).await;
            } else {
                conn.as_now(&command, t0, &mut st).await;
            }
            if t0 >= from {
                if way {
                    olds.push(st);
                } else {
                    news.push(st);
                }
            }
        }
    }
    for (label, all) in [("old", &olds), ("new", &news)] {
        let stage = |i: usize| {
            let mut d: Vec<u64> = all.iter().map(|st| st[i] - st[i - 1]).collect();
            d.sort_unstable();
            d[d.len() / 2] as f64 / 1e3
        };
        let mut total: Vec<u64> = all.iter().map(|st| st[6]).collect();
        total.sort_unstable();
        println!(
            "SPLIT|{}|{label}|{} ops|encode {:.2}|write {:.2}|wait {:.2}|read {:.2}|decode {:.2}|drop {:.2}|total p50 {:.1} us",
            s.name(),
            all.len(),
            stage(1),
            stage(2),
            stage(3),
            stage(4),
            stage(5),
            stage(6),
            total[total.len() / 2] as f64 / 1e3,
        );
    }
}

// ---- decoding alone --------------------------------------------------------

async fn capture(addr: &str, password: &str, s: Series, orders: u64) -> Vec<u8> {
    let mut conn = RawConn::open(addr, password).await;
    let command = s.command(&mut Rng::new(7), orders);
    let data = encode_command(&command).unwrap();
    conn.stream.write_all(&data).await.unwrap();
    let mut len_buf = [0u8; 4];
    conn.stream.read_exact(&mut len_buf).await.unwrap();
    let mut payload = vec![0u8; u32::from_be_bytes(len_buf) as usize];
    conn.stream.read_exact(&mut payload).await.unwrap();
    payload
}

fn decode_alone(s: Series, payload: &[u8]) {
    let old = || rows_of(decode_message::<Response>(payload).unwrap());
    let new = || rows_of(decode_response(payload).unwrap());
    let reference = old();
    assert_eq!(reference, new(), "the two decoders disagree");
    let rows = reference.len().max(1);
    let count = |f: &dyn Fn() -> Vec<Value>| {
        let before = allocations();
        drop(f());
        (allocations() - before) as f64 / rows as f64
    };
    let (old_allocs, new_allocs) = (count(&old), count(&new));
    let time = |f: &dyn Fn() -> Vec<Value>| {
        let n = 20_000;
        for _ in 0..n / 10 {
            drop(std::hint::black_box(f()));
        }
        let t = Instant::now();
        for _ in 0..n {
            drop(std::hint::black_box(f()));
        }
        t.elapsed().as_nanos() as f64 / n as f64 / 1e3
    };
    // `DECODE_ONLY=old|new` times one of them, for a profiler.
    let only = std::env::var("DECODE_ONLY").unwrap_or_default();
    let o = if only == "new" { f64::NAN } else { time(&old) };
    let n = if only == "old" { f64::NAN } else { time(&new) };
    println!(
        "DECODE|{}|{} B|{} rows|rmp_serde {o:.2} us|by hand {n:.2} us|{:.2}x|{:.0} ns/row by hand|allocations/row {old_allocs:.1} -> {new_allocs:.1}",
        s.name(),
        payload.len(),
        rows,
        o / n,
        n * 1e3 / rows as f64,
    );
}

// ---- seeding ---------------------------------------------------------------

async fn seed(addr: &str, password: &str, orders: u64) {
    let mut c = connect(addr, password).await;
    c.delete_database(DB).await.ok();
    c.create_database(DB).await.unwrap();
    for t in ["orders", "customers"] {
        c.create_collection(DB, t, None).await.unwrap();
    }
    for f in ["customer_id", "created_at", "status"] {
        c.create_index(
            DB,
            "orders",
            &format!("idx_{f}"),
            vec![f.to_string()],
            false,
            false,
        )
        .await
        .unwrap();
    }
    let customers = (orders / 10).max(1);
    let t = Instant::now();
    for start in (0..customers).step_by(1000) {
        let docs: Vec<Value> = (start..(start + 1000).min(customers))
            .map(customer)
            .collect();
        c.bulk_insert(DB, "customers", docs).await.unwrap();
    }
    for start in (0..orders).step_by(1000) {
        let docs: Vec<Value> = (start..(start + 1000).min(orders))
            .map(|i| order(i, customers))
            .collect();
        c.bulk_insert(DB, "orders", docs).await.unwrap();
    }
    println!(
        "seeded {orders} orders, {customers} customers in {:?}",
        t.elapsed()
    );
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (addr, password) = (args[0].clone(), args[1].clone());
    let mut opts: HashMap<String, String> = HashMap::new();
    let mut flags = Vec::new();
    let mut i = 2;
    while i < args.len() {
        let a = args[i].trim_start_matches("--").to_string();
        if matches!(a.as_str(), "split" | "decode" | "no-client") {
            flags.push(a);
            i += 1;
        } else {
            opts.insert(a, args[i + 1].clone());
            i += 2;
        }
    }
    let num = |k: &str, d: u64| opts.get(k).map(|v| v.parse().unwrap()).unwrap_or(d);
    let flag = |f: &str| flags.iter().any(|x| x == f);
    let secs = Duration::from_secs(num("secs", 5));
    let warm = Duration::from_secs(num("warm", 1));
    let series_list: Vec<Series> = opts
        .get("series")
        .map(String::as_str)
        .unwrap_or("ping,get,by_customer,recent,join")
        .split(',')
        .map(Series::parse)
        .collect();

    let orders = if let Some(n) = opts.get("seed") {
        let n = n.parse().unwrap();
        seed(&addr, &password, n).await;
        n
    } else {
        num("orders", 1_000_000)
    };

    // On a worker of the runtime, as the harness's workers are: `main`'s
    // own thread is not one, and every response would first wake a worker
    // that then wakes `main`.
    let (split_too, no_client) = (flag("split"), flag("no-client"));
    let decode = flag("decode");
    tokio::spawn(async move {
        if decode {
            for &s in &series_list {
                let payload = capture(&addr, &password, s, orders).await;
                decode_alone(s, &payload);
            }
        }
        if !no_client {
            for &s in &series_list {
                series(&addr, &password, s, orders, warm, secs).await;
            }
        }
        if split_too {
            for &s in &series_list {
                split(&addr, &password, s, orders, warm, secs).await;
            }
        }
    })
    .await
    .unwrap();
}
