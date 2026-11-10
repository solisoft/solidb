//! ArangoDB through its HTTP API, which is what its official drivers speak.
//! One HTTP/1.1 keep-alive connection per worker, a JWT obtained once per
//! connection. Both collections are created with `waitForSync`, and every
//! transaction asks for it as well: a commit is synced before it is
//! acknowledged, as PostgreSQL's `synchronous_commit=on` and SoliDB's
//! `SOLIDB_DURABILITY=full`.
//!
//! Two ways to move money:
//!
//! - `stream`: a stream transaction, five requests. `POST
//!   /_api/transaction/begin` declaring both collections for writing, then
//!   under `x-arango-trx-id` the conditional debit and the credit as AQL
//!   (`FOR a IN accounts FILTER a._key == @k FILTER a.balance >= @x UPDATE
//!   a WITH {balance: a.balance - @x} IN accounts RETURN 1`) and the ledger
//!   row as a document insert, then `PUT /_api/transaction/{id}`. The shape
//!   of SoliDB's `query` mode.
//! - `js`: the same three statements in a JavaScript transaction, one
//!   request (`POST /_api/transaction` with an `action`), the shape of
//!   SoliDB's `run_transaction` and of PostgreSQL's function.
//!
//! Two transactions that write the same document do not wait for each
//! other: the second is refused with a write-write conflict (error 1200),
//! and the transfer is retried.

use super::{Attempt, Res, Totals, Transfer};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

const DB: &str = "bank";
const COLLECTIONS: [&str; 2] = ["accounts", "transfers"];

const Q_DEBIT: &str = "FOR a IN accounts FILTER a._key == @k FILTER a.balance >= @x \
     UPDATE a WITH {balance: a.balance - @x} IN accounts RETURN 1";
const Q_CREDIT: &str = "FOR a IN accounts FILTER a._key == @k \
     UPDATE a WITH {balance: a.balance + @x} IN accounts RETURN 1";
const Q_AUDIT: &str = "FOR a IN accounts COLLECT AGGREGATE s = SUM(a.balance), n = COUNT(1), \
     lo = MIN(a.balance) RETURN {s, n, lo}";

/// The `js` mode's transaction: false when the debit matched nothing (and
/// nothing was written), true once all three statements ran.
const JS_TRANSFER: &str = r#"function (p) {
  const db = require("@arangodb").db;
  const debited = db._query(
    "FOR a IN accounts FILTER a._key == @k FILTER a.balance >= @x UPDATE a WITH {balance: a.balance - @x} IN accounts RETURN 1",
    {k: p.from, x: p.x}).toArray();
  if (debited.length === 0) { return false; }
  db._query(
    "FOR a IN accounts FILTER a._key == @k UPDATE a WITH {balance: a.balance + @x} IN accounts",
    {k: p.to, x: p.x});
  db._query(
    "INSERT {_key: @id, w: @w, seq: @seq, src: @src, dst: @dst, amount: @x} INTO transfers",
    {id: p.id, w: p.w, seq: p.seq, src: p.src, dst: p.dst, x: p.x});
  return true;
}"#;

/// Write-write conflict, and a lock that was not had in time.
const CONFLICT_ERRORS: [i64; 2] = [1200, 18];

pub struct Arango {
    url: String,
    password: String,
}

pub struct Conn {
    http: reqwest::Client,
    /// `http://host:port/_db/bank`.
    db: String,
    jwt: String,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .pool_max_idle_per_host(1)
        .tcp_nodelay(true)
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client")
}

/// Whether the collections are created with `waitForSync` (the default).
/// `BANK_ARANGO_COLLECTION_SYNC=0` leaves the sync to the transactions'
/// own `waitForSync`, to check the two do not add up.
fn collection_sync() -> bool {
    std::env::var("BANK_ARANGO_COLLECTION_SYNC").map_or(true, |v| v != "0")
}

fn key(account: u32) -> String {
    format!("a{account}")
}

fn int(v: &Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f.round() as i64))
        .unwrap_or(i64::MIN)
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

/// A refusal as an attempt's outcome: a conflict to retry, or a failure.
fn refused(what: &str, status: u16, body: &Value) -> Attempt {
    let conflict = body["errorNum"]
        .as_i64()
        .is_some_and(|n| CONFLICT_ERRORS.contains(&n));
    if conflict || status == 409 {
        Attempt::Conflict(format!("{what}: {body}"))
    } else {
        Attempt::Failed(format!("{what}: HTTP {status}: {body}"))
    }
}

