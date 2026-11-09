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

use super::{Attempt, Res, Totals, Transfer};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

const DB: &str = "bank";

const Q_DEBIT: &str = "FOR a IN accounts FILTER a._key == @k FILTER a.balance >= @x \
     UPDATE a WITH {balance: a.balance - @x} IN accounts";
const Q_CREDIT: &str =
    "FOR a IN accounts FILTER a._key == @k UPDATE a WITH {balance: a.balance + @x} IN accounts";
const Q_LEDGER: &str =
    "INSERT {_key: @id, w: @w, seq: @seq, src: @src, dst: @dst, amount: @x} INTO transfers";
const Q_AUDIT: &str = "FOR a IN accounts COLLECT AGGREGATE s = SUM(a.balance), n = COUNT(), \
     lo = MIN(a.balance) RETURN {s, n, lo}";

pub struct Soli {
    url: String,
    password: String,
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
                 s2 = SUM(t.seq * t.seq) RETURN [w, n, s, s2]",
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
