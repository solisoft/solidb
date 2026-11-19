//! One adapter per engine: the same vectors and the same searches, each in
//! the engine's own dialect and through its own client.

use std::future::Future;

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone)]
pub struct Settings {
    pub addr: String,
    pub password: String,
    /// HNSW: neighbours per node, and the candidate list while building.
    pub m: usize,
    pub ef_construction: usize,
    /// SoliDB only: `scalar`, `binary` or `pq` (cards #996, #999, #1000).
    pub quantization: Option<String>,
}

pub trait Engine: Sized + Send + Sync + 'static {
    type Conn: Send + 'static;

    fn open(s: &Settings) -> impl Future<Output = Res<Self>> + Send;
    fn version(&self) -> impl Future<Output = Res<String>> + Send;
    /// An empty collection or table for vectors of `dim` dimensions.
    fn reset(&self, dim: usize) -> impl Future<Output = Res<()>> + Send;
    fn connect(&self) -> impl Future<Output = Res<Self::Conn>> + Send;
    /// Rows `(id, vector)`, in one request; the category is `id % 10`.
    fn insert(
        c: &mut Self::Conn,
        rows: Vec<(u64, Vec<f32>)>,
    ) -> impl Future<Output = Res<()>> + Send;
    /// Build the HNSW index over what was loaded (cosine), and return once
    /// a search uses it.
    fn build_index(&self) -> impl Future<Output = Res<()>> + Send;
    /// The ids of the `k` nearest rows, searching `ef` candidates, among
    /// the rows of category `cat` when given.
    fn search(
        c: &mut Self::Conn,
        q: &[f32],
        k: usize,
        ef: usize,
        cat: Option<u64>,
    ) -> impl Future<Output = Res<Vec<u64>>> + Send;

    fn queries() -> Vec<(&'static str, String)>;
}

pub mod pgvector;
pub mod soli;
