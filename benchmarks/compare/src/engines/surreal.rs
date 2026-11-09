//! SurrealDB through its official Rust SDK over WebSocket, one connection
//! per worker, SurrealQL with bound parameters. An order's customer is a
//! record link computed on write (`DEFINE FIELD customer ... VALUE`), which
//! is how a SurrealDB schema expresses the relation the join follows.
//!
//! Durability is the datastore's `sync` mode, set on the server by run.sh.

use super::{as_f64, as_u64, err, rekey, Engine, Groups, JoinRow, Order, Res, Settings, Table};
use serde_json::{json, Value};
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

const SCHEMA: &[&str] = &[
    "REMOVE TABLE IF EXISTS orders",
    "REMOVE TABLE IF EXISTS customers",
    "DEFINE TABLE orders SCHEMALESS",
    "DEFINE TABLE customers SCHEMALESS",
    "DEFINE FIELD customer ON orders VALUE type::record('customers', customer_id)",
    "DEFINE INDEX idx_customer_id ON orders FIELDS customer_id",
    "DEFINE INDEX idx_created_at ON orders FIELDS created_at",
    "DEFINE INDEX idx_status ON orders FIELDS status",
];
const Q_LOAD_ORDERS: &str = "INSERT INTO orders $docs RETURN NONE";
const Q_LOAD_CUSTOMERS: &str = "INSERT INTO customers $docs RETURN NONE";
const Q_INSERT: &str = "INSERT INTO orders $doc RETURN NONE";
const Q_UPDATE: &str = "UPDATE type::record('orders', $k) SET status = $s RETURN NONE";
const Q_BY_CUSTOMER: &str = "SELECT * FROM orders WHERE customer_id = $c";
const Q_RECENT: &str = "SELECT * FROM orders WHERE created_at >= $lo AND created_at < $hi \
     ORDER BY created_at DESC LIMIT 20";
const Q_JOIN: &str = "SELECT id, amount, customer.name AS name, customer.tier AS tier FROM orders \
     WHERE created_at >= $lo AND created_at < $hi ORDER BY created_at DESC LIMIT 20";
const Q_AGGREGATE: &str =
    "SELECT status, count() AS n, math::sum(amount) AS total FROM orders GROUP BY status";

pub struct SurrealEngine {
    s: Settings,
}

async fn connect(s: &Settings) -> Res<Surreal<Client>> {
    let db = Surreal::new::<Ws>(s.addr.as_str()).await?;
    db.signin(Root {
        username: "root".into(),
        password: s.password.clone(),
    })
    .await?;
    db.use_ns("bench").use_db("bench").await?;
    Ok(db)
}

async fn rows(c: &Surreal<Client>, q: &str, vars: Value) -> Res<Vec<Value>> {
    rows_as(c, q, vars).await
}

/// The first statement's rows, taken as `T`.
async fn rows_as<T: SurrealValue>(c: &Surreal<Client>, q: &str, vars: Value) -> Res<Vec<T>> {
    let mut r = c.query(q).bind(vars).await?.check()?;
    Ok(r.take::<Vec<T>>(0)?)
}

async fn exec(c: &Surreal<Client>, q: &str, vars: Value) -> Res<()> {
    c.query(q).bind(vars).await?.check()?;
    Ok(())
}

impl Engine for SurrealEngine {
    type Conn = Surreal<Client>;

    async fn open(s: &Settings) -> Res<Self> {
        Ok(SurrealEngine { s: s.clone() })
    }

    async fn version(&self) -> Res<String> {
        Ok(connect(&self.s).await?.version().await?.to_string())
    }

    async fn reset(&self) -> Res<()> {
        let c = connect(&self.s).await?;
        for stmt in SCHEMA {
            exec(&c, stmt, json!({})).await?;
        }
        Ok(())
    }

    async fn connect(&self) -> Res<Surreal<Client>> {
        connect(&self.s).await
    }

    async fn load(c: &mut Surreal<Client>, t: Table, docs: Vec<Value>) -> Res<()> {
        let q = match t {
            Table::Orders => Q_LOAD_ORDERS,
            Table::Customers => Q_LOAD_CUSTOMERS,
        };
        let docs: Vec<Value> = docs.into_iter().map(|d| rekey(d, "id")).collect();
        exec(c, q, json!({ "docs": docs })).await
    }

    async fn insert_one(c: &mut Surreal<Client>, doc: Value) -> Res<()> {
        exec(c, Q_INSERT, json!({ "doc": rekey(doc, "id") })).await
    }

    async fn get(c: &mut Surreal<Client>, key: &str) -> Res<Option<String>> {
        let d: Option<Value> = c.select(("orders", key)).await?;
        Ok(d.and_then(|d| d["customer_id"].as_str().map(str::to_string)))
    }

    async fn update_status(c: &mut Surreal<Client>, key: &str, status: &str) -> Res<()> {
        exec(c, Q_UPDATE, json!({"k": key, "s": status})).await
    }

    async fn by_customer(c: &mut Surreal<Client>, customer: &str) -> Res<usize> {
        let r: Vec<Order> = rows_as(c, Q_BY_CUSTOMER, json!({ "c": customer })).await?;
        Ok(r.len())
    }

    async fn recent(c: &mut Surreal<Client>, lo: &str, hi: &str) -> Res<usize> {
        let r: Vec<Order> = rows_as(c, Q_RECENT, json!({"lo": lo, "hi": hi})).await?;
        Ok(r.len())
    }

    async fn join(c: &mut Surreal<Client>, lo: &str, hi: &str) -> Res<usize> {
        let r: Vec<JoinRow> = rows_as(c, Q_JOIN, json!({"lo": lo, "hi": hi})).await?;
        Ok(r.iter().filter(|r| r.name.is_some()).count())
    }

    async fn aggregate(c: &mut Surreal<Client>) -> Res<Groups> {
        let r = rows(c, Q_AGGREGATE, json!({})).await?;
        if r.is_empty() {
            return err("no groups");
        }
        Ok(r.iter()
            .map(|g| {
                (
                    g["status"].as_str().unwrap_or_default().to_string(),
                    as_u64(&g["n"]),
                    as_f64(&g["total"]),
                )
            })
            .collect())
    }

    fn queries(_: &Settings) -> Vec<(&'static str, String)> {
        let mut v = vec![
            ("load", Q_LOAD_ORDERS.to_string()),
            ("insert", Q_INSERT.to_string()),
            ("get", "SDK select((\"orders\", key))".to_string()),
            ("update", Q_UPDATE.to_string()),
            ("by_customer", Q_BY_CUSTOMER.to_string()),
            ("recent", Q_RECENT.to_string()),
            ("join", Q_JOIN.to_string()),
            ("aggregate", Q_AGGREGATE.to_string()),
        ];
        for stmt in SCHEMA {
            v.push(("schema", stmt.to_string()));
        }
        v
    }
}
