//! The columnar benchmark: one wide table of events and four analytical
//! questions, against SoliDB, ClickHouse and DuckDB (col.sh).
//!
//! SoliDB and ClickHouse are servers the harness talks to. DuckDB is a
//! library: it runs in this process, on the threads it is given, with no
//! network between a question and its answer.
//!
//! Row `i` is a function of `i`, so the harness computes every answer
//! itself and compares it with the database's, before the timing and during
//! it.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const T0: i64 = 1_760_000_000_000;
const STEP_MS: i64 = 100;
const USERS: u64 = 100_000;
const COUNTRIES: u64 = 50;
const DEVICES: [&str; 5] = ["android", "desktop", "ios", "tablet", "tv"];
const BATCH: u64 = 20_000;
const LOADERS: u64 = 8;
const SCENARIOS: [&str; 4] = ["filter_agg", "group_all", "window_group", "top_users"];

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[derive(Clone, Copy)]
struct Row {
    ts: i64,
    user: u32,
    country: u8,
    device: u8,
    /// A quarter of a whole number, so that sums are exact in a double
    /// (a third with `COL_VALUES=thirds`).
    amount: f64,
    bytes: i64,
}

fn row(i: u64) -> Row {
    let h = mix(i);
    Row {
        ts: T0 + i as i64 * STEP_MS,
        user: (mix(h ^ 1) % USERS) as u32,
        country: (mix(h ^ 2) % COUNTRIES) as u8,
        device: (mix(h ^ 3) % DEVICES.len() as u64) as u8,
        amount: (mix(h ^ 4) % 40_000) as f64 / divisor(),
        bytes: (mix(h ^ 5) % 100_000) as i64,
    }
}

/// What an amount is a fraction of: 4, whose sums are exact in a double,
/// or with `COL_VALUES=thirds` in the environment 3, values a double
/// rounds and sums that round too.
fn divisor() -> f64 {
    static THIRDS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *THIRDS.get_or_init(|| std::env::var("COL_VALUES").as_deref() == Ok("thirds")) {
        3.0
    } else {
        4.0
    }
}

fn country(c: u8) -> String {
    format!("c{c:02}")
}

fn row_json(r: &Row) -> Value {
    json!({"ts": r.ts, "user": r.user, "country": country(r.country),
           "device": DEVICES[r.device as usize], "amount": r.amount, "bytes": r.bytes})
}

/// One request and the answer the generator gives: a count of rows or
/// groups, and a sum that identifies the content.
#[derive(Clone)]
enum Case {
    FilterAgg { country: u8, want: (u64, f64) },
    GroupAll { want: (u64, f64) },
    WindowGroup { lo: i64, hi: i64, want: (u64, f64) },
    TopUsers { want: (u64, f64) },
}

