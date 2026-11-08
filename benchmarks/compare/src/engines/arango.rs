//! ArangoDB through its HTTP API, which is also what its official drivers
//! (JavaScript, Go, Java, Python) speak; there is no maintained Rust driver.
//! One reqwest client with a keep-alive pool as large as the concurrency, a
//! JWT obtained once, and AQL with bind variables through `/_api/cursor`.
//!
//! Writes return before they are synced unless the collection has
//! `waitForSync`, which `--durable` sets.

use super::{as_f64, as_u64, err, rekey, Engine, Groups, Res, Settings, Table};
use reqwest::{Client, Method, RequestBuilder};
use serde_json::{json, Value};

const Q_BY_CUSTOMER: &str = "FOR o IN orders FILTER o.customer_id == @c RETURN o";
const Q_RECENT: &str = "FOR o IN orders FILTER o.created_at >= @lo AND o.created_at < @hi \
     SORT o.created_at DESC LIMIT 20 RETURN o";
const Q_JOIN: &str = "FOR o IN orders FILTER o.created_at >= @lo AND o.created_at < @hi \
     SORT o.created_at DESC LIMIT 20 \
     LET c = DOCUMENT(CONCAT(\"customers/\", o.customer_id)) \
     RETURN {order: o._key, amount: o.amount, name: c.name, tier: c.tier}";
const Q_AGGREGATE: &str = "FOR o IN orders COLLECT status = o.status \
     AGGREGATE n = COUNT(1), total = SUM(o.amount) RETURN {status, n, total}";

pub struct Arango {
    http: Client,
    base: String,
    jwt: String,
    durable: bool,
}

#[derive(Clone)]
pub struct Conn {
    http: Client,
    db: String,
    jwt: String,
}

impl Conn {
    fn req(&self, m: Method, path: &str) -> RequestBuilder {
        self.http
            .request(m, format!("{}{path}", self.db))
            .bearer_auth(&self.jwt)
    }

