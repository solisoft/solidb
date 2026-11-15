//! One adapter per database: the same points and the same questions, each
//! in the database's own dialect and through its own client.

use super::Point;
use std::future::Future;

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn err<T>(msg: impl Into<String>) -> Res<T> {
    Err(msg.into().into())
}

#[derive(Clone)]
pub struct Settings {
    pub addr: String,
    pub password: String,
}

pub trait Engine: Sized + Send + Sync + 'static {
    type Conn: Send + 'static;

    fn open(s: &Settings) -> impl Future<Output = Res<Self>> + Send;
    fn version(&self) -> impl Future<Output = Res<String>> + Send;
    /// An empty table or collection, with the indexes the questions need.
    fn reset(&self) -> impl Future<Output = Res<()>> + Send;
    fn connect(&self) -> impl Future<Output = Res<Self::Conn>> + Send;
    /// Whatever has to happen before `points` points can be read back.
    fn after_load(&self, points: u64) -> impl Future<Output = Res<()>> + Send;

    /// Insert points in one request.
    fn insert(c: &mut Self::Conn, points: Vec<Point>) -> impl Future<Output = Res<()>> + Send;
    /// Count and sum of one series over `[lo, hi)` (milliseconds).
    fn range(
        c: &mut Self::Conn,
        series: u32,
        lo: i64,
        hi: i64,
    ) -> impl Future<Output = Res<(u64, f64)>> + Send;
    /// The five-minute averages of one series over `[lo, hi)`: how many,
    /// and their sum.
    fn bucket(
        c: &mut Self::Conn,
        series: u32,
        lo: i64,
        hi: i64,
    ) -> impl Future<Output = Res<(u64, f64)>> + Send;
    /// The last point of one series: its instant (milliseconds) and value.
    fn last(c: &mut Self::Conn, series: u32) -> impl Future<Output = Res<(i64, f64)>> + Send;
    /// Count and sum of every series over `[lo, hi)`.
    fn window_all(
        c: &mut Self::Conn,
        lo: i64,
        hi: i64,
    ) -> impl Future<Output = Res<(u64, f64)>> + Send;

    fn queries() -> Vec<(&'static str, String)>;
}

pub mod questdb;
pub mod soli;
pub mod timescale;