impl Arango {
    pub fn new(addr: &str, password: &str) -> Self {
        Arango {
            url: format!("http://{addr}"),
            password: password.to_string(),
        }
    }

    pub async fn connect(&self) -> Res<Conn> {
        let http = client();
        let (s, v) = send(
            http.post(format!("{}/_open/auth", self.url))
                .json(&json!({"username": "root", "password": self.password})),
        )
        .await?;
        if s != 200 {
            return Err(format!("login: HTTP {s}: {v}").into());
        }
        Ok(Conn {
            http,
            db: format!("{}/_db/{DB}", self.url),
            jwt: v["jwt"].as_str().ok_or("login: no jwt")?.to_string(),
        })
    }

    pub async fn ready(&self) -> Res<()> {
        let c = self.connect().await?;
        c.ok(c.http.get(format!("{}/_api/version", self.url)))
            .await?;
        Ok(())
    }

    pub async fn reset(&self, accounts: u32, initial: i64) -> Res<()> {
        let c = self.connect().await?;
        let system = format!("{}/_api/database", self.url);
        let _ = send(c.http.delete(format!("{system}/{DB}")).bearer_auth(&c.jwt)).await;
        c.ok(c.http.post(&system).json(&json!({"name": DB})))
            .await?;
        for name in COLLECTIONS {
            c.ok(c
                .http
                .post(format!("{}/_api/collection", c.db))
                .json(&json!({"name": name, "waitForSync": collection_sync()})))
                .await?;
        }
        c.cursor(
            "FOR i IN 0..@last INSERT {_key: CONCAT(\"a\", i), id: i, balance: @b} INTO accounts",
            json!({"last": accounts - 1, "b": initial}),
        )
        .await?;
        Ok(())
    }

    pub async fn balances(&self) -> Res<Vec<(u32, i64)>> {
        let c = self.connect().await?;
        let rows = c
            .cursor("FOR a IN accounts RETURN [a.id, a.balance]", json!({}))
            .await?;
        Ok(rows
            .iter()
            .map(|r| (int(&r[0]) as u32, int(&r[1])))
            .collect())
    }

    /// Per account: (debited, credited) by the ledger.
    pub async fn ledger_by_account(&self) -> Res<HashMap<u32, (i64, i64)>> {
        let c = self.connect().await?;
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

    /// Per worker: (rows, sum of seq, sum of seq² mod 1,000,003).
    pub async fn ledger_by_worker(&self) -> Res<HashMap<u32, (u64, u64, u64)>> {
        let c = self.connect().await?;
        let rows = c
            .cursor(
                "FOR t IN transfers COLLECT w = t.w AGGREGATE n = COUNT(1), s = SUM(t.seq), \
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
                .get(format!("{}/_api/document/transfers/{id}", c.db))
                .bearer_auth(&c.jwt),
        )
        .await?;
        match s {
            200 => Ok(true),
            404 => Ok(false),
            _ => Err(format!("read transfer {id}: HTTP {s}: {v}").into()),
        }
    }
}

impl Conn {
    async fn ok(&self, rb: reqwest::RequestBuilder) -> Res<Value> {
        let (s, v) = send(rb.bearer_auth(&self.jwt)).await?;
        if !(200..300).contains(&s) {
            return Err(format!("HTTP {s}: {v}").into());
        }
        Ok(v)
    }