    async fn cursor(&self, query: &str, vars: Value) -> Res<Vec<Value>> {
        let r: Value = self
            .req(Method::POST, "/_api/cursor")
            .json(&json!({"query": query, "bindVars": vars, "batchSize": 1000}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if r["hasMore"].as_bool() == Some(true) {
            return err("cursor has more than one batch");
        }
        match r["result"].as_array() {
            Some(rows) => Ok(rows.clone()),
            None => err(format!("no result: {r}")),
        }
    }
}

impl Arango {
    fn req(&self, m: Method, path: &str) -> RequestBuilder {
        self.http
            .request(m, format!("{}{path}", self.base))
            .bearer_auth(&self.jwt)
    }
}

impl Engine for Arango {
    type Conn = Conn;

    async fn open(s: &Settings) -> Res<Self> {
        let http = Client::builder()
            .pool_max_idle_per_host(64)
            .tcp_nodelay(true)
            .build()?;
        let base = format!("http://{}", s.addr);
        let auth: Value = http
            .post(format!("{base}/_open/auth"))
            .json(&json!({"username": "root", "password": s.password}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let jwt = auth["jwt"].as_str().ok_or("no jwt")?.to_string();
        Ok(Arango {
            http,
            base,
            jwt,
            durable: s.durable,
        })
    }

    async fn version(&self) -> Res<String> {
        let v: Value = self
            .req(Method::GET, "/_api/version")
            .send()
            .await?
            .json()
            .await?;
        Ok(v["version"].as_str().unwrap_or("unknown").to_string())
    }

    async fn reset(&self) -> Res<()> {
        self.req(Method::DELETE, "/_api/database/bench")
            .send()
            .await?;
        self.req(Method::POST, "/_api/database")
            .json(&json!({"name": "bench"}))
            .send()
            .await?
            .error_for_status()?;
        for name in ["orders", "customers"] {
            self.req(Method::POST, "/_db/bench/_api/collection")
                .json(&json!({"name": name, "waitForSync": self.durable}))
                .send()
                .await?
                .error_for_status()?;
        }
        for f in ["customer_id", "created_at", "status"] {
            self.req(Method::POST, "/_db/bench/_api/index?collection=orders")
                .json(&json!({"type": "persistent", "fields": [f]}))
                .send()
                .await?
                .error_for_status()?;
        }
        Ok(())
    }

    async fn connect(&self) -> Res<Conn> {
        Ok(Conn {
            http: self.http.clone(),
            db: format!("{}/_db/bench", self.base),
            jwt: self.jwt.clone(),
        })
    }

    async fn load(c: &mut Conn, t: Table, docs: Vec<Value>) -> Res<()> {
        let coll = match t {
            Table::Orders => "orders",
            Table::Customers => "customers",
        };
        let docs: Vec<Value> = docs.into_iter().map(|d| rekey(d, "_key")).collect();
        let r = c
            .req(Method::POST, &format!("/_api/document/{coll}?silent=true"))
            .json(&docs)
            .send()
            .await?
            .error_for_status()?;
        if let Some(codes) = r.headers().get("x-arango-error-codes") {
            return err(format!("batch errors: {codes:?}"));
        }
        Ok(())
    }

    async fn insert_one(c: &mut Conn, doc: Value) -> Res<()> {
        c.req(Method::POST, "/_api/document/orders")
            .json(&rekey(doc, "_key"))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn get(c: &mut Conn, key: &str) -> Res<Option<String>> {
        let r = c
            .req(Method::GET, &format!("/_api/document/orders/{key}"))
            .send()
            .await?;
        if r.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let d: Value = r.error_for_status()?.json().await?;
        Ok(d["customer_id"].as_str().map(str::to_string))
    }

    async fn update_status(c: &mut Conn, key: &str, status: &str) -> Res<()> {
        c.req(Method::PATCH, &format!("/_api/document/orders/{key}"))
            .json(&json!({ "status": status }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn by_customer(c: &mut Conn, customer: &str) -> Res<usize> {
        Ok(c.cursor(Q_BY_CUSTOMER, json!({"c": customer})).await?.len())
    }

    async fn recent(c: &mut Conn, lo: &str, hi: &str) -> Res<usize> {
        Ok(c.cursor(Q_RECENT, json!({"lo": lo, "hi": hi})).await?.len())
    }

    async fn join(c: &mut Conn, lo: &str, hi: &str) -> Res<usize> {
        let rows = c.cursor(Q_JOIN, json!({"lo": lo, "hi": hi})).await?;
        Ok(rows.iter().filter(|r| r["name"].is_string()).count())
    }

    async fn aggregate(c: &mut Conn) -> Res<Groups> {
        let rows = c.cursor(Q_AGGREGATE, json!({})).await?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r["status"].as_str().unwrap_or_default().to_string(),
                    as_u64(&r["n"]),
                    as_f64(&r["total"]),
                )
            })
            .collect())
    }

    fn queries(s: &Settings) -> Vec<(&'static str, String)> {
        vec![
            (
                "load",
                "POST /_db/bench/_api/document/orders?silent=true [1,000 documents]".into(),
            ),
            (
                "insert",
                "POST /_db/bench/_api/document/orders {document}".into(),
            ),
            ("get", "GET /_db/bench/_api/document/orders/{key}".into()),
            (
                "update",
                "PATCH /_db/bench/_api/document/orders/{key} {status}".into(),
            ),
            ("by_customer", Q_BY_CUSTOMER.into()),
            ("recent", Q_RECENT.into()),
            ("join", Q_JOIN.into()),
            ("aggregate", Q_AGGREGATE.into()),
            (
                "indexes",
                "persistent indexes on orders: customer_id, created_at, status".into(),
            ),
            ("collections", format!("waitForSync: {}", s.durable)),
        ]
    }
}
