//! PostgreSQL with pgvector: a table `items(id, cat, emb vector(dim))`, an
//! HNSW index with `vector_cosine_ops` built after the load (as pgvector
//! advises), searched with `ORDER BY emb <=> $1 LIMIT k` and
//! `hnsw.ef_search` set per search. A filtered search adds `WHERE cat = $2`
//! with `hnsw.iterative_scan = relaxed_order` (pgvector 0.8), so that a
//! selective filter still returns k rows.

use super::{Engine, Res, Settings};
use tokio_postgres::{Client, NoTls};

pub struct PgVector {
    s: Settings,
}

async fn connect(s: &Settings) -> Res<Client> {
    let (host, port) = s.addr.split_once(':').unwrap_or((&s.addr, "5432"));
    let conf = format!(
        "host={host} port={port} user=postgres password={} dbname=postgres",
        s.password
    );
    let (client, conn) = tokio_postgres::connect(&conf, NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("pgvector connection: {e}");
        }
    });
    Ok(client)
}

fn literal(v: &[f32]) -> String {
    let mut s = String::with_capacity(v.len() * 10 + 2);
    s.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&x.to_string());
    }
    s.push(']');
    s
}

const Q_SEARCH: &str = "SELECT id FROM items ORDER BY emb <=> $1::text::vector LIMIT $2";
const Q_FILTERED: &str =
    "SELECT id FROM items WHERE cat = $3 ORDER BY emb <=> $1::text::vector LIMIT $2";

impl Engine for PgVector {
    type Conn = (Client, usize);

    async fn open(s: &Settings) -> Res<Self> {
        Ok(PgVector { s: s.clone() })
    }

    async fn version(&self) -> Res<String> {
        let c = connect(&self.s).await?;
        c.batch_execute("CREATE EXTENSION IF NOT EXISTS vector")
            .await?;
        let row = c
            .query_one(
                "SELECT version() || ', pgvector ' || extversion FROM pg_extension WHERE extname = 'vector'",
                &[],
            )
            .await?;
        Ok(row.get(0))
    }

    async fn reset(&self, dim: usize) -> Res<()> {
        let c = connect(&self.s).await?;
        c.batch_execute(&format!(
            "DROP TABLE IF EXISTS items; \
             CREATE TABLE items (id bigint PRIMARY KEY, cat bigint NOT NULL, emb vector({dim}) NOT NULL)"
        ))
        .await?;
        Ok(())
    }

    async fn connect(&self) -> Res<(Client, usize)> {
        let c = connect(&self.s).await?;
        c.batch_execute("SET hnsw.iterative_scan = relaxed_order")
            .await?;
        Ok((c, 0))
    }

    async fn insert(c: &mut (Client, usize), rows: Vec<(u64, Vec<f32>)>) -> Res<()> {
        let mut sql = String::from("INSERT INTO items (id, cat, emb) VALUES ");
        for (i, (id, v)) in rows.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            sql.push_str(&format!(
                "({id},{},'{}')",
                id % crate::CATEGORIES,
                literal(v)
            ));
        }
        c.0.batch_execute(&sql).await?;
        Ok(())
    }

    async fn build_index(&self) -> Res<()> {
        let c = connect(&self.s).await?;
        c.batch_execute(&format!(
            "SET maintenance_work_mem = '4GB'; SET max_parallel_maintenance_workers = 7; \
             CREATE INDEX ON items USING hnsw (emb vector_cosine_ops) WITH (m = {}, ef_construction = {}); \
             CREATE INDEX ON items (cat); ANALYZE items",
            self.s.m, self.s.ef_construction
        ))
        .await?;
        Ok(())
    }

    async fn search(
        c: &mut (Client, usize),
        q: &[f32],
        k: usize,
        ef: usize,
        cat: Option<u64>,
    ) -> Res<Vec<u64>> {
        if c.1 != ef {
            c.0.batch_execute(&format!("SET hnsw.ef_search = {ef}"))
                .await?;
            c.1 = ef;
        }
        let v = literal(q);
        let k = k as i64;
        let rows = match cat {
            Some(cat) => c.0.query(Q_FILTERED, &[&v, &k, &(cat as i64)]).await?,
            None => c.0.query(Q_SEARCH, &[&v, &k]).await?,
        };
        Ok(rows.iter().map(|r| r.get::<_, i64>(0) as u64).collect())
    }

    fn queries() -> Vec<(&'static str, String)> {
        vec![("search", Q_SEARCH.into()), ("filtered", Q_FILTERED.into())]
    }
}
