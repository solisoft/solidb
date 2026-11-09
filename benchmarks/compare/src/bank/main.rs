//! Money moved between accounts in transactions, against PostgreSQL or
//! SoliDB: how many transfers commit per second with every commit fsynced,
//! whether any money appears or disappears, and what survives a `kill -9`.
//!
//! Each transfer is one transaction: take `amount` from one account (only if
//! it holds that much), give it to another, and insert a ledger row
//! `w{worker}-{seq}`. A transfer refused by the concurrency control (a lock
//! held, a serialization failure, a stale write) is retried with the same
//! backoff on both databases, up to 200 times.
//!
//! **Checks**, after every run and every crash cycle:
//!
//! | What | Holds when |
//! |---|---|
//! | Money | the balances add up to accounts × initial |
//! | No overdraft | no balance is negative |
//! | Acknowledged transfers | each balance is initial + what the acknowledged transfers moved |
//! | Ledger | each balance is initial + what the stored ledger rows moved |
//! | Durability | every acknowledged transfer has its ledger row; nothing else does |
//! | Consistent reads | an auditor summing every balance while transfers run always sees the full total |
//!
//! A transfer whose commit got no answer (the server was killed) is looked
//! up afterwards and counted as landed or not; either is allowed.
//!
//! ```text
//! solidb-bank --engine postgres|solidb --mode MODE --addr HOST:PORT --out FILE
//!     [--password bench] [--label NAME] [--initial 1000] [--max-amount 100]
//!     [--series 10000:1,10000:32,10:32] [--warm 5] [--secs 20] [--runs 3]
//!     [--seed 1] [--audit-ms 10 (0: no auditor)] [--db-cgroup PATH] [--meta key=value ...]
//!     BANK_STEPS=1 in the environment: the driver mode's mean time per step
//! crash mode:
//!     --crash-cycles 20 --kill CMD --start CMD
//!     [--accounts 10000] [--concurrency 32] [--min-ms 1000] [--max-ms 4000]
//! ```
//!
//! Modes: postgres `update` (READ COMMITTED, conditional UPDATEs),
//! `serializable` (read then write), `naive` (read then write at READ
//! COMMITTED, the control); solidb `query` (SDBQL UPDATE in an HTTP
//! transaction), `doc` (document reads and writes in an HTTP transaction),
//! `driver` (`query`'s statements over the native driver).

mod pg;
mod soli;

use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const MAX_ATTEMPTS: u32 = 200;
const OP_TIMEOUT: Duration = Duration::from_secs(30);
/// A worker that hits this many refusals that are not conflicts stops.
const MAX_FAILURES: u64 = 1_000;

#[derive(Clone, Debug)]
pub struct Transfer {
    pub w: u32,
    pub seq: u64,
    pub from: u32,
    pub to: u32,
    pub amount: i64,
}

impl Transfer {
    pub fn id(&self) -> String {
        format!("w{}-{}", self.w, self.seq)
    }
}

/// How one attempt at a transfer ended.
pub enum Attempt {
    Committed,
    /// The source account held less than the amount: rolled back, by the rules.
    Insufficient,
    /// Refused by the concurrency control: rolled back, retried.
    Conflict(String),
    /// Refused for another reason: rolled back.
    Failed(String),
    /// No answer before the commit was sent: not committed.
    Lost(String),
    /// The commit was sent and no answer came back.
    Unknown(String),
}

pub struct Totals {
    pub sum: i64,
    pub count: u64,
    pub min: i64,
}

enum Db {
    Pg(pg::Pg),
    Soli(soli::Soli),
}

enum Conn {
    Pg(pg::Conn),
    Soli(soli::Conn),
    SoliDriver(soli::DriverConn),
}

