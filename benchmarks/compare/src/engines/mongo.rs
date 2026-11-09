//! MongoDB through its official Rust driver. The driver pools connections
//! itself, so every worker shares one `Client` whose pool is as large as
//! the concurrency; a worker's "connection" is a collection handle.
//!
//! A standalone server acknowledges `w:1` writes before they reach the
//! journal; `--durable` asks for `j:true` on every write.

use super::{err, rekey, Engine, Groups, JoinRow, Order, Res, Settings, Table};
use futures::TryStreamExt;
use mongodb::bson::{doc, Bson, Document};
use mongodb::options::{CollectionOptions, WriteConcern};
use mongodb::{Client, Collection, IndexModel};
use serde_json::Value;

pub struct Mongo {
    client: Client,
    durable: bool,
}

pub struct Conn {
    orders: Collection<Document>,
    /// The same collection, read into the shared `Order`.
    typed: Collection<Order>,
    customers: Collection<Document>,
}

fn pipeline_join(lo: &str, hi: &str) -> Vec<Document> {
    vec![
        doc! {"$match": {"created_at": {"$gte": lo, "$lt": hi}}},
        doc! {"$sort": {"created_at": -1}},
        doc! {"$limit": 20},
        doc! {"$lookup": {"from": "customers", "localField": "customer_id", "foreignField": "_id", "as": "c"}},
        doc! {"$unwind": "$c"},
        doc! {"$project": {"_id": 1, "amount": 1, "name": "$c.name", "tier": "$c.tier"}},
    ]
}

fn pipeline_aggregate() -> Vec<Document> {
    vec![doc! {"$group": {"_id": "$status", "n": {"$sum": 1}, "total": {"$sum": "$amount"}}}]
}

fn to_bson(v: Value) -> Res<Document> {
    Ok(mongodb::bson::to_document(&rekey(v, "_id"))?)
}

fn int(b: Option<&Bson>) -> u64 {
    match b {
        Some(Bson::Int32(n)) => *n as u64,
        Some(Bson::Int64(n)) => *n as u64,
        Some(Bson::Double(f)) => *f as u64,
        _ => 0,
    }
}

impl Engine for Mongo {
    type Conn = Conn;

    async fn open(s: &Settings) -> Res<Self> {
        // A pool as large as the largest concurrency run, so no worker waits
        // for a connection the driver will not open.
        let uri = format!("mongodb://{}/?maxPoolSize=64&minPoolSize=0", s.addr);
        Ok(Mongo {
            client: Client::with_uri_str(&uri).await?,
            durable: s.durable,
        })
    }

    async fn version(&self) -> Res<String> {
        let info = self
            .client
            .database("admin")
            .run_command(doc! {"buildInfo": 1})
            .await?;
        Ok(info.get_str("version").unwrap_or("unknown").to_string())
    }

    async fn reset(&self) -> Res<()> {
        let db = self.client.database("bench");
        db.drop().await?;
        db.create_collection("orders").await?;
        db.create_collection("customers").await?;
        let orders = db.collection::<Document>("orders");
        for f in ["customer_id", "created_at", "status"] {
            orders
                .create_index(IndexModel::builder().keys(doc! {f: 1}).build())
                .await?;
        }
        Ok(())
    }

    async fn connect(&self) -> Res<Conn> {
        let db = self.client.database("bench");
        let opts = if self.durable {
            CollectionOptions::builder()
                .write_concern(
                    WriteConcern::builder()
                        .w(mongodb::options::Acknowledgment::Nodes(1))
                        .journal(true)
                        .build(),
                )
                .build()
        } else {
            CollectionOptions::default()
        };
        Ok(Conn {
            orders: db.collection_with_options("orders", opts.clone()),
            typed: db.collection_with_options("orders", opts.clone()),
            customers: db.collection_with_options("customers", opts),
        })
    }

    async fn load(c: &mut Conn, t: Table, docs: Vec<Value>) -> Res<()> {
        let n = docs.len();
        let docs = docs.into_iter().map(to_bson).collect::<Res<Vec<_>>>()?;
        let coll = match t {
            Table::Orders => &c.orders,
            Table::Customers => &c.customers,
        };
        let r = coll.insert_many(docs).ordered(false).await?;
        if r.inserted_ids.len() != n {
            return err(format!("inserted {} of {n}", r.inserted_ids.len()));
        }
        Ok(())
    }

