//! TimescaleDB through tokio-postgres: a hypertable on `ts`, an index on
//! `(series, ts DESC)`, prepared statements.

use super::{Engine, Res, Settings};
use crate::{name, Point};
use tokio_postgres::{Client, NoTls, Statement};

const Q_RANGE: &str = "SELECT count(*), coalesce(sum(v), 0)::float8 FROM points \
     WHERE series = $1 AND ts >= to_timestamp($2) AND ts < to_timestamp($3)";
const Q_BUCKET: &str = "SELECT avg(v)::float8 FROM points \
     WHERE series = $1 AND ts >= to_timestamp($2) AND ts < to_timestamp($3) \
     GROUP BY time_bucket('5 minutes', ts)";
const Q_LAST: &str = "SELECT (extract(epoch FROM ts) * 1000)::bigint, v FROM points \
     WHERE series = $1 ORDER BY ts DESC LIMIT 1";
const Q_WINDOW: &str = "SELECT count(*), coalesce(sum(v), 0)::float8 FROM points \
     WHERE ts >= to_timestamp($1) AND ts < to_timestamp($2)";
const Q_INSERT: &str = "INSERT INTO points \
     SELECT s, to_timestamp(t), v FROM unnest($1::text[], $2::float8[], $3::float8[]) AS u(s, t, v)";

pub struct Timescale {
    s: Settings,
}

pub struct Conn {
    client: Client,
    range: Statement,
    bucket: Statement,
    last: Statement,
    window: Statement,
    insert: Statement,
}

async fn client(s: &Settings) -> Res<Client> {
    let (host, port) = s.addr.split_once(':').unwrap_or((&s.addr, "5432"));
    let config = format!(
        "host={host} port={port} user=postgres password={} dbname=postgres",
        s.password
    );
    let (client, connection) = tokio_postgres::connect(&config, NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("timescale connection: {e}");
        }
    });
    Ok(client)
}

fn secs(ms: i64) -> f64 {
    ms as f64 / 1000.0
}

impl Engine for Timescale {
    type Conn = Conn;

    async fn open(s: &Settings) -> Res<Self> {
        Ok(Timescale { s: s.clone() })
    }

    async fn version(&self) -> Res<String> {
        let c = client(&self.s).await?;
        let pg: String = c.query_one("SHOW server_version", &[]).await?.get(0);
        let ts: String = c
            .query_one(
                "SELECT extversion FROM pg_extension WHERE extname = 'timescaledb'",
                &[],
            )
            .await?
            .get(0);
        Ok(format!("TimescaleDB {ts} on PostgreSQL {pg}"))
    }

    async fn reset(&self) -> Res<()> {
        let c = client(&self.s).await?;
        c.batch_execute(
            "CREATE EXTENSION IF NOT EXISTS timescaledb;
             DROP TABLE IF EXISTS points;
             CREATE TABLE points (series text NOT NULL, ts timestamptz NOT NULL,
                                  v double precision NOT NULL);
             SELECT create_hypertable('points', 'ts');
             CREATE INDEX points_series_ts ON points (series, ts DESC);",
        )
        .await?;
        Ok(())
    }

    async fn connect(&self) -> Res<Conn> {
        let client = client(&self.s).await?;
        Ok(Conn {
            range: client.prepare(Q_RANGE).await?,
            bucket: client.prepare(Q_BUCKET).await?,
            last: client.prepare(Q_LAST).await?,
            window: client.prepare(Q_WINDOW).await?,
            insert: client.prepare(Q_INSERT).await?,
            client,
        })
    }

    async fn after_load(&self, _: u64) -> Res<()> {
        client(&self.s)
            .await?
            .batch_execute("ANALYZE points")
            .await?;
        Ok(())
    }

    async fn insert(c: &mut Conn, points: Vec<Point>) -> Res<()> {
        let series: Vec<String> = points.iter().map(|p| name(p.0)).collect();
        let ts: Vec<f64> = points.iter().map(|p| secs(p.1)).collect();
        let v: Vec<f64> = points.iter().map(|p| p.2).collect();
        c.client.execute(&c.insert, &[&series, &ts, &v]).await?;
        Ok(())
    }

    async fn range(c: &mut Conn, series: u32, lo: i64, hi: i64) -> Res<(u64, f64)> {
        let row = c
            .client
            .query_one(&c.range, &[&name(series), &secs(lo), &secs(hi)])
            .await?;
        Ok((row.get::<_, i64>(0) as u64, row.get(1)))
    }

    async fn bucket(c: &mut Conn, series: u32, lo: i64, hi: i64) -> Res<(u64, f64)> {
        let rows = c
            .client
            .query(&c.bucket, &[&name(series), &secs(lo), &secs(hi)])
            .await?;
        Ok((
            rows.len() as u64,
            rows.iter().map(|r| r.get::<_, f64>(0)).sum(),
        ))
    }

    async fn last(c: &mut Conn, series: u32) -> Res<(i64, f64)> {
        let row = c.client.query_one(&c.last, &[&name(series)]).await?;
        Ok((row.get(0), row.get(1)))
    }

    async fn window_all(c: &mut Conn, lo: i64, hi: i64) -> Res<(u64, f64)> {
        let row = c
            .client
            .query_one(&c.window, &[&secs(lo), &secs(hi)])
            .await?;
        Ok((row.get::<_, i64>(0) as u64, row.get(1)))
    }

    fn queries() -> Vec<(&'static str, String)> {
        vec![
            ("range", Q_RANGE.into()),
            ("bucket", Q_BUCKET.into()),
            ("last", Q_LAST.into()),
            ("window_all", Q_WINDOW.into()),
            ("insert", Q_INSERT.into()),
        ]
    }
}
