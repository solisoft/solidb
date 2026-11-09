//! SoliDB through its HTTP transaction API (`…/transaction/begin`,
//! `…/{tx}/query`, `…/{tx}/document/…`, `…/{tx}/commit`), the API the Soli
//! framework's model transactions use. The native driver's transactions take
//! document writes by key and read-only queries, so a conditional debit
//! cannot be written there. One HTTP/1.1 keep-alive connection per worker.
//!
//! Two ways to move money, each a transaction of five to seven requests:
//!
//! - `query`: the idiom. `FOR a IN accounts FILTER a._key == @k FILTER
//!   a.balance >= @x UPDATE a WITH {balance: a.balance - @x} IN accounts`,
//!   the credit the same way, then `INSERT` the ledger row. A transaction's
//!   locks do not wait: a key another transaction holds is refused with 409
//!   at once, and the transfer is retried.
//! - `doc`: read both accounts with `GET` (shared locks), decide in the
//!   client, write the new balances with `PUT` (exclusive locks), insert
//!   the ledger row with `POST`.
//! - `driver`: `query`'s statements through the native MessagePack driver
//!   (`solidb-client`, one connection per worker): begin, the debit and the
//!   credit as queries ending in `RETURN 1`, the ledger row as an insert,
//!   commit. Needs a server that stages writing queries in a driver
//!   transaction (after 2.9.1); 2.9.1 and earlier refuse them.
//! - `run`: the same three statements in one request
//!   (`run_transaction`: begin, steps, commit on the server; the debit and
//!   the credit `require_rows`). Needs a server after 2.9.2.

use super::{Attempt, Res, Totals, Transfer};
use serde_json::{json, Value};
use solidb_client::{
    DriverError, SoliDBClient, SoliDBClientBuilder, TransactionOutcome, TransactionStep,
};
use std::collections::HashMap;
use std::time::Duration;

const DB: &str = "bank";

const Q_DEBIT: &str = "FOR a IN accounts FILTER a._key == @k FILTER a.balance >= @x \
     UPDATE a WITH {balance: a.balance - @x} IN accounts";
const Q_CREDIT: &str =
    "FOR a IN accounts FILTER a._key == @k UPDATE a WITH {balance: a.balance + @x} IN accounts";
const Q_LEDGER: &str =
    "INSERT {_key: @id, w: @w, seq: @seq, src: @src, dst: @dst, amount: @x} INTO transfers";
const Q_DEBIT_RETURN: &str = "FOR a IN accounts FILTER a._key == @k FILTER a.balance >= @x \
     UPDATE a WITH {balance: a.balance - @x} IN accounts RETURN 1";
const Q_CREDIT_RETURN: &str =
    "FOR a IN accounts FILTER a._key == @k UPDATE a WITH {balance: a.balance + @x} IN accounts RETURN 1";
const Q_AUDIT: &str = "FOR a IN accounts COLLECT AGGREGATE s = SUM(a.balance), n = COUNT(), \
     lo = MIN(a.balance) RETURN {s, n, lo}";

pub struct Soli {
    addr: String,
    url: String,
    password: String,
}

/// A worker's connection over the native driver.
pub struct DriverConn {
    c: SoliDBClient,
}

pub struct Conn {
    http: reqwest::Client,
    url: String,
    token: String,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .pool_max_idle_per_host(1)
        .tcp_nodelay(true)
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client")
}

fn key(account: u32) -> String {
    format!("a{account}")
}

/// One request's outcome: the status and body, or no answer.
async fn send(rb: reqwest::RequestBuilder) -> Result<(u16, Value), String> {
    let r = rb.send().await.map_err(|e| e.to_string())?;
    let status = r.status().as_u16();
    let text = r.text().await.map_err(|e| e.to_string())?;
    Ok((
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    ))
}

impl Soli {
    pub fn new(addr: &str, password: &str) -> Self {
        Soli {
            addr: addr.to_string(),
            url: format!("http://{addr}"),
            password: password.to_string(),
        }
    }

