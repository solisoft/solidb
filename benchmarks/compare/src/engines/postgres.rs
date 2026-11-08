//! PostgreSQL through tokio-postgres, one connection per worker, every
//! statement prepared once per connection. Two schemas:
//!
//! - **jsonb** (the default): `orders(key text PRIMARY KEY, doc jsonb)`, with
//!   btree expression indexes on the fields the queries filter on. The same
//!   document model as every other engine here.
//! - **typed** (`--typed`): real columns for the scalar fields, jsonb for
//!   the nested ones. The schema a PostgreSQL user would design.
//!
//! Durability is the server's `synchronous_commit`, set on its command line
//! by run.sh, not by this client.

use super::{as_f64, err, Engine, Groups, Res, Settings, Table};
use serde_json::Value;
use tokio_postgres::{Client, NoTls, Statement};

pub struct Postgres {
    s: Settings,
}

pub struct Conn {
    c: Client,
    load_orders: Statement,
    load_customers: Statement,
    insert: Statement,
    get: Statement,
    update: Statement,
    by_customer: Statement,
    recent: Statement,
    join: Statement,
    aggregate: Statement,
}

struct Sql {
    load_orders: &'static str,
    load_customers: &'static str,
    insert: &'static str,
    get: &'static str,
    update: &'static str,
    by_customer: &'static str,
    recent: &'static str,
    join: &'static str,
    aggregate: &'static str,
    schema: &'static [&'static str],
}

const JSONB: Sql = Sql {
    load_orders: "INSERT INTO orders (key, doc) SELECT * FROM unnest($1::text[], $2::jsonb[])",
    load_customers: "INSERT INTO customers (key, doc) SELECT * FROM unnest($1::text[], $2::jsonb[])",
    insert: "INSERT INTO orders (key, doc) VALUES ($1, $2)",
    get: "SELECT doc FROM orders WHERE key = $1",
    update: "UPDATE orders SET doc = jsonb_set(doc, '{status}', to_jsonb($2::text)) WHERE key = $1",
    by_customer: "SELECT doc FROM orders WHERE doc->>'customer_id' = $1",
    recent: "SELECT doc FROM orders WHERE doc->>'created_at' >= $1 AND doc->>'created_at' < $2 \
             ORDER BY doc->>'created_at' DESC LIMIT 20",
    join: "SELECT o.key, (o.doc->>'amount')::float8 AS amount, c.doc->>'name' AS name, c.doc->>'tier' AS tier \
           FROM (SELECT key, doc FROM orders WHERE doc->>'created_at' >= $1 AND doc->>'created_at' < $2 \
                 ORDER BY doc->>'created_at' DESC LIMIT 20) o \
           JOIN customers c ON c.key = o.doc->>'customer_id'",
    aggregate: "SELECT doc->>'status' AS status, count(*) AS n, sum((doc->>'amount')::float8) AS total \
                FROM orders GROUP BY 1",
    schema: &[
        "CREATE TABLE orders (key text PRIMARY KEY, doc jsonb NOT NULL)",
        "CREATE TABLE customers (key text PRIMARY KEY, doc jsonb NOT NULL)",
        "CREATE INDEX idx_customer_id ON orders ((doc->>'customer_id'))",
        "CREATE INDEX idx_created_at ON orders ((doc->>'created_at'))",
        "CREATE INDEX idx_status ON orders ((doc->>'status'))",
    ],
};

const TYPED: Sql = Sql {
    load_orders: "INSERT INTO orders SELECT * FROM unnest($1::text[], $2::text[], $3::text[], $4::float8[], \
                  $5::text[], $6::text[], $7::text[], $8::text[], $9::jsonb[], $10::jsonb[], $11::jsonb[], $12::text[])",
    load_customers: "INSERT INTO customers SELECT * FROM unnest($1::text[], $2::text[], $3::text[], $4::text[], \
                     $5::text[], $6::text[])",
    insert: "INSERT INTO orders VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    get: "SELECT * FROM orders WHERE key = $1",
    update: "UPDATE orders SET status = $2 WHERE key = $1",
    by_customer: "SELECT * FROM orders WHERE customer_id = $1",
    recent: "SELECT * FROM orders WHERE created_at >= $1 AND created_at < $2 ORDER BY created_at DESC LIMIT 20",
    join: "SELECT o.key, o.amount, c.name, c.tier \
           FROM (SELECT key, amount, customer_id FROM orders WHERE created_at >= $1 AND created_at < $2 \
                 ORDER BY created_at DESC LIMIT 20) o \
           JOIN customers c ON c.key = o.customer_id",
    aggregate: "SELECT status, count(*) AS n, sum(amount) AS total FROM orders GROUP BY status",
    schema: &[
        "CREATE TABLE orders (key text PRIMARY KEY, customer_id text NOT NULL, status text NOT NULL, \
         amount float8 NOT NULL, currency text NOT NULL, created_at text NOT NULL, country text NOT NULL, \
         channel text NOT NULL, tags jsonb NOT NULL, items jsonb NOT NULL, address jsonb NOT NULL, note text NOT NULL)",
        "CREATE TABLE customers (key text PRIMARY KEY, name text NOT NULL, email text NOT NULL, \
         tier text NOT NULL, country text NOT NULL, since text NOT NULL)",
        "CREATE INDEX idx_customer_id ON orders (customer_id)",
        "CREATE INDEX idx_created_at ON orders (created_at)",
        "CREATE INDEX idx_status ON orders (status)",
    ],
};

