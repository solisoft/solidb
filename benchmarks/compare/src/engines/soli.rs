//! SoliDB through its native driver (MessagePack over TCP), the transport
//! the Soli framework uses. Queries are SDBQL with bind variables, sent with
//! the query cache off so that every request is executed.

use super::{as_f64, as_u64, err, rekey, Engine, Groups, JoinRow, Order, Res, Settings, Table};
use serde_json::{json, Value};
use solidb_client::{SoliDBClient, SoliDBClientBuilder};
use std::collections::HashMap;
use std::time::Duration;

const DB: &str = "bench";

const Q_BY_CUSTOMER: &str = "FOR o IN orders FILTER o.customer_id == @c RETURN o";
const Q_RECENT: &str = "FOR o IN orders FILTER o.created_at >= @lo AND o.created_at < @hi \
     SORT o.created_at DESC LIMIT 20 RETURN o";
const Q_JOIN: &str = "FOR o IN orders FILTER o.created_at >= @lo AND o.created_at < @hi \
     SORT o.created_at DESC LIMIT 20 \
     LET c = DOCUMENT(CONCAT(\"customers/\", o.customer_id)) \
     RETURN {order: o._key, amount: o.amount, name: c.name, tier: c.tier}";
const Q_AGGREGATE: &str = "FOR o IN orders COLLECT status = o.status \
     AGGREGATE n = COUNT(), total = SUM(o.amount) RETURN {status, n, total}";

pub struct Soli {
    s: Settings,
}

/// Connect, retrying: a server under load drops connections it cannot
/// dispatch in time ("driver dispatch channel full").
async fn connect(s: &Settings) -> Res<SoliDBClient> {
    let mut attempt = 0;
    loop {
        match SoliDBClientBuilder::new(&s.addr)
            .auth("_system", "admin", &s.password)
            .pool_size(1)
            .build()
            .await
        {
            Ok(c) => return Ok(c),
            Err(e) if attempt < 50 => {
                attempt += 1;
                eprintln!("solidb connect attempt {attempt}: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => return Err(e.to_string().into()),
        }
    }
}

async fn query(c: &mut SoliDBClient, q: &str, vars: Vec<(&str, Value)>) -> Res<Vec<Value>> {
    let vars: HashMap<String, Value> = vars.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    Ok(c.query_with_cache(DB, q, Some(vars), false).await?)
}

/// Rows deserialized straight from the driver's MessagePack into `T`.
async fn query_as<T: serde::de::DeserializeOwned>(
    c: &mut SoliDBClient,
    q: &str,
    vars: Vec<(&str, Value)>,
) -> Res<Vec<T>> {
    let vars: HashMap<String, Value> = vars.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    Ok(c.query_as(DB, q, Some(vars), false).await?)
}

impl Engine for Soli {
    type Conn = SoliDBClient;

    async fn open(s: &Settings) -> Res<Self> {
        Ok(Soli { s: s.clone() })
    }

    async fn version(&self) -> Res<String> {
        // No endpoint reports it; run.sh records `solidb --version` from the
        // image in the result's `meta`.
        Ok("see meta.server_version".into())
    }

    async fn reset(&self) -> Res<()> {
        let mut c = connect(&self.s).await?;
        c.delete_database(DB).await.ok();
        c.create_database(DB).await?;
        for t in ["orders", "customers"] {
            c.create_collection(DB, t, None).await?;
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
            .await?;
        }
        Ok(())
    }

    async fn connect(&self) -> Res<SoliDBClient> {
        connect(&self.s).await
    }

    async fn load(c: &mut SoliDBClient, t: Table, docs: Vec<Value>) -> Res<()> {
        let coll = match t {
            Table::Orders => "orders",
            Table::Customers => "customers",
        };
        let n = docs.len();
        let docs: Vec<Value> = docs.into_iter().map(|d| rekey(d, "_key")).collect();
        let inserted = c.bulk_insert(DB, coll, docs).await?;
        if inserted != n {
            return err(format!("bulk_insert stored {inserted} of {n}"));
        }
        Ok(())
    }

    async fn insert_one(c: &mut SoliDBClient, doc: Value) -> Res<()> {
        let key = doc["key"].as_str().unwrap_or_default().to_string();
        let mut doc = doc;
        doc.as_object_mut().map(|o| o.remove("key"));
        c.insert(DB, "orders", Some(&key), doc).await?;
        Ok(())
    }

    async fn get(c: &mut SoliDBClient, key: &str) -> Res<Option<String>> {
        let doc = c.get(DB, "orders", key).await?;
        Ok(doc
            .get("customer_id")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    async fn update_status(c: &mut SoliDBClient, key: &str, status: &str) -> Res<()> {
        c.update(DB, "orders", key, json!({ "status": status }), true)
            .await?;
        Ok(())
    }

    async fn by_customer(c: &mut SoliDBClient, customer: &str) -> Res<usize> {
        let rows: Vec<Order> = query_as(c, Q_BY_CUSTOMER, vec![("c", json!(customer))]).await?;
        Ok(rows.len())
    }

    async fn recent(c: &mut SoliDBClient, lo: &str, hi: &str) -> Res<usize> {
        let rows: Vec<Order> =
            query_as(c, Q_RECENT, vec![("lo", json!(lo)), ("hi", json!(hi))]).await?;
        Ok(rows.len())
    }

    async fn join(c: &mut SoliDBClient, lo: &str, hi: &str) -> Res<usize> {
        let rows: Vec<JoinRow> =
            query_as(c, Q_JOIN, vec![("lo", json!(lo)), ("hi", json!(hi))]).await?;
        Ok(rows.iter().filter(|r| r.name.is_some()).count())
    }

    async fn aggregate(c: &mut SoliDBClient) -> Res<Groups> {
        let rows = query(c, Q_AGGREGATE, vec![]).await?;
        Ok(rows
            .iter()
            .map(|r| {
                let s = r["status"].as_str().unwrap_or_default().to_string();
                (s, as_u64(&r["n"]), as_f64(&r["total"]))
            })
            .collect())
    }

    fn queries(_: &Settings) -> Vec<(&'static str, String)> {
        vec![
            (
                "get",
                "driver Get {database, collection: \"orders\", key}".into(),
            ),
            (
                "update",
                "driver Update {collection: \"orders\", key, document: {status}, merge: true}"
                    .into(),
            ),
            (
                "insert",
                "driver Insert {collection: \"orders\", key, document}".into(),
            ),
            (
                "load",
                "driver BulkInsert {collection, documents: [1,000 documents]}".into(),
            ),
            ("by_customer", Q_BY_CUSTOMER.into()),
            ("recent", Q_RECENT.into()),
            ("join", Q_JOIN.into()),
            ("aggregate", Q_AGGREGATE.into()),
            (
                "indexes",
                "persistent indexes on orders: customer_id, created_at, status".into(),
            ),
        ]
    }
}