    async fn login(&self, http: &reqwest::Client) -> Res<String> {
        let (s, v) =
            send(http.post(format!("{}/auth/login", self.url)).json(
                &json!({"database": "_system", "username": "admin", "password": self.password}),
            ))
            .await?;
        if s != 200 {
            return Err(format!("login: HTTP {s}: {v}").into());
        }
        Ok(v["token"].as_str().ok_or("login: no token")?.to_string())
    }

    pub async fn connect(&self) -> Res<Conn> {
        let http = client();
        let token = self.login(&http).await?;
        Ok(Conn {
            http,
            url: self.url.clone(),
            token,
        })
    }

    pub async fn connect_driver(&self) -> Res<DriverConn> {
        let c = SoliDBClientBuilder::new(&self.addr)
            .auth("_system", "admin", &self.password)
            .pool_size(1)
            .build()
            .await?;
        Ok(DriverConn { c })
    }

    pub async fn ready(&self) -> Res<()> {
        let mut c = self.connect().await?;
        c.cursor_in("_system", "RETURN 1", json!({})).await?;
        Ok(())
    }

    pub async fn reset(&self, accounts: u32, initial: i64) -> Res<()> {
        let c = self.connect().await?;
        let _ = send(
            c.http
                .delete(format!("{}/_api/database/{DB}", c.url))
                .bearer_auth(&c.token),
        )
        .await;
        c.ok(c
            .http
            .post(format!("{}/_api/database", c.url))
            .json(&json!({"name": DB})))
            .await?;
        for coll in ["accounts", "transfers"] {
            c.ok(c
                .http
                .post(format!("{}/_api/database/{DB}/collection", c.url))
                .json(&json!({"name": coll})))
                .await?;
        }
        let mut c = c;
        c.cursor(
            "FOR i IN 0..@last INSERT {_key: CONCAT(\"a\", i), id: i, balance: @b} INTO accounts",
            json!({"last": accounts - 1, "b": initial}),
        )
        .await?;
        Ok(())
    }

    pub async fn balances(&self) -> Res<Vec<(u32, i64)>> {
        let mut c = self.connect().await?;
        let rows = c
            .cursor("FOR a IN accounts RETURN [a.id, a.balance]", json!({}))
            .await?;
        Ok(rows
            .iter()
            .map(|r| (int(&r[0]) as u32, int(&r[1])))
            .collect())
    }

    pub async fn ledger_by_account(&self) -> Res<HashMap<u32, (i64, i64)>> {
        let mut c = self.connect().await?;
        let mut out: HashMap<u32, (i64, i64)> = HashMap::new();
        for r in c
            .cursor(
                "FOR t IN transfers COLLECT a = t.src AGGREGATE s = SUM(t.amount) RETURN [a, s]",
                json!({}),
            )
            .await?
        {
            out.entry(int(&r[0]) as u32).or_default().0 = int(&r[1]);
        }
        for r in c
            .cursor(
                "FOR t IN transfers COLLECT a = t.dst AGGREGATE s = SUM(t.amount) RETURN [a, s]",
                json!({}),
            )
            .await?
        {
            out.entry(int(&r[0]) as u32).or_default().1 = int(&r[1]);
        }
        Ok(out)
    }

    pub async fn ledger_by_worker(&self) -> Res<HashMap<u32, (u64, u64, u64)>> {
        let mut c = self.connect().await?;
        let rows = c
            .cursor(
                "FOR t IN transfers COLLECT w = t.w AGGREGATE n = COUNT(), s = SUM(t.seq), \
                 s2 = SUM((t.seq * t.seq) % 1000003) RETURN [w, n, s, s2]",
                json!({}),
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    int(&r[0]) as u32,
                    (int(&r[1]) as u64, int(&r[2]) as u64, int(&r[3]) as u64),
                )
            })
            .collect())
    }

    pub async fn present(&self, id: &str) -> Res<bool> {
        let c = self.connect().await?;
        let (s, v) = send(
            c.http
                .get(format!(
                    "{}/_api/database/{DB}/document/transfers/{id}",
                    c.url
                ))
                .bearer_auth(&c.token),
        )
        .await?;
        match s {
            200 => Ok(true),
            404 => Ok(false),
            _ => Err(format!("read transfer {id}: HTTP {s}: {v}").into()),
        }
    }
}