fn sql(s: &Settings) -> &'static Sql {
    if s.typed {
        &TYPED
    } else {
        &JSONB
    }
}

async fn client(s: &Settings) -> Res<Client> {
    let (host, port) = s.addr.split_once(':').unwrap_or((&s.addr, "5432"));
    let cfg = format!(
        "host={host} port={port} user=postgres password={} dbname=postgres",
        s.password
    );
    let (client, conn) = tokio_postgres::connect(&cfg, NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("postgres connection: {e}");
        }
    });
    Ok(client)
}

fn s(v: &Value, f: &str) -> String {
    v[f].as_str().unwrap_or_default().to_string()
}

/// Read every column of a typed row, so that the client pays the decoding
/// it pays for a jsonb document (the other drivers decode whole documents).
fn decode_typed(r: &tokio_postgres::Row) -> String {
    let customer: String = r.get("customer_id");
    let _: (String, String, f64, String) = (
        r.get("key"),
        r.get("status"),
        r.get("amount"),
        r.get("created_at"),
    );
    let _: (Value, Value, Value) = (r.get("tags"), r.get("items"), r.get("address"));
    customer
}

impl Engine for Postgres {
    type Conn = Conn;

    async fn open(s: &Settings) -> Res<Self> {
        Ok(Postgres { s: s.clone() })
    }

    async fn version(&self) -> Res<String> {
        let c = client(&self.s).await?;
        Ok(c.query_one("SHOW server_version", &[]).await?.get(0))
    }

    async fn reset(&self) -> Res<()> {
        let c = client(&self.s).await?;
        c.batch_execute("DROP TABLE IF EXISTS orders; DROP TABLE IF EXISTS customers")
            .await?;
        for stmt in sql(&self.s).schema {
            c.batch_execute(stmt).await?;
        }
        Ok(())
    }

    async fn connect(&self) -> Res<Conn> {
        let c = client(&self.s).await?;
        let q = sql(&self.s);
        Ok(Conn {
            load_orders: c.prepare(q.load_orders).await?,
            load_customers: c.prepare(q.load_customers).await?,
            insert: c.prepare(q.insert).await?,
            get: c.prepare(q.get).await?,
            update: c.prepare(q.update).await?,
            by_customer: c.prepare(q.by_customer).await?,
            recent: c.prepare(q.recent).await?,
            join: c.prepare(q.join).await?,
            aggregate: c.prepare(q.aggregate).await?,
            c,
        })
    }

    async fn load(c: &mut Conn, t: Table, docs: Vec<Value>) -> Res<()> {
        let typed = c.load_orders.params().len() > 2;
        let keys: Vec<String> = docs.iter().map(|d| s(d, "key")).collect();
        let n = match (t, typed) {
            (Table::Orders, false) | (Table::Customers, false) => {
                let stmt = if matches!(t, Table::Orders) {
                    &c.load_orders
                } else {
                    &c.load_customers
                };
                let docs: Vec<Value> = docs
                    .into_iter()
                    .map(|mut d| {
                        d.as_object_mut().map(|o| o.remove("key"));
                        d
                    })
                    .collect();
                c.c.execute(stmt, &[&keys, &docs]).await?
            }
            (Table::Orders, true) => {
                let col = |f: &str| docs.iter().map(|d| s(d, f)).collect::<Vec<String>>();
                let json = |f: &str| docs.iter().map(|d| d[f].clone()).collect::<Vec<Value>>();
                let amount: Vec<f64> = docs.iter().map(|d| as_f64(&d["amount"])).collect();
                c.c.execute(
                    &c.load_orders,
                    &[
                        &keys,
                        &col("customer_id"),
                        &col("status"),
                        &amount,
                        &col("currency"),
                        &col("created_at"),
                        &col("country"),
                        &col("channel"),
                        &json("tags"),
                        &json("items"),
                        &json("address"),
                        &col("note"),
                    ],
                )
                .await?
            }
            (Table::Customers, true) => {
                let col = |f: &str| docs.iter().map(|d| s(d, f)).collect::<Vec<String>>();
                c.c.execute(
                    &c.load_customers,
                    &[
                        &keys,
                        &col("name"),
                        &col("email"),
                        &col("tier"),
                        &col("country"),
                        &col("since"),
                    ],
                )
                .await?
            }
        };
        if n as usize != keys.len() {
            return err(format!("inserted {n} of {}", keys.len()));
        }
        Ok(())
    }