impl Db {
    /// A worker's connection for `mode`; the auditor's is any other mode's.
    async fn connect(&self, mode: &str) -> Res<Conn> {
        Ok(match self {
            Db::Pg(d) => Conn::Pg(d.connect().await?),
            Db::Soli(d) if mode == "driver" => Conn::SoliDriver(d.connect_driver().await?),
            Db::Soli(d) => Conn::Soli(d.connect().await?),
        })
    }
    async fn ready(&self) -> Res<()> {
        match self {
            Db::Pg(d) => d.ready().await,
            Db::Soli(d) => d.ready().await,
        }
    }
    async fn reset(&self, accounts: u32, initial: i64) -> Res<()> {
        match self {
            Db::Pg(d) => d.reset(accounts, initial).await,
            Db::Soli(d) => d.reset(accounts, initial).await,
        }
    }
    async fn balances(&self) -> Res<Vec<(u32, i64)>> {
        match self {
            Db::Pg(d) => d.balances().await,
            Db::Soli(d) => d.balances().await,
        }
    }
    async fn ledger_by_account(&self) -> Res<HashMap<u32, (i64, i64)>> {
        match self {
            Db::Pg(d) => d.ledger_by_account().await,
            Db::Soli(d) => d.ledger_by_account().await,
        }
    }
    async fn ledger_by_worker(&self) -> Res<HashMap<u32, (u64, u64, u64)>> {
        match self {
            Db::Pg(d) => d.ledger_by_worker().await,
            Db::Soli(d) => d.ledger_by_worker().await,
        }
    }
    async fn present(&self, id: &str) -> Res<bool> {
        match self {
            Db::Pg(d) => d.present(id).await,
            Db::Soli(d) => d.present(id).await,
        }
    }