/// A JSON number as i64 (sums come back as floats; they are whole).
fn int(v: &Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f.round() as i64))
        .unwrap_or(i64::MIN)
}

impl Conn {
    async fn ok(&self, rb: reqwest::RequestBuilder) -> Res<Value> {
        let (s, v) = send(rb.bearer_auth(&self.token)).await?;
        if !(200..300).contains(&s) {
            return Err(format!("HTTP {s}: {v}").into());
        }
        Ok(v)
    }

    async fn cursor(&mut self, q: &str, vars: Value) -> Res<Vec<Value>> {
        self.cursor_in(DB, q, vars).await
    }

    /// Every row of a query, following the cursor's batches.
    async fn cursor_in(&mut self, db: &str, q: &str, vars: Value) -> Res<Vec<Value>> {
        let mut v = self
            .ok(self
                .http
                .post(format!("{}/_api/database/{db}/cursor", self.url))
                .json(&json!({"query": q, "bindVars": vars, "cache": false, "batchSize": 10000})))
            .await?;
        let mut rows: Vec<Value> = v["result"].as_array().cloned().unwrap_or_default();
        while v["hasMore"].as_bool().or(v["has_more"].as_bool()) == Some(true) {
            let id = v["id"].as_str().ok_or("cursor: no id")?.to_string();
            v = self
                .ok(self.http.put(format!("{}/_api/cursor/{id}", self.url)))
                .await?;
            rows.extend(v["result"].as_array().cloned().unwrap_or_default());
        }
        Ok(rows)
    }

    pub async fn audit(&mut self) -> Res<Totals> {
        let rows = self.cursor(Q_AUDIT, json!({})).await?;
        let r = rows.first().ok_or("audit: no row")?;
        Ok(Totals {
            sum: int(&r["s"]),
            count: int(&r["n"]) as u64,
            min: int(&r["lo"]),
        })
    }

    pub async fn transfer(&mut self, mode: &str, t: &Transfer) -> Attempt {
        let base = format!("{}/_api/database/{DB}/transaction", self.url);
        let begin = send(
            self.http
                .post(format!("{base}/begin"))
                .bearer_auth(&self.token)
                .json(&json!({"isolationLevel": "read_committed"})),
        )
        .await;
        let tx = match begin {
            Err(e) => return Attempt::Lost(e),
            Ok((s, v)) if s != 200 && s != 201 => {
                return Attempt::Failed(format!("begin: HTTP {s}: {v}"))
            }
            Ok((_, v)) => match v["id"].as_str() {
                Some(id) => id.to_string(),
                None => return Attempt::Failed(format!("begin: no id: {v}")),
            },
        };
        let txu = format!("{base}/{tx}");
        let body = match mode {
            "doc" => self.doc_body(&txu, t).await,
            _ => self.query_body(&txu, t).await,
        };
        if let Err(outcome) = body {
            if !matches!(outcome, Attempt::Lost(_)) {
                let _ = self
                    .http
                    .post(format!("{txu}/rollback"))
                    .bearer_auth(&self.token)
                    .send()
                    .await;
            }
            return outcome;
        }
        match send(
            self.http
                .post(format!("{txu}/commit"))
                .bearer_auth(&self.token),
        )
        .await
        {
            Err(e) => Attempt::Unknown(e),
            Ok((s, _)) if (200..300).contains(&s) => Attempt::Committed,
            Ok((409, v)) => Attempt::Conflict(format!("commit: {v}")),
            Ok((s, v)) => Attempt::Failed(format!("commit: HTTP {s}: {v}")),
        }
    }