    async fn insert_one(c: &mut Conn, mut doc: Value) -> Res<()> {
        let key = s(&doc, "key");
        if c.insert.params().len() == 2 {
            doc.as_object_mut().map(|o| o.remove("key"));
            c.c.execute(&c.insert, &[&key, &doc]).await?;
        } else {
            let d = &doc;
            c.c.execute(
                &c.insert,
                &[
                    &key,
                    &s(d, "customer_id"),
                    &s(d, "status"),
                    &as_f64(&d["amount"]),
                    &s(d, "currency"),
                    &s(d, "created_at"),
                    &s(d, "country"),
                    &s(d, "channel"),
                    &d["tags"],
                    &d["items"],
                    &d["address"],
                    &s(d, "note"),
                ],
            )
            .await?;
        }
        Ok(())
    }

    async fn get(c: &mut Conn, key: &str) -> Res<Option<String>> {
        let row = c.c.query_opt(&c.get, &[&key]).await?;
        Ok(row.map(|r| {
            if r.columns().len() == 1 {
                let doc: Value = r.get(0);
                s(&doc, "customer_id")
            } else {
                decode_typed(&r)
            }
        }))
    }

    async fn update_status(c: &mut Conn, key: &str, status: &str) -> Res<()> {
        let n = c.c.execute(&c.update, &[&key, &status]).await?;
        if n != 1 {
            return err(format!("updated {n} rows"));
        }
        Ok(())
    }

    async fn by_customer(c: &mut Conn, customer: &str) -> Res<usize> {
        let rows = c.c.query(&c.by_customer, &[&customer]).await?;
        Ok(rows.iter().map(decode_row).count())
    }

    async fn recent(c: &mut Conn, lo: &str, hi: &str) -> Res<usize> {
        let rows = c.c.query(&c.recent, &[&lo, &hi]).await?;
        Ok(rows.iter().map(decode_row).count())
    }

    async fn join(c: &mut Conn, lo: &str, hi: &str) -> Res<usize> {
        let rows = c.c.query(&c.join, &[&lo, &hi]).await?;
        Ok(rows
            .iter()
            .filter(|r| {
                let _: (String, f64, Option<String>) =
                    (r.get("key"), r.get("amount"), r.get("tier"));
                r.get::<_, Option<String>>("name").is_some()
            })
            .count())
    }

    async fn aggregate(c: &mut Conn) -> Res<Groups> {
        let rows = c.c.query(&c.aggregate, &[]).await?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get::<_, String>("status"),
                    r.get::<_, i64>("n") as u64,
                    r.get::<_, f64>("total"),
                )
            })
            .collect())
    }

    fn queries(s: &Settings) -> Vec<(&'static str, String)> {
        let q = sql(s);
        let mut v = vec![
            ("load", q.load_orders.to_string()),
            ("insert", q.insert.to_string()),
            ("get", q.get.to_string()),
            ("update", q.update.to_string()),
            ("by_customer", q.by_customer.to_string()),
            ("recent", q.recent.to_string()),
            ("join", q.join.to_string()),
            ("aggregate", q.aggregate.to_string()),
        ];
        for stmt in q.schema {
            v.push(("schema", stmt.to_string()));
        }
        v
    }
}

/// Decode a returned order the way the client would use it.
fn decode_row(r: &tokio_postgres::Row) -> String {
    if r.columns().len() == 1 {
        let doc: Value = r.get(0);
        s(&doc, "customer_id")
    } else {
        decode_typed(r)
    }
}