    /// Every row of a query outside any transaction.
    async fn cursor(&self, q: &str, vars: Value) -> Res<Vec<Value>> {
        let mut v = self
            .ok(self
                .http
                .post(format!("{}/_api/cursor", self.db))
                .json(&json!({"query": q, "bindVars": vars, "batchSize": 10000})))
            .await?;
        let mut rows: Vec<Value> = v["result"].as_array().cloned().unwrap_or_default();
        while v["hasMore"].as_bool() == Some(true) {
            let id = v["id"].as_str().ok_or("cursor: no id")?.to_string();
            v = self
                .ok(self.http.post(format!("{}/_api/cursor/{id}", self.db)))
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
        if mode == "js" {
            return self.transfer_in_one(t).await;
        }
        let begin = send(
            self.http
                .post(format!("{}/_api/transaction/begin", self.db))
                .bearer_auth(&self.jwt)
                .json(&json!({"collections": {"write": COLLECTIONS}, "waitForSync": true})),
        )
        .await;
        let tx = match begin {
            Err(e) => return Attempt::Lost(e),
            Ok((s, v)) if !(200..300).contains(&s) => return refused("begin", s, &v),
            Ok((_, v)) => match v["result"]["id"].as_str() {
                Some(id) => id.to_string(),
                None => return Attempt::Failed(format!("begin: no id: {v}")),
            },
        };
        let end = format!("{}/_api/transaction/{tx}", self.db);
        if let Err(outcome) = self.stream_body(&tx, t).await {
            if !matches!(outcome, Attempt::Lost(_)) {
                let _ = self.http.delete(&end).bearer_auth(&self.jwt).send().await;
            }
            return outcome;
        }
        match send(self.http.put(&end).bearer_auth(&self.jwt)).await {
            Err(e) => Attempt::Unknown(e),
            Ok((s, _)) if (200..300).contains(&s) => Attempt::Committed,
            Ok((s, v)) => refused("commit", s, &v),
        }
    }

    /// One request of a stream transaction: its body, or how it was refused.
    async fn step(
        &self,
        tx: &str,
        rb: reqwest::RequestBuilder,
        what: &str,
    ) -> Result<Value, Attempt> {
        match send(rb.bearer_auth(&self.jwt).header("x-arango-trx-id", tx)).await {
            Err(e) => Err(Attempt::Lost(e)),
            Ok((s, v)) if (200..300).contains(&s) => Ok(v),
            Ok((s, v)) => Err(refused(what, s, &v)),
        }
    }

    /// Rows a statement of the transaction returned.
    async fn tx_query(&self, tx: &str, q: &str, vars: Value, what: &str) -> Result<usize, Attempt> {
        let v = self
            .step(
                tx,
                self.http
                    .post(format!("{}/_api/cursor", self.db))
                    .json(&json!({"query": q, "bindVars": vars})),
                what,
            )
            .await?;
        Ok(v["result"].as_array().map_or(0, Vec::len))
    }

    async fn stream_body(&self, tx: &str, t: &Transfer) -> Result<(), Attempt> {
        let debited = self
            .tx_query(
                tx,
                Q_DEBIT,
                json!({"k": key(t.from), "x": t.amount}),
                "debit",
            )
            .await?;
        if debited == 0 {
            return Err(Attempt::Insufficient);
        }
        let credited = self
            .tx_query(
                tx,
                Q_CREDIT,
                json!({"k": key(t.to), "x": t.amount}),
                "credit",
            )
            .await?;
        if credited == 0 {
            return Err(Attempt::Failed("credit: account missing".into()));
        }
        self.step(
            tx,
            self.http
                .post(format!("{}/_api/document/transfers", self.db))
                .json(&json!({
                    "_key": t.id(), "w": t.w, "seq": t.seq,
                    "src": t.from, "dst": t.to, "amount": t.amount,
                })),
            "ledger",
        )
        .await?;
        Ok(())
    }

    /// The `js` mode: one request, begun and committed by the server.
    async fn transfer_in_one(&mut self, t: &Transfer) -> Attempt {
        let sent = send(
            self.http
                .post(format!("{}/_api/transaction", self.db))
                .bearer_auth(&self.jwt)
                .json(&json!({
                    "collections": {"write": COLLECTIONS},
                    "waitForSync": true,
                    "action": JS_TRANSFER,
                    "params": {
                        "from": key(t.from), "to": key(t.to), "x": t.amount,
                        "id": t.id(), "w": t.w, "seq": t.seq, "src": t.from, "dst": t.to,
                    },
                })),
        )
        .await;
        match sent {
            // The request carried the commit: no answer means unknown.
            Err(e) => Attempt::Unknown(e),
            Ok((s, v)) if (200..300).contains(&s) => match v["result"].as_bool() {
                Some(true) => Attempt::Committed,
                Some(false) => Attempt::Insufficient,
                None => Attempt::Failed(format!("js: no result: {v}")),
            },
            Ok((s, v)) => refused("js", s, &v),
        }
    }
}