    /// One request inside the transaction; a refusal or no answer ends it.
    async fn step(&self, rb: reqwest::RequestBuilder, what: &str) -> Result<Value, Attempt> {
        match send(rb.bearer_auth(&self.token)).await {
            Err(e) => Err(Attempt::Lost(e)),
            Ok((s, v)) if (200..300).contains(&s) => Ok(v),
            Ok((409, v)) => Err(Attempt::Conflict(format!("{what}: {v}"))),
            Ok((s, v)) => Err(Attempt::Failed(format!("{what}: HTTP {s}: {v}"))),
        }
    }

    /// `bind_vars`: through 2.9.0 the transactional query read only that
    /// spelling and dropped `bindVars` without a word.
    async fn tx_query(&self, txu: &str, q: &str, vars: Value, what: &str) -> Result<u64, Attempt> {
        let v = self
            .step(
                self.http
                    .post(format!("{txu}/query"))
                    .json(&json!({"query": q, "bind_vars": vars})),
                what,
            )
            .await?;
        Ok(v["mutationCount"].as_u64().unwrap_or(0))
    }

    async fn query_body(&self, txu: &str, t: &Transfer) -> Result<(), Attempt> {
        if self
            .tx_query(
                txu,
                Q_DEBIT,
                json!({"k": key(t.from), "x": t.amount}),
                "debit",
            )
            .await?
            == 0
        {
            return Err(Attempt::Insufficient);
        }
        if self
            .tx_query(
                txu,
                Q_CREDIT,
                json!({"k": key(t.to), "x": t.amount}),
                "credit",
            )
            .await?
            != 1
        {
            return Err(Attempt::Failed("credit matched no account".into()));
        }
        self.tx_query(
            txu,
            Q_LEDGER,
            json!({"id": t.id(), "w": t.w, "seq": t.seq, "src": t.from, "dst": t.to, "x": t.amount}),
            "ledger",
        )
        .await?;
        Ok(())
    }

    async fn doc_body(&self, txu: &str, t: &Transfer) -> Result<(), Attempt> {
        let read = |account: u32| {
            self.step(
                self.http
                    .get(format!("{txu}/document/accounts/{}", key(account))),
                "read",
            )
        };
        let src = int(&read(t.from).await?["balance"]);
        let dst = int(&read(t.to).await?["balance"]);
        if src < t.amount {
            return Err(Attempt::Insufficient);
        }
        for (account, balance) in [(t.from, src - t.amount), (t.to, dst + t.amount)] {
            self.step(
                self.http
                    .put(format!("{txu}/document/accounts/{}", key(account)))
                    .json(&json!({"balance": balance})),
                "write",
            )
            .await?;
        }
        self.step(
            self.http.post(format!("{txu}/document/transfers")).json(&json!({
                "_key": t.id(), "w": t.w, "seq": t.seq, "src": t.from, "dst": t.to, "amount": t.amount,
            })),
            "ledger",
        )
        .await?;
        Ok(())
    }
}

/// A refusal by the concurrency control (a lock held, a stale write) reads
/// "… conflict …"; no answer at all is a connection error.
fn driver_refused(e: DriverError, what: &str) -> Attempt {
    match &e {
        DriverError::ConnectionError(_) => Attempt::Lost(format!("{what}: {e}")),
        _ if e.to_string().to_lowercase().contains("conflict") => {
            Attempt::Conflict(format!("{what}: {e}"))
        }
        _ => Attempt::Failed(format!("{what}: {e}")),
    }
}

/// Where a driver transfer's time goes, step by step, when `BANK_STEPS=1`:
/// (calls, total nanoseconds) for begin, debit, credit, ledger, commit.
static STEPS: std::sync::Mutex<[(u64, u128); 5]> = std::sync::Mutex::new([(0, 0); 5]);
pub const STEP_NAMES: [&str; 5] = ["begin", "debit", "credit", "ledger", "commit"];

fn steps_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("BANK_STEPS").is_ok_and(|v| v == "1"))
}

fn step(i: usize, since: std::time::Instant) -> std::time::Instant {
    let now = std::time::Instant::now();
    if steps_on() {
        let mut s = STEPS.lock().unwrap();
        s[i].0 += 1;
        s[i].1 += (now - since).as_nanos();
    }
    now
}

