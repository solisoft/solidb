//! One module per database. Each turns the same operations into that
//! database's own idiom, through its own client, and returns what the
//! correctness guard needs to check the answer.

use serde_json::Value;
use std::future::Future;

pub mod arango;
pub mod mongo;
pub mod postgres;
pub mod soli;
pub mod surreal;

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn err<T>(msg: impl Into<String>) -> Res<T> {
    Err(msg.into().into())
}

/// Settings every engine reads; each takes what applies to it.
#[derive(Clone)]
pub struct Settings {
    pub addr: String,
    pub password: String,
    /// Make every write durable before it is acknowledged, where the engine
    /// lets a client or a collection ask for it.
    pub durable: bool,
    /// postgres only: real columns instead of one jsonb column.
    pub typed: bool,
}

#[derive(Clone, Copy)]
pub enum Table {
    Orders,
    Customers,
}

/// (status, count, sum of amounts) per group.
pub type Groups = Vec<(String, u64, f64)>;

pub trait Engine: Sized + Send + Sync + 'static {
    type Conn: Send + 'static;

    fn open(s: &Settings) -> impl Future<Output = Res<Self>> + Send;
    /// The server's own version string.
    fn version(&self) -> impl Future<Output = Res<String>> + Send;
    /// Drop and recreate both collections and their indexes. Indexes exist
    /// before the load, as they do in a running application.
    fn reset(&self) -> impl Future<Output = Res<()>> + Send;
    /// One connection per worker.
    fn connect(&self) -> impl Future<Output = Res<Self::Conn>> + Send;

    /// Insert a batch of documents in one request (each has a `key` field).
    fn load(c: &mut Self::Conn, t: Table, docs: Vec<Value>)
        -> impl Future<Output = Res<()>> + Send;
    fn insert_one(c: &mut Self::Conn, doc: Value) -> impl Future<Output = Res<()>> + Send;
    /// The document's `customer_id`, read back to check it is the right one.
    fn get(c: &mut Self::Conn, key: &str) -> impl Future<Output = Res<Option<String>>> + Send;
    fn update_status(
        c: &mut Self::Conn,
        key: &str,
        status: &str,
    ) -> impl Future<Output = Res<()>> + Send;
    /// Every order of a customer: the number of rows returned.
    fn by_customer(c: &mut Self::Conn, customer: &str) -> impl Future<Output = Res<usize>> + Send;
    /// The 20 most recent orders in `[lo, hi)`: the rows returned.
    fn recent(c: &mut Self::Conn, lo: &str, hi: &str) -> impl Future<Output = Res<usize>> + Send;
    /// The same 20 orders, each with its customer's name: the rows that
    /// came back with a name.
    fn join(c: &mut Self::Conn, lo: &str, hi: &str) -> impl Future<Output = Res<usize>> + Send;
    /// Count and total amount per status, over the whole collection.
    fn aggregate(c: &mut Self::Conn) -> impl Future<Output = Res<Groups>> + Send;

    /// The query text of each scenario, as sent, for the published results.
    fn queries(s: &Settings) -> Vec<(&'static str, String)>;
}

/// A JSON number or string as f64 (Arango and SoliDB return sums as
/// numbers; nothing here should return them as strings, but be lenient).
pub fn as_f64(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => s.parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

pub fn as_u64(v: &Value) -> u64 {
    match v {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().map(|f| f as u64))
            .unwrap_or(0),
        _ => 0,
    }
}

/// Rename the generator's `key` field to the engine's identifier field.
pub fn rekey(mut doc: Value, field: &str) -> Value {
    if let Some(obj) = doc.as_object_mut() {
        if let Some(k) = obj.remove("key") {
            obj.insert(field.to_string(), k);
        }
    }
    doc
}
