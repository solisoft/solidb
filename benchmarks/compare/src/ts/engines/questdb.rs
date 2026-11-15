//! QuestDB as its documentation asks to be used: points written over the
//! InfluxDB line protocol on HTTP (port 9000), which answers once they are
//! committed to the WAL, and read over the PostgreSQL wire protocol (port
//! 8812) with prepared statements. A `SYMBOL` column for the series, a
//! designated timestamp, partitions by hour, `SAMPLE BY` and `LATEST ON`.

use super::{err, Engine, Res, Settings};
use crate::{name, Point};
use tokio_postgres::types::Type;
use tokio_postgres::{Client, NoTls, Statement};

const Q_RANGE: &str =
    "SELECT count(), sum(v) FROM points WHERE series = $1 AND ts >= $2 AND ts < $3";
const Q_BUCKET: &str = "SELECT avg(v) FROM points WHERE series = $1 AND ts >= $2 AND ts < $3 \
     SAMPLE BY 5m ALIGN TO CALENDAR";
const Q_LAST: &str = "SELECT cast(ts AS long), v FROM points WHERE series = $1 \
     LATEST ON ts PARTITION BY series";
const Q_WINDOW: &str = "SELECT count(), sum(v) FROM points WHERE ts >= $1 AND ts < $2";

pub struct QuestDb {
    s: Settings,
}

pub struct Conn {
    client: Client,
    http: reqwest::Client,
    write: String,
    range: Statement,
    bucket: Statement,
    last: Statement,
    window: Statement,
}

fn host(s: &Settings) -> &str {
    s.addr.split_once(':').map_or(s.addr.as_str(), |(h, _)| h)
}

async fn client(s: &Settings) -> Res<Client> {
    let port = s.addr.split_once(':').map_or("8812", |(_, p)| p);
    let config = format!(
        "host={} port={port} user=admin password=quest dbname=qdb",
        host(s)
    );
    let (client, connection) = tokio_postgres::connect(&config, NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("questdb connection: {e}");
        }
    });
    Ok(client)
}

/// QuestDB timestamps are microseconds.
fn micros(ms: i64) -> i64 {
    ms * 1000
}

impl Engine for QuestDb {
    type Conn = Conn;

    async fn open(s: &Settings) -> Res<Self> {
        Ok(QuestDb { s: s.clone() })
    }

    async fn version(&self) -> Res<String> {
        let c = client(&self.s).await?;
        let row = c.query_one("SELECT build()", &[]).await?;
        Ok(row.get(0))
    }

    async fn reset(&self) -> Res<()> {
        let c = client(&self.s).await?;
        c.batch_execute("DROP TABLE IF EXISTS points").await?;
        c.batch_execute(
            "CREATE TABLE points (series SYMBOL, ts TIMESTAMP, v DOUBLE) \
             TIMESTAMP(ts) PARTITION BY HOUR WAL",
        )
        .await?;
        Ok(())
    }

    async fn connect(&self) -> Res<Conn> {
        let client = client(&self.s).await?;
        let series_span = [Type::VARCHAR, Type::INT8, Type::INT8];
        Ok(Conn {
            range: client.prepare_typed(Q_RANGE, &series_span).await?,
            bucket: client.prepare_typed(Q_BUCKET, &series_span).await?,
            last: client.prepare_typed(Q_LAST, &[Type::VARCHAR]).await?,
            window: client
                .prepare_typed(Q_WINDOW, &[Type::INT8, Type::INT8])
                .await?,
            client,
            http: reqwest::Client::new(),
            write: format!("http://{}:9000/write?precision=ms", host(&self.s)),
        })
    }

    /// A WAL table is applied in the background: wait until every point
    /// that was acknowledged can be read.
    async fn after_load(&self, points: u64) -> Res<()> {
        let c = client(&self.s).await?;
        for _ in 0..2400 {
            let seen: i64 = c.query_one("SELECT count() FROM points", &[]).await?.get(0);
            if seen as u64 >= points {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        err("the WAL was not applied within ten minutes")
    }

    async fn insert(c: &mut Conn, points: Vec<Point>) -> Res<()> {
        use std::fmt::Write;
        let mut body = String::with_capacity(points.len() * 40);
        for (series, ts, v) in &points {
            // A float field needs its decimal point: `7` alone is refused.
            writeln!(body, "points,series={} v={:?} {}", name(*series), v, ts)?;
        }
        let answer = c.http.post(&c.write).body(body).send().await?;
        if !answer.status().is_success() {
            return err(format!(
                "write: {} {}",
                answer.status(),
                answer.text().await?
            ));
        }
        Ok(())
    }

    async fn range(c: &mut Conn, series: u32, lo: i64, hi: i64) -> Res<(u64, f64)> {
        let row = c
            .client
            .query_one(&c.range, &[&name(series), &micros(lo), &micros(hi)])
            .await?;
        Ok((
            row.get::<_, i64>(0) as u64,
            row.get::<_, Option<f64>>(1).unwrap_or(0.0),
        ))
    }

    async fn bucket(c: &mut Conn, series: u32, lo: i64, hi: i64) -> Res<(u64, f64)> {
        let rows = c
            .client
            .query(&c.bucket, &[&name(series), &micros(lo), &micros(hi)])
            .await?;
        Ok((
            rows.len() as u64,
            rows.iter().filter_map(|r| r.get::<_, Option<f64>>(0)).sum(),
        ))
    }

    async fn last(c: &mut Conn, series: u32) -> Res<(i64, f64)> {
        let row = c.client.query_one(&c.last, &[&name(series)]).await?;
        Ok((row.get::<_, i64>(0) / 1000, row.get(1)))
    }

    async fn window_all(c: &mut Conn, lo: i64, hi: i64) -> Res<(u64, f64)> {
        let row = c
            .client
            .query_one(&c.window, &[&micros(lo), &micros(hi)])
            .await?;
        Ok((
            row.get::<_, i64>(0) as u64,
            row.get::<_, Option<f64>>(1).unwrap_or(0.0),
        ))
    }

    fn queries() -> Vec<(&'static str, String)> {
        vec![
            ("range", Q_RANGE.into()),
            ("bucket", Q_BUCKET.into()),
            ("last", Q_LAST.into()),
            ("window_all", Q_WINDOW.into()),
            (
                "insert",
                "POST /write?precision=ms  points,series=<s> v=<v> <ms>".into(),
            ),
        ]
    }
}