/// The mean of each step since the last call, and reset.
pub fn take_steps() -> Option<Vec<(&'static str, u64, f64)>> {
    if !steps_on() {
        return None;
    }
    let mut s = STEPS.lock().unwrap();
    let out = STEP_NAMES
        .iter()
        .zip(s.iter())
        .map(|(n, (c, ns))| {
            (
                *n,
                *c,
                if *c > 0 {
                    *ns as f64 / *c as f64 / 1000.0
                } else {
                    0.0
                },
            )
        })
        .collect();
    *s = [(0, 0); 5];
    Some(out)
}

impl DriverConn {
    /// The `run` mode: the whole transfer in one request.
    pub async fn transfer_in_one(&mut self, t: &Transfer) -> Attempt {
        let vars = |k: u32| {
            Some(HashMap::from([
                ("k".to_string(), json!(key(k))),
                ("x".to_string(), json!(t.amount)),
            ]))
        };
        let steps = vec![
            TransactionStep::query(DB, Q_DEBIT_RETURN, vars(t.from)).require_rows(),
            TransactionStep::query(DB, Q_CREDIT_RETURN, vars(t.to)).require_rows(),
            TransactionStep::insert(
                DB,
                "transfers",
                Some(&t.id()),
                json!({"w": t.w, "seq": t.seq, "src": t.from, "dst": t.to, "amount": t.amount}),
            ),
        ];
        match self.c.run_transaction(DB, steps).await {
            Ok(TransactionOutcome::Committed(_)) => Attempt::Committed,
            Ok(TransactionOutcome::Stopped { step: 0 }) => Attempt::Insufficient,
            Ok(TransactionOutcome::Stopped { step }) => {
                Attempt::Failed(format!("step {step} matched nothing"))
            }
            // The request carried the commit: no answer means unknown.
            Err(DriverError::ConnectionError(e)) => Attempt::Unknown(format!("run: {e}")),
            Err(e) => driver_refused(e, "run"),
        }
    }

    pub async fn transfer(&mut self, t: &Transfer) -> Attempt {
        let t0 = std::time::Instant::now();
        if let Err(e) = self.c.begin_transaction(DB, None).await {
            return driver_refused(e, "begin");
        }
        let t1 = step(0, t0);
        if let Err(outcome) = self.body(t, t1).await {
            // Also when the connection is gone: it clears the client's
            // open transaction either way.
            let _ = self.c.rollback().await;
            return outcome;
        }
        let t4 = std::time::Instant::now();
        let committed = self.c.commit().await;
        step(4, t4);
        match committed {
            Ok(()) => Attempt::Committed,
            Err(DriverError::ConnectionError(e)) => Attempt::Unknown(format!("commit: {e}")),
            Err(e) => driver_refused(e, "commit"),
        }
    }

    async fn body(&mut self, t: &Transfer, t1: std::time::Instant) -> Result<(), Attempt> {
        let vars = |k: u32| {
            Some(HashMap::from([
                ("k".to_string(), json!(key(k))),
                ("x".to_string(), json!(t.amount)),
            ]))
        };
        let debited = self
            .c
            .query_with_cache(DB, Q_DEBIT_RETURN, vars(t.from), false)
            .await
            .map_err(|e| driver_refused(e, "debit"))?;
        let t2 = step(1, t1);
        if debited.is_empty() {
            return Err(Attempt::Insufficient);
        }
        let credited = self
            .c
            .query_with_cache(DB, Q_CREDIT_RETURN, vars(t.to), false)
            .await
            .map_err(|e| driver_refused(e, "credit"))?;
        let t3 = step(2, t2);
        if credited.len() != 1 {
            return Err(Attempt::Failed("credit matched no account".into()));
        }
        self.c
            .insert(
                DB,
                "transfers",
                Some(&t.id()),
                json!({"w": t.w, "seq": t.seq, "src": t.from, "dst": t.to, "amount": t.amount}),
            )
            .await
            .map_err(|e| driver_refused(e, "ledger"))?;
        step(3, t3);
        Ok(())
    }
}
