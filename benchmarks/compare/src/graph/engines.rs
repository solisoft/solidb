//! One adapter per database: the same graph and the same questions, each in
//! the database's own query language and through its own client.

use std::future::Future;

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn err<T>(msg: impl Into<String>) -> Res<T> {
    Err(msg.into().into())
}

#[derive(Clone)]
pub struct Settings {
    pub addr: String,
    pub password: String,
    /// bolt only: `neo4j` or `memgraph` (their index and shortest-path
    /// syntaxes differ).
    pub flavor: String,
}

/// A person: number, country, age.
pub type Person = (u32, &'static str, i64);
/// An edge between people: from, to, since (a year), weight.
pub type Knows = (u32, u32, i64, i64);

/// `(count, sum of the people's numbers)`.
pub type Tally = (u64, u64);

pub trait Engine: Sized + Send + Sync + 'static {
    type Conn: Send + 'static;

    fn open(s: &Settings) -> impl Future<Output = Res<Self>> + Send;
    fn version(&self) -> impl Future<Output = Res<String>> + Send;
    /// Empty collections or tables, with the indexes a lookup by number
    /// needs. Edge indexes may wait for `after_load`.
    fn reset(&self) -> impl Future<Output = Res<()>> + Send;
    fn connect(&self) -> impl Future<Output = Res<Self::Conn>> + Send;

    fn load_people(c: &mut Self::Conn, rows: Vec<Person>) -> impl Future<Output = Res<()>> + Send;
    fn load_products(c: &mut Self::Conn, rows: Vec<u32>) -> impl Future<Output = Res<()>> + Send;
    fn load_knows(c: &mut Self::Conn, rows: Vec<Knows>) -> impl Future<Output = Res<()>> + Send;
    fn load_bought(
        c: &mut Self::Conn,
        rows: Vec<(u32, u32)>,
    ) -> impl Future<Output = Res<()>> + Send;
    /// Whatever the engine does once the data is in: build the edge
    /// indexes, analyze.
    fn after_load(&self) -> impl Future<Output = Res<()>> + Send;

    /// The distinct people within `depth` hops (1 to 3) of `start`, other
    /// than `start`.
    fn hops(c: &mut Self::Conn, start: u32, depth: u32) -> impl Future<Output = Res<Tally>> + Send;
    /// The distinct people, other than `start`, two hops away through a
    /// second edge dating from `year` or later, who live in `country`.
    fn filtered(
        c: &mut Self::Conn,
        start: u32,
        year: i64,
        country: &str,
    ) -> impl Future<Output = Res<Tally>> + Send;
    /// The distinct people within two hops of `start` who bought `product`.
    fn fof_bought(
        c: &mut Self::Conn,
        start: u32,
        product: u32,
    ) -> impl Future<Output = Res<Tally>> + Send;
    /// The number of edges on a shortest path from `from` to `to`.
    fn shortest(
        c: &mut Self::Conn,
        from: u32,
        to: u32,
    ) -> impl Future<Output = Res<Option<u64>>> + Send;

    /// Set the weight of the edge from `from` to `to`: the write of the
    /// mixed runs (`--writes`), which changes no scenario's answer.
    fn reweigh(
        c: &mut Self::Conn,
        from: u32,
        to: u32,
        weight: i64,
    ) -> impl Future<Output = Res<()>> + Send;

    /// The query text of each scenario, as sent.
    fn queries(s: &Settings) -> Vec<(&'static str, String)>;
}

pub mod arango;
pub mod bolt;
pub mod postgres;
pub mod soli;