    async fn insert_one(c: &mut Conn, doc: Value) -> Res<()> {
        c.orders.insert_one(to_bson(doc)?).await?;
        Ok(())
    }

    async fn get(c: &mut Conn, key: &str) -> Res<Option<String>> {
        let d = c.orders.find_one(doc! {"_id": key}).await?;
        Ok(d.and_then(|d| d.get_str("customer_id").ok().map(str::to_string)))
    }

    async fn update_status(c: &mut Conn, key: &str, status: &str) -> Res<()> {
        let r = c
            .orders
            .update_one(doc! {"_id": key}, doc! {"$set": {"status": status}})
            .await?;
        if r.matched_count != 1 {
            return err(format!("matched {} documents", r.matched_count));
        }
        Ok(())
    }

    async fn by_customer(c: &mut Conn, customer: &str) -> Res<usize> {
        let docs: Vec<Order> = c
            .typed
            .find(doc! {"customer_id": customer})
            .await?
            .try_collect()
            .await?;
        Ok(docs.len())
    }

    async fn recent(c: &mut Conn, lo: &str, hi: &str) -> Res<usize> {
        let docs: Vec<Order> = c
            .typed
            .find(doc! {"created_at": {"$gte": lo, "$lt": hi}})
            .sort(doc! {"created_at": -1})
            .limit(20)
            .await?
            .try_collect()
            .await?;
        Ok(docs.len())
    }

    async fn join(c: &mut Conn, lo: &str, hi: &str) -> Res<usize> {
        let rows: Vec<JoinRow> = c
            .orders
            .aggregate(pipeline_join(lo, hi))
            .with_type::<JoinRow>()
            .await?
            .try_collect()
            .await?;
        Ok(rows.iter().filter(|r| r.name.is_some()).count())
    }

    async fn aggregate(c: &mut Conn) -> Res<Groups> {
        let docs: Vec<Document> = c
            .orders
            .aggregate(pipeline_aggregate())
            .await?
            .try_collect()
            .await?;
        Ok(docs
            .iter()
            .map(|d| {
                let total = match d.get("total") {
                    Some(Bson::Double(f)) => *f,
                    other => int(other) as f64,
                };
                (
                    d.get_str("_id").unwrap_or_default().to_string(),
                    int(d.get("n")),
                    total,
                )
            })
            .collect())
    }

    fn queries(s: &Settings) -> Vec<(&'static str, String)> {
        let wc = if s.durable {
            "{w: 1, j: true}"
        } else {
            "server default (w: 1)"
        };
        vec![
            (
                "load",
                format!(
                    "orders.insertMany([1,000 documents], {{ordered: false}}), write concern {wc}"
                ),
            ),
            (
                "insert",
                format!("orders.insertOne(document), write concern {wc}"),
            ),
            ("get", "orders.findOne({_id: key})".into()),
            (
                "update",
                format!("orders.updateOne({{_id: key}}, {{$set: {{status}}}}), write concern {wc}"),
            ),
            ("by_customer", "orders.find({customer_id: c})".into()),
            (
                "recent",
                "orders.find({created_at: {$gte: lo, $lt: hi}}).sort({created_at: -1}).limit(20)"
                    .into(),
            ),
            (
                "join",
                format!(
                    "orders.aggregate({})",
                    Bson::from(
                        pipeline_join("lo", "hi")
                            .into_iter()
                            .map(Bson::Document)
                            .collect::<Vec<_>>()
                    )
                ),
            ),
            (
                "aggregate",
                format!(
                    "orders.aggregate({})",
                    Bson::from(
                        pipeline_aggregate()
                            .into_iter()
                            .map(Bson::Document)
                            .collect::<Vec<_>>()
                    )
                ),
            ),
            (
                "indexes",
                "orders.createIndex({customer_id: 1}), ({created_at: 1}), ({status: 1})".into(),
            ),
        ]
    }
}
