//! ArangoDB through its HTTP API (what its official drivers speak): AQL
//! with bind variables through `/_api/cursor`, one keep-alive pool.

use super::{err, Engine, Knows, Person, Res, Settings, Tally};
use reqwest::{Client, Method, RequestBuilder};
use serde_json::{json, Value};

fn q_hops(depth: u32) -> String {
    format!(
        "FOR v IN 1..{depth} OUTBOUND @start knows \
         OPTIONS {{ uniqueVertices: \"global\", order: \"bfs\" }} \
         COLLECT AGGREGATE c = COUNT(1), s = SUM(v.n) RETURN [c, s]"
    )
}
const Q_FILTERED: &str = "LET ns = (FOR v, e IN 2..2 OUTBOUND @start knows \
     OPTIONS { uniqueVertices: \"path\" } \
     FILTER e.since >= @year AND v.country == @country RETURN DISTINCT v.n) \
     RETURN [LENGTH(ns), SUM(ns)]";
const Q_FOF: &str = "LET buyers = (FOR b IN 1..1 INBOUND @product bought RETURN b.n) \
     LET ns = (FOR v IN 1..2 OUTBOUND @start knows \
       OPTIONS { uniqueVertices: \"global\", order: \"bfs\" } \
       FILTER v.n IN buyers RETURN v.n) \
     RETURN [LENGTH(ns), SUM(ns)]";
const Q_SHORTEST: &str = "LET p = (FOR v IN OUTBOUND SHORTEST_PATH @from TO @to knows RETURN 1) \
     RETURN LENGTH(p) - 1";

pub struct Arango {
    conn: Conn,
    base: String,
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
        match r.get("result") {
            Some(Value::Array(rows)) => Ok(rows.clone()),
            _ => err(format!("no result: {r}")),
        }
    }

    async fn insert(&self, collection: &str, docs: Vec<Value>) -> Res<()> {
        let r: Value = self
            .req(Method::POST, &format!("/_api/document/{collection}?silent=true"))
            .json(&docs)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        // With `silent`, the reply lists only the documents that failed.
        match r.as_array() {
            Some(failed) if !failed.is_empty() => err(format!("insert failed: {}", failed[0])),
            _ => Ok(()),
        }
    }
}

fn tally(rows: Vec<Value>) -> Res<Tally> {
    let Some(row) = rows.first().and_then(Value::as_array) else {
        return err(format!("not a [count, sum] row: {rows:?}"));
    };
    let n = |v: Option<&Value>| v.and_then(Value::as_f64).unwrap_or(0.0) as u64;
    Ok((n(row.first()), n(row.get(1))))
}

fn person(n: u32) -> String {
    format!("people/{n}")
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
        let conn = Conn {
            http,
            db: format!("{base}/_db/graph"),
            jwt,
        };
        Ok(Arango { conn, base })
    }

    async fn version(&self) -> Res<String> {
        let v: Value = self
            .conn
            .http
            .get(format!("{}/_api/version", self.base))
            .bearer_auth(&self.conn.jwt)
            .send()
            .await?
            .json()
            .await?;
        Ok(v["version"].as_str().unwrap_or("?").to_string())
    }

    async fn reset(&self) -> Res<()> {
        let system = |m: Method, path: &str| {
            self.conn
                .http
                .request(m, format!("{}{path}", self.base))
                .bearer_auth(&self.conn.jwt)
        };
        system(Method::DELETE, "/_api/database/graph").send().await?;
        system(Method::POST, "/_api/database")
            .json(&json!({"name": "graph"}))
            .send()
            .await?
            .error_for_status()?;
        for (name, kind) in [("people", 2), ("products", 2), ("knows", 3), ("bought", 3)] {
            self.conn
                .req(Method::POST, "/_api/collection")
                .json(&json!({"name": name, "type": kind}))
                .send()
                .await?
                .error_for_status()?;
        }
        Ok(())
    }

    async fn connect(&self) -> Res<Conn> {
        Ok(self.conn.clone())
    }

    async fn load_people(c: &mut Conn, rows: Vec<Person>) -> Res<()> {
        let docs = rows
            .into_iter()
            .map(|(n, country, age)| json!({"_key": n.to_string(), "n": n, "country": country, "age": age}))
            .collect();
        c.insert("people", docs).await
    }

    async fn load_products(c: &mut Conn, rows: Vec<u32>) -> Res<()> {
        let docs = rows
            .into_iter()
            .map(|n| json!({"_key": n.to_string(), "n": n}))
            .collect();
        c.insert("products", docs).await
    }

    async fn load_knows(c: &mut Conn, rows: Vec<Knows>) -> Res<()> {
        let docs = rows
            .into_iter()
            .map(|(from, to, since, weight)| {
                json!({"_from": person(from), "_to": person(to), "since": since, "weight": weight})
            })
            .collect();
        c.insert("knows", docs).await
    }

    async fn load_bought(c: &mut Conn, rows: Vec<(u32, u32)>) -> Res<()> {
        let docs = rows
            .into_iter()
            .map(|(p, product)| json!({"_from": person(p), "_to": format!("products/{product}")}))
            .collect();
        c.insert("bought", docs).await
    }

    async fn after_load(&self) -> Res<()> {
        // Edge collections carry their edge index from creation.
        Ok(())
    }

    async fn hops(c: &mut Conn, start: u32, depth: u32) -> Res<Tally> {
        tally(c.cursor(&q_hops(depth), json!({"start": person(start)})).await?)
    }

    async fn filtered(c: &mut Conn, start: u32, year: i64, country: &str) -> Res<Tally> {
        let vars = json!({"start": person(start), "year": year, "country": country});
        tally(c.cursor(Q_FILTERED, vars).await?)
    }

    async fn fof_bought(c: &mut Conn, start: u32, product: u32) -> Res<Tally> {
        let vars = json!({"start": person(start), "product": format!("products/{product}")});
        tally(c.cursor(Q_FOF, vars).await?)
    }

    async fn shortest(c: &mut Conn, from: u32, to: u32) -> Res<Option<u64>> {
        let rows = c
            .cursor(Q_SHORTEST, json!({"from": person(from), "to": person(to)}))
            .await?;
        // No path: zero vertices, so -1.
        Ok(rows.first().and_then(Value::as_i64).filter(|l| *l >= 0).map(|l| l as u64))
    }

    fn queries(_: &Settings) -> Vec<(&'static str, String)> {
        vec![
            ("hop1", q_hops(1)),
            ("hop2", q_hops(2)),
            ("hop3", q_hops(3)),
            ("filtered", Q_FILTERED.into()),
            ("fof_bought", Q_FOF.into()),
            ("shortest", Q_SHORTEST.into()),
        ]
    }
}