    /// Wait until the database answers, at most `limit`.
    async fn wait_ready(&self, limit: Duration) -> Res<Duration> {
        let t = Instant::now();
        loop {
            match self.ready().await {
                Ok(()) => return Ok(t.elapsed()),
                Err(e) if t.elapsed() > limit => {
                    return Err(format!("not ready after {limit:?}: {e}").into())
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }
}

impl Conn {
    async fn transfer(&mut self, mode: &str, t: &Transfer) -> Attempt {
        match self {
            Conn::Pg(c) => c.transfer(mode, t).await,
            Conn::Soli(c) => c.transfer(mode, t).await,
            Conn::SoliDriver(c) => c.transfer(t).await,
        }
    }
    async fn audit(&mut self) -> Res<Totals> {
        match self {
            Conn::Pg(c) => c.audit().await,
            Conn::Soli(c) => c.audit().await,
            Conn::SoliDriver(_) => Err("the auditor reads over HTTP".into()),
        }
    }
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// The first retry goes at once, then a random wait up to 0.2, 0.4, … 10 ms.
async fn backoff(attempt: u32, rng: &mut Rng) {
    if attempt < 2 {
        return;
    }
    let cap = (200u64 << (attempt - 2).min(6)).min(10_000);
    tokio::time::sleep(Duration::from_micros(1 + rng.below(cap))).await;
}

/// What one worker did.
#[derive(Default)]
struct Work {
    committed: Vec<Transfer>,
    unknown: Vec<Transfer>,
    next_seq: u64,
    // Counted when the transfer (or the attempt) ended inside the window.
    commits: u64,
    attempts: u64,
    conflicts: u64,
    insufficient: u64,
    gave_up: u64,
    failed: u64,
    latencies_us: Vec<u32>,
    failures: u64,
    errors: Vec<String>,
    stopped: Option<String>,
}

impl Work {
    fn note(&mut self, e: String) {
        if self.errors.len() < 3 {
            self.errors.push(e);
        }
    }
}

struct Job {
    db: Arc<Db>,
    mode: Arc<str>,
    accounts: u32,
    max_amount: i64,
    seed: u64,
    stop: Arc<AtomicBool>,
    window: Option<(Instant, Instant)>,
}

async fn worker(job: Arc<Job>, w: u32, first_seq: u64) -> Work {
    let mut work = Work {
        next_seq: first_seq,
        ..Work::default()
    };
    let mut conn = match job.db.connect(&job.mode).await {
        Ok(c) => c,
        Err(e) => {
            work.stopped = Some(format!("connect: {e}"));
            return work;
        }
    };
    let mut rng = Rng::new(job.seed ^ ((w as u64 + 1) << 32) ^ first_seq);
    let in_window = |t: Instant| job.window.is_some_and(|(a, b)| t >= a && t < b);
    while !job.stop.load(Ordering::Relaxed) {
        let from = rng.below(job.accounts as u64) as u32;
        let mut to = rng.below(job.accounts as u64 - 1) as u32;
        if to >= from {
            to += 1;
        }
        let t = Transfer {
            w,
            seq: work.next_seq,
            from,
            to,
            amount: 1 + rng.below(job.max_amount as u64) as i64,
        };
        work.next_seq += 1;
        let started = Instant::now();
        let mut attempts = 0;
        let outcome = loop {
            attempts += 1;
            let a = match tokio::time::timeout(OP_TIMEOUT, conn.transfer(&job.mode, &t)).await {
                Ok(a) => a,
                Err(_) => Attempt::Unknown(format!("no answer in {OP_TIMEOUT:?}")),
            };
            if in_window(Instant::now()) {
                work.attempts += 1;
                if matches!(a, Attempt::Conflict(_)) {
                    work.conflicts += 1;
                }
            }
            match a {
                Attempt::Conflict(_) if attempts < MAX_ATTEMPTS => {
                    backoff(attempts, &mut rng).await
                }
                other => break other,
            }
        };
        let done = Instant::now();
        let counted = in_window(done);
        match outcome {
            Attempt::Committed => {
                if counted {
                    work.commits += 1;
                    work.latencies_us
                        .push((done - started).as_micros().min(u32::MAX as u128) as u32);
                }
                work.committed.push(t);
            }
            Attempt::Insufficient => work.insufficient += counted as u64,
            Attempt::Conflict(e) => {
                work.gave_up += counted as u64;
                work.note(format!("gave up after {MAX_ATTEMPTS} conflicts: {e}"));
            }
            Attempt::Failed(e) => {
                work.failed += counted as u64;
                work.failures += 1;
                work.note(e);
                if work.failures >= MAX_FAILURES {
                    work.stopped = Some(format!("{MAX_FAILURES} failures"));
                    break;
                }
            }
            Attempt::Lost(e) => {
                work.stopped = Some(format!("lost: {e}"));
                break;
            }
            Attempt::Unknown(e) => {
                work.unknown.push(t);
                work.stopped = Some(format!("commit unanswered: {e}"));
                break;
            }
        }
    }
    work
}

#[derive(Default)]
struct Audits {
    n: u64,
    wrong_total: u64,
    wrong_count: u64,
    negative: u64,
    first_wrong: Option<Value>,
    errors: u64,
}

/// Sum every balance in one statement, over and over, while transfers run.
async fn auditor(
    db: Arc<Db>,
    stop: Arc<AtomicBool>,
    accounts: u32,
    total: i64,
    every_ms: u64,
) -> Audits {
    let mut a = Audits::default();
    if every_ms == 0 {
        return a;
    }
    let Ok(mut conn) = db.connect("audit").await else {
        a.errors += 1;
        return a;
    };
    while !stop.load(Ordering::Relaxed) {
        match conn.audit().await {
            Ok(t) => {
                a.n += 1;
                let wrong = t.sum != total || t.count != accounts as u64 || t.min < 0;
                a.wrong_total += (t.sum != total) as u64;
                a.wrong_count += (t.count != accounts as u64) as u64;
                a.negative += (t.min < 0) as u64;
                if wrong && a.first_wrong.is_none() {
                    a.first_wrong = Some(json!({"sum": t.sum, "count": t.count, "min": t.min}));
                }
            }
            Err(_) => a.errors += 1,
        }
        tokio::time::sleep(Duration::from_millis(every_ms)).await;
    }
    a
}

/// Everything the database holds against what was acknowledged. The
/// transfers in `unknown` are looked up and moved to `committed` when they
/// landed.
async fn check(
    db: &Db,
    accounts: u32,
    initial: i64,
    committed: &mut Vec<Transfer>,
    unknown: &mut Vec<Transfer>,
) -> Res<Value> {
    let in_flight = unknown.len();
    let mut landed = 0;
    for t in unknown.drain(..) {
        if db.present(&t.id()).await? {
            landed += 1;
            committed.push(t);
        }
    }
    let mut violations: Vec<String> = Vec::new();

    let mut expected: HashMap<u32, (u64, u64, u64)> = HashMap::new();
    let mut moved: HashMap<u32, i64> = HashMap::new();
    for t in committed.iter() {
        let e = expected.entry(t.w).or_default();
        e.0 += 1;
        e.1 += t.seq;
        e.2 += t.seq * t.seq;
        *moved.entry(t.from).or_default() -= t.amount;
        *moved.entry(t.to).or_default() += t.amount;
    }
    let found = db.ledger_by_worker().await?;
    let (mut missing, mut extra, mut mismatched) = (0u64, 0u64, 0u64);
    let workers: BTreeSet<u32> = expected.keys().chain(found.keys()).copied().collect();
    for w in workers {
        let e = expected.get(&w).copied().unwrap_or_default();
        let f = found.get(&w).copied().unwrap_or_default();
        if e != f {
            if f.0 < e.0 {
                missing += e.0 - f.0;
            } else if f.0 > e.0 {
                extra += f.0 - e.0;
            } else {
                mismatched += 1;
            }
        }
    }
    let ledger_rows: u64 = found.values().map(|f| f.0).sum();
    if missing > 0 {
        violations.push(format!(
            "{missing} acknowledged transfers have no ledger row"
        ));
    }
    if extra > 0 {
        violations.push(format!("{extra} ledger rows were never acknowledged"));
    }
    if mismatched > 0 {
        violations.push(format!(
            "{mismatched} workers hold other ledger rows than acknowledged"
        ));
    }

    let balances = db.balances().await?;
    let ledger = db.ledger_by_account().await?;
    let total: i64 = balances.iter().map(|b| b.1).sum();
    let drift = total - accounts as i64 * initial;
    let negative = balances.iter().filter(|b| b.1 < 0).count();
    let off_acked = balances
        .iter()
        .filter(|(a, b)| *b != initial + moved.get(a).copied().unwrap_or(0))
        .count();
    let off_ledger = balances
        .iter()
        .filter(|(a, b)| {
            let (out, inn) = ledger.get(a).copied().unwrap_or_default();
            *b != initial - out + inn
        })
        .count();
    if balances.len() != accounts as usize {
        violations.push(format!(
            "{} accounts read, {accounts} expected",
            balances.len()
        ));
    }
    if drift != 0 {
        violations.push(format!(
            "money {}: the balances add up to {total}",
            fmt_signed(drift)
        ));
    }
    if negative > 0 {
        violations.push(format!("{negative} negative balances"));
    }
    if off_acked > 0 {
        violations.push(format!(
            "{off_acked} accounts differ from what the acknowledged transfers moved"
        ));
    }
    if off_ledger > 0 {
        violations.push(format!(
            "{off_ledger} accounts differ from their ledger rows"
        ));
    }
    Ok(json!({
        "acknowledged": committed.len() - landed,
        "in_flight": in_flight,
        "in_flight_landed": landed,
        "ledger_rows": ledger_rows,
        "acknowledged_missing": missing,
        "ledger_extra": extra,
        "ledger_workers_mismatched": mismatched,
        "accounts_read": balances.len(),
        "money_drift": drift,
        "negative_balances": negative,
        "accounts_off_acknowledged": off_acked,
        "accounts_off_ledger": off_ledger,
        "violations": violations,
    }))
}

fn fmt_signed(v: i64) -> String {
    if v > 0 {
        format!("created ({v:+})")
    } else {
        format!("destroyed ({v})")
    }
}

fn pct(sorted: &[u32], p: f64) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn db_cpu(cgroup: &Option<String>) -> Option<f64> {
    let text = std::fs::read_to_string(format!("{}/cpu.stat", cgroup.as_ref()?)).ok()?;
    let usec = text.lines().find_map(|l| l.strip_prefix("usage_usec "))?;
    Some(usec.trim().parse::<f64>().ok()? / 1e6)
}

fn load_avg() -> f64 {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|l| l.split_whitespace().next().and_then(|v| v.parse().ok()))
        .unwrap_or(f64::NAN)
}

struct Args {
    engine: String,
    mode: String,
    label: String,
    addr: String,
    password: String,
    out: String,
    initial: i64,
    max_amount: i64,
    series: Vec<(u32, usize)>,
    warm: Duration,
    secs: Duration,
    runs: usize,
    seed: u64,
    db_cgroup: Option<String>,
    meta: Vec<(String, String)>,
    /// The auditor's pause between two sums; 0 turns it off.
    audit_ms: u64,
    crash_cycles: usize,
    kill: Option<String>,
    start: Option<String>,
    accounts: u32,
    concurrency: usize,
    min_ms: u64,
    max_ms: u64,
}

fn parse_args() -> Args {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut kv: HashMap<String, Vec<String>> = HashMap::new();
    let mut i = 0;
    while i < argv.len() {
        let k = argv[i].trim_start_matches("--").to_string();
        let v = argv
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| panic!("--{k} takes a value"));
        kv.entry(k).or_default().push(v);
        i += 2;
    }
    let one = |k: &str| kv.get(k).and_then(|v| v.last().cloned());
    let num = |k: &str, d: u64| one(k).map(|v| v.parse().expect(k)).unwrap_or(d);
    let engine = one("engine").expect("--engine");
    let mode = one("mode").expect("--mode");
    Args {
        label: one("label").unwrap_or_else(|| format!("{engine}-{mode}")),
        addr: one("addr").expect("--addr"),
        password: one("password").unwrap_or_else(|| "bench".into()),
        out: one("out").expect("--out"),
        initial: num("initial", 1000) as i64,
        max_amount: num("max-amount", 100) as i64,
        series: one("series")
            .unwrap_or_else(|| "10000:1,10000:32,10:32".into())
            .split(',')
            .map(|s| {
                let (a, c) = s
                    .trim()
                    .split_once(':')
                    .expect("--series ACCOUNTS:CLIENTS,…");
                (a.parse().expect("accounts"), c.parse().expect("clients"))
            })
            .collect(),
        warm: Duration::from_secs(num("warm", 5)),
        secs: Duration::from_secs(num("secs", 20)),
        runs: num("runs", 3) as usize,
        seed: num("seed", 1),
        db_cgroup: one("db-cgroup").filter(|c| !c.is_empty()),
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
        audit_ms: num("audit-ms", 10),
        crash_cycles: num("crash-cycles", 0) as usize,
        kill: one("kill"),
        start: one("start"),
        accounts: num("accounts", 10_000) as u32,
        concurrency: num("concurrency", 32) as usize,
        min_ms: num("min-ms", 1_000),
        max_ms: num("max-ms", 4_000),
        engine,
        mode,
    }
}

fn header(a: &Args) -> Value {
    let mut meta = serde_json::Map::new();
    for (k, v) in &a.meta {
        meta.insert(k.clone(), Value::String(v.clone()));
    }
    json!({
        "label": a.label,
        "engine": a.engine,
        "mode": a.mode,
        "initial_balance": a.initial,
        "max_amount": a.max_amount,
        "max_attempts": MAX_ATTEMPTS,
        "seed": a.seed,
        "meta": meta,
    })
}

fn write_out(path: &str, v: &Value) {
    std::fs::write(path, serde_json::to_string_pretty(v).unwrap() + "\n").expect("write --out");
}

async fn run_once(db: &Arc<Db>, a: &Args, accounts: u32, clients: usize, run: usize) -> Res<Value> {
    db.reset(accounts, a.initial).await?;
    let stop = Arc::new(AtomicBool::new(false));
    let t0 = Instant::now();
    let window = (t0 + a.warm, t0 + a.warm + a.secs);
    let job = Arc::new(Job {
        db: db.clone(),
        mode: a.mode.as_str().into(),
        accounts,
        max_amount: a.max_amount,
        seed: a.seed ^ ((run as u64) << 48) ^ ((accounts as u64) << 20) ^ clients as u64,
        stop: stop.clone(),
        window: Some(window),
    });
    let handles: Vec<_> = (0..clients)
        .map(|w| tokio::spawn(worker(job.clone(), w as u32, 0)))
        .collect();
    let aud = tokio::spawn(auditor(
        db.clone(),
        stop.clone(),
        accounts,
        accounts as i64 * a.initial,
        a.audit_ms,
    ));
    let _ = soli::take_steps();
    tokio::time::sleep_until(window.0.into()).await;
    let (cpu0, load0) = (db_cpu(&a.db_cgroup), load_avg());
    tokio::time::sleep_until(window.1.into()).await;
    let cpu1 = db_cpu(&a.db_cgroup);
    stop.store(true, Ordering::Relaxed);
    let mut works = Vec::new();
    for h in handles {
        works.push(h.await?);
    }
    let audits = aud.await?;
    if let Some(steps) = soli::take_steps() {
        let line: Vec<String> = steps
            .iter()
            .map(|(n, c, us)| format!("{n} {us:.1} µs (×{c})"))
            .collect();
        eprintln!("{} steps, mean per call: {}", a.label, line.join(", "));
    }

    let mut committed: Vec<Transfer> = Vec::new();
    let mut unknown: Vec<Transfer> = Vec::new();
    let mut lat: Vec<u32> = Vec::new();
    let (mut commits, mut attempts, mut conflicts, mut insufficient, mut gave_up, mut failed) =
        (0, 0, 0, 0, 0, 0);
    let mut errors: Vec<String> = Vec::new();
    let mut stopped = Vec::new();
    for w in works.iter_mut() {
        committed.append(&mut w.committed);
        unknown.append(&mut w.unknown);
        lat.append(&mut w.latencies_us);
        commits += w.commits;
        attempts += w.attempts;
        conflicts += w.conflicts;
        insufficient += w.insufficient;
        gave_up += w.gave_up;
        failed += w.failed;
        for e in &w.errors {
            if errors.len() < 5 {
                errors.push(e.clone());
            }
        }
        if let Some(s) = &w.stopped {
            stopped.push(s.clone());
        }
    }
    lat.sort_unstable();
    let secs = a.secs.as_secs_f64();
    let check = check(db, accounts, a.initial, &mut committed, &mut unknown).await?;
    let db_cpu_s = match (cpu0, cpu1) {
        (Some(x), Some(y)) => Some(y - x),
        _ => None,
    };
    Ok(json!({
        "accounts": accounts,
        "clients": clients,
        "run": run,
        "commits": commits,
        "per_sec": commits as f64 / secs,
        "attempts": attempts,
        "conflicts": conflicts,
        "conflicts_per_commit": if commits > 0 { conflicts as f64 / commits as f64 } else { f64::NAN },
        "insufficient": insufficient,
        "gave_up": gave_up,
        "failed": failed,
        "p50_us": pct(&lat, 0.50),
        "p90_us": pct(&lat, 0.90),
        "p99_us": pct(&lat, 0.99),
        "p999_us": pct(&lat, 0.999),
        "max_us": lat.last().copied().unwrap_or(0),
        "db_cpu_s": db_cpu_s,
        "db_cpu_us_per_commit": db_cpu_s.map(|c| c * 1e6 / commits.max(1) as f64),
        "load_before": load0,
        "audits": {
            "n": audits.n,
            "wrong_total": audits.wrong_total,
            "wrong_count": audits.wrong_count,
            "negative_min": audits.negative,
            "first_wrong": audits.first_wrong,
            "errors": audits.errors,
        },
        "check": check,
        "errors": errors,
        "stopped_workers": stopped,
    }))
}

async fn throughput(db: Arc<Db>, a: &Args) -> Res<usize> {
    let mut out = header(a);
    let mut series_out = Vec::new();
    let mut violations = 0;
    for &(accounts, clients) in &a.series {
        let mut runs = Vec::new();
        for run in 0..a.runs {
            let r = run_once(&db, a, accounts, clients, run).await?;
            let v = r["check"]["violations"].as_array().map_or(0, |v| v.len());
            violations += v + r["audits"]["wrong_total"].as_u64().unwrap_or(0) as usize;
            eprintln!(
                "{} {accounts} accounts, {clients} clients, run {run}: {:.0}/s p50 {} µs p99 {} µs, \
                 {:.2} conflicts per commit, {} gave up, {} failed; audits {}/{} wrong; {}",
                a.label,
                r["per_sec"].as_f64().unwrap_or(0.0),
                r["p50_us"],
                r["p99_us"],
                r["conflicts_per_commit"].as_f64().unwrap_or(f64::NAN),
                r["gave_up"],
                r["failed"],
                r["audits"]["wrong_total"],
                r["audits"]["n"],
                if v == 0 {
                    "checks ok".to_string()
                } else {
                    format!("VIOLATIONS {}", r["check"]["violations"])
                }
            );
            runs.push(r);
        }
        let mut order: Vec<usize> = (0..runs.len()).collect();
        order.sort_by(|&x, &y| {
            runs[x]["per_sec"]
                .as_f64()
                .partial_cmp(&runs[y]["per_sec"].as_f64())
                .unwrap()
        });
        let median = order[order.len() / 2];
        series_out.push(json!({
            "accounts": accounts, "clients": clients, "median_run": median, "runs": runs,
        }));
        out["series"] = Value::Array(series_out.clone());
        write_out(&a.out, &out);
    }
    Ok(violations)
}

async fn shell(cmd: &str) -> Res<()> {
    let st = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .status()
        .await?;
    if !st.success() {
        return Err(format!("`{cmd}` exited with {st}").into());
    }
    Ok(())
}

async fn crash(db: Arc<Db>, a: &Args) -> Res<usize> {
    let kill = a.kill.as_deref().ok_or("--kill CMD")?;
    let start = a.start.as_deref().ok_or("--start CMD")?;
    db.wait_ready(Duration::from_secs(120)).await?;
    db.reset(a.accounts, a.initial).await?;
    let mut out = header(a);
    out["accounts"] = json!(a.accounts);
    out["clients"] = json!(a.concurrency);
    let mut rng = Rng::new(a.seed);
    let mut seqs = vec![0u64; a.concurrency];
    let mut committed: Vec<Transfer> = Vec::new();
    let mut unknown: Vec<Transfer> = Vec::new();
    let mut cycles = Vec::new();
    let mut violations = 0;
    for cycle in 0..a.crash_cycles {
        let stop = Arc::new(AtomicBool::new(false));
        let job = Arc::new(Job {
            db: db.clone(),
            mode: a.mode.as_str().into(),
            accounts: a.accounts,
            max_amount: a.max_amount,
            seed: a.seed ^ ((cycle as u64 + 1) << 40),
            stop: stop.clone(),
            window: None,
        });
        let handles: Vec<_> = (0..a.concurrency)
            .map(|w| tokio::spawn(worker(job.clone(), w as u32, seqs[w])))
            .collect();
        let ran = Duration::from_millis(a.min_ms + rng.below(a.max_ms - a.min_ms + 1));
        tokio::time::sleep(ran).await;
        shell(kill).await?;
        stop.store(true, Ordering::Relaxed);
        let before = committed.len();
        let mut stopped = 0;
        for (w, h) in handles.into_iter().enumerate() {
            let mut work = h.await?;
            seqs[w] = work.next_seq;
            committed.append(&mut work.committed);
            unknown.append(&mut work.unknown);
            stopped += work.stopped.is_some() as usize;
        }
        let acked = committed.len() - before;
        let in_flight = unknown.len();
        shell(start).await?;
        let recovery = db.wait_ready(Duration::from_secs(600)).await?;
        let c = check(&db, a.accounts, a.initial, &mut committed, &mut unknown).await?;
        let v = c["violations"].as_array().map_or(0, |v| v.len());
        violations += v;
        eprintln!(
            "{} cycle {cycle}: ran {} ms, {acked} acknowledged, {in_flight} in flight ({} landed), \
             ready again in {} ms; {}",
            a.label,
            ran.as_millis(),
            c["in_flight_landed"],
            recovery.as_millis(),
            if v == 0 {
                "checks ok".to_string()
            } else {
                format!("VIOLATIONS {}", c["violations"])
            }
        );
        cycles.push(json!({
            "cycle": cycle,
            "ran_ms": ran.as_millis() as u64,
            "acknowledged_this_cycle": acked,
            "workers_stopped_by_the_kill": stopped,
            "recovery_ms": recovery.as_millis() as u64,
            "check": c,
        }));
        out["cycles"] = Value::Array(cycles.clone());
        out["acknowledged_total"] = json!(committed.len());
        out["violations_total"] = json!(violations);
        write_out(&a.out, &out);
    }
    Ok(violations)
}

#[tokio::main]
async fn main() {
    let a = parse_args();
    let db = Arc::new(match a.engine.as_str() {
        "postgres" => Db::Pg(pg::Pg::new(&a.addr, &a.password)),
        "solidb" => Db::Soli(soli::Soli::new(&a.addr, &a.password)),
        e => panic!("unknown engine {e}"),
    });
    let ok_modes: &[&str] = match a.engine.as_str() {
        "postgres" => &["update", "serializable", "naive"],
        _ => &["query", "doc", "driver"],
    };
    assert!(
        ok_modes.contains(&a.mode.as_str()),
        "--mode for {}: {ok_modes:?}",
        a.engine
    );
    if let Err(e) = db.wait_ready(Duration::from_secs(120)).await {
        eprintln!("{}: {e}", a.label);
        std::process::exit(2);
    }
    let result = if a.crash_cycles > 0 {
        crash(db, &a).await
    } else {
        throughput(db, &a).await
    };
    match result {
        Ok(0) => {}
        Ok(n) => {
            eprintln!("{}: {n} violations", a.label);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("{}: {e}", a.label);
            std::process::exit(2);
        }
    }
}