fn cases(rows: &[Row], scenario: &str) -> Vec<Case> {
    match scenario {
        "filter_agg" => (0..COUNTRIES as u8)
            .map(|c| {
                let hit = rows.iter().filter(|r| r.country == c);
                let want = hit.fold((0u64, 0f64), |(n, s), r| (n + 1, s + r.amount));
                Case::FilterAgg { country: c, want }
            })
            .collect(),
        // Per country: the count and the sum of bytes. Checked by the
        // number of groups and the sum of count * 1e-3 + bytes.
        "group_all" => {
            let mut groups: HashMap<u8, (u64, i64)> = HashMap::new();
            for r in rows {
                let g = groups.entry(r.country).or_default();
                g.0 += 1;
                g.1 += r.bytes;
            }
            let sum = groups.values().map(|(n, b)| *b as f64 + *n as f64 * 1e-3).sum();
            vec![Case::GroupAll { want: (groups.len() as u64, sum) }]
        }
        // Per device over a twentieth of the span: bytes and count.
        "window_group" => {
            let span = rows.len() as i64 * STEP_MS;
            (0..40i64)
                .map(|k| {
                    let lo = T0 + (mix(k as u64 ^ 77) % 19) as i64 * span / 20 + k * 1_000;
                    let hi = lo + span / 20;
                    let mut groups: HashMap<u8, (u64, i64)> = HashMap::new();
                    for r in rows.iter().filter(|r| r.ts >= lo && r.ts < hi) {
                        let g = groups.entry(r.device).or_default();
                        g.0 += 1;
                        g.1 += r.bytes;
                    }
                    let sum = groups.values().map(|(n, b)| *b as f64 + *n as f64 * 1e-3).sum();
                    Case::WindowGroup { lo, hi, want: (groups.len() as u64, sum) }
                })
                .collect()
        }
        // The ten users with the largest total amount: ten rows, and the
        // sum of their totals.
        "top_users" => {
            let mut totals = vec![0f64; USERS as usize];
            for r in rows {
                totals[r.user as usize] += r.amount;
            }
            totals.sort_by(|a, b| b.partial_cmp(a).expect("no NaN"));
            let top: f64 = totals.iter().take(10).sum();
            vec![Case::TopUsers { want: (totals.len().min(10) as u64, top) }]
        }
        other => panic!("unknown scenario {other}"),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

// ---------------------------------------------------------------- engines

#[derive(Clone)]
enum Engine {
    Soli { addr: String, password: String },
    ClickHouse { addr: String },
    /// The database file's first connection; the others are cloned from it.
    Duck(Arc<std::sync::Mutex<duckdb::Connection>>),
}

enum Conn {
    Soli(solidb_client::SoliDBClient),
    ClickHouse(reqwest::Client, String),
    Duck(duckdb::Connection),
}

/// Run `sql` on DuckDB and give the numeric columns of each row.
fn duck_rows(conn: &duckdb::Connection, sql: &str) -> Res<Vec<Vec<f64>>> {
    use duckdb::types::ValueRef;
    let mut statement = conn.prepare(sql)?;
    let mut rows = statement.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut values = Vec::new();
        let mut at = 0;
        while let Ok(value) = row.get_ref(at) {
            values.push(match value {
                ValueRef::TinyInt(v) => v as f64,
                ValueRef::SmallInt(v) => v as f64,
                ValueRef::Int(v) => v as f64,
                ValueRef::BigInt(v) => v as f64,
                ValueRef::HugeInt(v) => v as f64,
                ValueRef::UTinyInt(v) => v as f64,
                ValueRef::USmallInt(v) => v as f64,
                ValueRef::UInt(v) => v as f64,
                ValueRef::UBigInt(v) => v as f64,
                ValueRef::Float(v) => v as f64,
                ValueRef::Double(v) => v,
                _ => f64::NAN,
            });
            at += 1;
        }
        out.push(values);
    }
    Ok(out)
}

/// ClickHouse's SQL as DuckDB reads it.
fn duck_sql(sql: &str) -> String {
    sql.replace("count()", "count(*)").replace("user", "\"user\"")
}

const DB: &str = "bench";

const S_FILTER: &str = "FOR r IN events FILTER r.country == @c \
     COLLECT AGGREGATE n = COUNT(), s = SUM(r.amount) RETURN [n, s]";
const S_GROUP: &str = "FOR r IN events COLLECT c = r.country \
     AGGREGATE n = COUNT(), b = SUM(r.bytes) RETURN [c, n, b]";
const S_WINDOW: &str = "FOR r IN events FILTER r.ts >= @lo AND r.ts < @hi COLLECT d = r.device \
     AGGREGATE n = COUNT(), b = SUM(r.bytes) RETURN [d, n, b]";
const S_TOP: &str = "FOR r IN events COLLECT u = r.user AGGREGATE s = SUM(r.amount) \
     SORT s DESC LIMIT 10 RETURN [u, s]";

fn c_filter(c: u8) -> String {
    format!("SELECT count(), sum(amount) FROM events WHERE country = '{}'", country(c))
}
const C_GROUP: &str = "SELECT country, count(), sum(bytes) FROM events GROUP BY country";
fn c_window(lo: i64, hi: i64) -> String {
    format!(
        "SELECT device, count(), sum(bytes) FROM events WHERE ts >= {lo} AND ts < {hi} \
         GROUP BY device"
    )
}
const C_TOP: &str =
    "SELECT user, sum(amount) AS s FROM events GROUP BY user ORDER BY s DESC LIMIT 10";

async fn soli_http(addr: &str, password: &str) -> Res<(reqwest::Client, String)> {
    let http = reqwest::Client::new();
    let login: Value = http
        .post(format!("http://{addr}/auth/login"))
        .json(&json!({"username": "admin", "password": password}))
        .send()
        .await?
        .json()
        .await?;
    let token = login["token"].as_str().ok_or(format!("login: {login}"))?.to_string();
    Ok((http, token))
}

async fn clickhouse(http: &reqwest::Client, addr: &str, sql: &str, body: Option<String>) -> Res<String> {
    let request = match body {
        Some(body) => http.post(format!("http://{addr}/")).query(&[("query", sql)]).body(body),
        None => http.post(format!("http://{addr}/")).body(sql.to_string()),
    };
    let answer = request.send().await?;
    let status = answer.status();
    let text = answer.text().await?;
    if !status.is_success() {
        return Err(format!("clickhouse {status}: {text}").into());
    }
    Ok(text)
}

impl Engine {
    async fn reset(&self) -> Res<()> {
        match self {
            Engine::Soli { addr, password } => {
                let (http, token) = soli_http(addr, password).await?;
                let api = format!("http://{addr}/_api/database");
                http.delete(format!("{api}/{DB}")).bearer_auth(&token).send().await?;
                http.post(&api).bearer_auth(&token).json(&json!({"name": DB})).send().await?;
                let columns = json!({"name": "events", "compression": "lz4", "columns": [
                    {"name": "ts", "type": "INT64"}, {"name": "user", "type": "INT64"},
                    {"name": "country", "type": "STRING"}, {"name": "device", "type": "STRING"},
                    {"name": "amount", "type": "FLOAT64"}, {"name": "bytes", "type": "INT64"}]});
                let made = http
                    .post(format!("{api}/{DB}/columnar"))
                    .bearer_auth(&token)
                    .json(&columns)
                    .send()
                    .await?;
                if !made.status().is_success() {
                    return Err(format!("create: {}", made.text().await?).into());
                }
                Ok(())
            }
            Engine::Duck(root) => {
                let conn = root.lock().map_err(|e| e.to_string())?;
                // The cores and the memory the other two get from their
                // containers.
                conn.execute_batch(
                    "SET threads = 8; SET memory_limit = '16GB';
                     DROP TABLE IF EXISTS events;
                     CREATE TABLE events (ts BIGINT, \"user\" UINTEGER, country VARCHAR,
                                          device VARCHAR, amount DOUBLE, bytes BIGINT);",
                )?;
                Ok(())
            }
            Engine::ClickHouse { addr } => {
                let http = reqwest::Client::new();
                clickhouse(&http, addr, "DROP TABLE IF EXISTS events", None).await?;
                clickhouse(
                    &http,
                    addr,
                    "CREATE TABLE events (ts Int64, user UInt32, country LowCardinality(String), \
                     device LowCardinality(String), amount Float64, bytes Int64) \
                     ENGINE = MergeTree ORDER BY (country, ts)",
                    None,
                )
                .await?;
                Ok(())
            }
        }
    }

    async fn version(&self) -> Res<String> {
        match self {
            Engine::Soli { .. } => Ok("see the image".into()),
            Engine::Duck(root) => {
                let conn = root.lock().map_err(|e| e.to_string())?;
                Ok(conn.query_row(
                    "SELECT version() || ', threads ' || current_setting('threads') \
                     || ', memory limit ' || current_setting('memory_limit')",
                    [],
                    |row| row.get::<_, String>(0),
                )?)
            }
            Engine::ClickHouse { addr } => {
                let http = reqwest::Client::new();
                Ok(clickhouse(&http, addr, "SELECT version()", None).await?.trim().to_string())
            }
        }
    }

    /// Insert rows `from..to` in one request.
    async fn insert(&self, from: u64, to: u64) -> Res<()> {
        match self {
            Engine::Soli { addr, password } => {
                let (http, token) = soli_http(addr, password).await?;
                let rows: Vec<Value> = (from..to).map(|i| row_json(&row(i))).collect();
                let sent = http
                    .post(format!("http://{addr}/_api/database/{DB}/columnar/events/insert"))
                    .bearer_auth(&token)
                    .json(&json!({"rows": rows}))
                    .send()
                    .await?;
                if !sent.status().is_success() {
                    return Err(format!("insert: {}", sent.text().await?).into());
                }
                Ok(())
            }
            Engine::Duck(root) => {
                let conn = root.lock().map_err(|e| e.to_string())?.try_clone()?;
                tokio::task::block_in_place(|| -> Res<()> {
                    let mut appender = conn.appender("events")?;
                    for i in from..to {
                        let r = row(i);
                        appender.append_row(duckdb::params![
                            r.ts,
                            r.user,
                            country(r.country),
                            DEVICES[r.device as usize],
                            r.amount,
                            r.bytes
                        ])?;
                    }
                    appender.flush()?;
                    Ok(())
                })
            }
            Engine::ClickHouse { addr } => {
                let http = reqwest::Client::new();
                let mut body = String::new();
                for i in from..to {
                    body.push_str(&row_json(&row(i)).to_string());
                    body.push('\n');
                }
                clickhouse(&http, addr, "INSERT INTO events FORMAT JSONEachRow", Some(body)).await?;
                Ok(())
            }
        }
    }

    async fn after_load(&self) -> Res<()> {
        match self {
            Engine::ClickHouse { addr } => {
                let http = reqwest::Client::new();
                clickhouse(&http, addr, "OPTIMIZE TABLE events FINAL", None).await?;
            }
            Engine::Duck(root) => {
                let conn = root.lock().map_err(|e| e.to_string())?;
                conn.execute_batch("CHECKPOINT;")?;
            }
            Engine::Soli { .. } => {}
        }
        Ok(())
    }

    async fn connect(&self) -> Res<Conn> {
        match self {
            Engine::Soli { addr, password } => Ok(Conn::Soli(
                solidb_client::SoliDBClientBuilder::new(addr)
                    .auth("_system", "admin", password)
                    .pool_size(1)
                    .build()
                    .await
                    .map_err(|e| e.to_string())?,
            )),
            Engine::ClickHouse { addr } => {
                Ok(Conn::ClickHouse(reqwest::Client::new(), addr.clone()))
            }
            Engine::Duck(root) => Ok(Conn::Duck(
                root.lock().map_err(|e| e.to_string())?.try_clone()?,
            )),
        }
    }

    fn queries(&self) -> Value {
        match self {
            Engine::Soli { .. } => json!({"filter_agg": S_FILTER, "group_all": S_GROUP,
                "window_group": S_WINDOW, "top_users": S_TOP}),
            Engine::ClickHouse { .. } => json!({"filter_agg": c_filter(7), "group_all": C_GROUP,
                "window_group": c_window(0, 1), "top_users": C_TOP}),
            Engine::Duck(_) => json!({"filter_agg": duck_sql(&c_filter(7)),
                "group_all": duck_sql(C_GROUP), "window_group": duck_sql(&c_window(0, 1)),
                "top_users": duck_sql(C_TOP)}),
        }
    }
}

/// The rows of an answer as `(count, sum)`: `count` rows, and the sum of
/// `f(row)` over them.
fn digest(rows: &[Vec<f64>], f: impl Fn(&[f64]) -> f64) -> (u64, f64) {
    (rows.len() as u64, rows.iter().map(|r| f(r)).sum())
}

/// The numeric columns of each row of an answer (a string column is NaN).
async fn ask(conn: &mut Conn, soli: (&str, Vec<(&str, Value)>), sql: String) -> Res<Vec<Vec<f64>>> {
    let number = |v: &Value| match v {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => s.parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    };
    if let Conn::Duck(conn) = conn {
        return tokio::task::block_in_place(|| duck_rows(conn, &duck_sql(&sql)));
    }
    let rows: Vec<Value> = match conn {
        Conn::Duck(_) => Vec::new(),
        Conn::Soli(client) => {
            let vars: HashMap<String, Value> =
                soli.1.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            client.query_with_cache(DB, soli.0, Some(vars), false).await.map_err(|e| e.to_string())?
        }
        Conn::ClickHouse(http, addr) => {
            let text = clickhouse(http, addr, &format!("{sql} FORMAT JSONCompact"), None).await?;
            let answer: Value = serde_json::from_str(&text)?;
            answer["data"].as_array().cloned().unwrap_or_default()
        }
    };
    Ok(rows
        .iter()
        .map(|r| r.as_array().map(|a| a.iter().map(number).collect()).unwrap_or_default())
        .collect())
}

/// Run one case; `Some(problem)` when the answer is not the generator's.
async fn one(conn: &mut Conn, case: &Case) -> Res<Option<String>> {
    let (got, want, what) = match case {
        Case::FilterAgg { country: c, want } => {
            let rows = ask(conn, (S_FILTER, vec![("c", json!(country(*c)))]), c_filter(*c)).await?;
            let got = rows.first().map_or((0, 0.0), |r| (r[0] as u64, r[1]));
            (got, *want, format!("filter_agg({c})"))
        }
        Case::GroupAll { want } => {
            let rows = ask(conn, (S_GROUP, vec![]), C_GROUP.to_string()).await?;
            (digest(&rows, |r| r[2] + r[1] * 1e-3), *want, "group_all".to_string())
        }
        Case::WindowGroup { lo, hi, want } => {
            let vars = vec![("lo", json!(lo)), ("hi", json!(hi))];
            let rows = ask(conn, (S_WINDOW, vars), c_window(*lo, *hi)).await?;
            (digest(&rows, |r| r[2] + r[1] * 1e-3), *want, format!("window_group({lo}, {hi})"))
        }
        Case::TopUsers { want } => {
            let rows = ask(conn, (S_TOP, vec![]), C_TOP.to_string()).await?;
            (digest(&rows, |r| r[1]), *want, "top_users".to_string())
        }
    };
    Ok((got.0 != want.0 || !close(got.1, want.1))
        .then(|| format!("{what}: {got:?}, expected {want:?}")))
}

fn db_cpu(cgroup: &Option<String>) -> Option<f64> {
    let stat = std::fs::read_to_string(format!("{}/cpu.stat", cgroup.as_ref()?)).ok()?;
    let usec = stat.lines().find_map(|l| l.strip_prefix("usage_usec "))?;
    Some(usec.trim().parse::<f64>().ok()? / 1e6)
}

async fn timed(engine: &Engine, n: usize, dur: Duration, cases: Arc<Vec<Case>>, cg: &Option<String>) -> Res<Value> {
    let stop = Arc::new(AtomicBool::new(false));
    let (errors, wrong) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let db0 = db_cpu(cg);
    let start = Instant::now();
    let mut tasks = Vec::new();
    for t in 0..n {
        let mut conn = engine.connect().await?;
        let (stop, errors, wrong, cases) = (stop.clone(), errors.clone(), wrong.clone(), cases.clone());
        tasks.push(tokio::spawn(async move {
            let mut lat = Vec::new();
            let mut at = t * 7;
            while !stop.load(Ordering::Relaxed) {
                let case = &cases[at % cases.len()];
                at += 1;
                let began = Instant::now();
                match one(&mut conn, case).await {
                    Ok(None) => lat.push(began.elapsed().as_micros() as u64),
                    Ok(Some(_)) => {
                        wrong.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            lat
        }));
    }
    tokio::time::sleep(dur).await;
    stop.store(true, Ordering::Relaxed);
    let mut lat: Vec<u64> = Vec::new();
    for task in tasks {
        lat.extend(task.await?);
    }
    let secs = start.elapsed().as_secs_f64();
    lat.sort_unstable();
    let at = |p: f64| lat.get(((lat.len() as f64 * p) as usize).min(lat.len().saturating_sub(1))).copied();
    Ok(json!({
        "ops": lat.len(), "secs": secs, "ops_per_sec": lat.len() as f64 / secs,
        "errors": errors.load(Ordering::Relaxed), "wrong": wrong.load(Ordering::Relaxed),
        "db_cpu_cores": db_cpu(cg).zip(db0).map(|(a, b)| (a - b) / secs),
        "p50_us": at(0.50), "p99_us": at(0.99), "max_us": lat.last(),
    }))
}

#[tokio::main]
async fn main() -> Res<()> {
    let mut args: HashMap<String, String> = HashMap::new();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        args.insert(flag.trim_start_matches("--").to_string(), it.next().expect("a value"));
    }
    let get = |k: &str, d: &str| args.get(k).cloned().unwrap_or_else(|| d.to_string());
    let addr = get("addr", "");
    let engine = match get("engine", "").as_str() {
        "solidb" => Engine::Soli { addr, password: get("password", "bench") },
        "clickhouse" => Engine::ClickHouse { addr },
        // `--addr` is the database file.
        "duckdb" => Engine::Duck(Arc::new(std::sync::Mutex::new(duckdb::Connection::open(&addr)?))),
        other => panic!("unknown engine {other}"),
    };
    let total: u64 = get("rows", "10000000").parse()?;
    let (warm, secs, runs): (u64, u64, u64) =
        (get("warm", "5").parse()?, get("secs", "20").parse()?, get("runs", "3").parse()?);
    let concurrency: Vec<usize> = get("concurrency", "1,4").split(',').map(|c| c.parse().expect("--concurrency")).collect();
    let only = args.get("only").map(|o| o.split(',').map(str::to_string).collect::<Vec<_>>());
    let cgroup = args.get("cgroup").cloned();

    engine.reset().await?;
    eprintln!("{total} rows");
    let began = Instant::now();
    let next = Arc::new(AtomicU64::new(0));
    let mut loaders = Vec::new();
    for _ in 0..LOADERS {
        let (engine, next) = (engine.clone(), next.clone());
        loaders.push(tokio::spawn(async move {
            loop {
                let from = next.fetch_add(BATCH, Ordering::Relaxed);
                if from >= total {
                    return Ok::<(), String>(());
                }
                engine.insert(from, (from + BATCH).min(total)).await.map_err(|e| e.to_string())?;
            }
        }));
    }
    for loader in loaders {
        loader.await??;
    }
    engine.after_load().await?;
    let load = began.elapsed().as_secs_f64();
    eprintln!("loaded in {load:.1} s ({:.0} rows/s)", total as f64 / load);

    let rows: Vec<Row> = (0..total).map(row).collect();
    let mut results = Vec::new();
    for scenario in SCENARIOS {
        if only.as_ref().is_some_and(|o| !o.iter().any(|s| s == scenario)) {
            continue;
        }
        let cases = Arc::new(cases(&rows, scenario));
        let mut conn = engine.connect().await?;
        let mut problem = None;
        for case in cases.iter().take(3) {
            let began = Instant::now();
            match one(&mut conn, case).await {
                Ok(None) => eprintln!("{scenario}: first answer in {:.1} ms", began.elapsed().as_secs_f64() * 1e3),
                Ok(Some(p)) => problem = Some(format!("WRONG {p}")),
                Err(e) => problem = Some(format!("ERROR {e}")),
            }
        }
        if let Some(problem) = problem {
            eprintln!("{scenario}: {problem}");
            results.push(json!({"scenario": scenario, "skipped": problem}));
            continue;
        }
        for &c in &concurrency {
            timed(&engine, c, Duration::from_secs(warm), cases.clone(), &None).await?;
            for run in 0..runs {
                let mut r = timed(&engine, c, Duration::from_secs(secs), cases.clone(), &cgroup).await?;
                eprintln!(
                    "{scenario} c={c} run {run}: {:.1}/s p50 {} µs p99 {} µs, {} wrong, {} errors",
                    r["ops_per_sec"].as_f64().unwrap_or(0.0), r["p50_us"], r["p99_us"], r["wrong"], r["errors"]
                );
                r["scenario"] = json!(scenario);
                r["concurrency"] = json!(c);
                r["run"] = json!(run);
                results.push(r);
            }
        }
    }
    let out = json!({
        "engine": get("engine", ""), "server_version": engine.version().await?, "rows": total,
        "load": {"secs": load, "rows_per_sec": total as f64 / load},
        "warm_secs": warm, "measure_secs": secs, "runs": runs,
        "queries": engine.queries(), "series_results": results,
    });
    let text = serde_json::to_string_pretty(&out)?;
    match args.get("out") {
        Some(path) => std::fs::write(path, text)?,
        None => println!("{text}"),
    }
    Ok(())
}
